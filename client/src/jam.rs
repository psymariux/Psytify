//! Jam: ascolto condiviso con altre persone, fino a 8 per stanza. Il client parla con la stanza su
//! WebSocket da un thread proprio, così l'interfaccia non aspetta mai la rete: i comandi (lo stato
//! locale da inviare) e gli eventi (lo stato della stanza) passano per due canali, letti a ogni tick.
//! Il server non trasporta audio: ogni client risolve i propri URL firmati.

use std::thread;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use uuid::Uuid;

/// Brani nella coda condivisa: il server rifiuta una coda più lunga.
pub const MAX_QUEUE: usize = 500;
/// Persone per stanza: la schermata lo mostra, il server lo fa rispettare.
pub const MAX_MEMBERS: usize = 8;
/// Lunghezza del codice di una stanza.
pub const CODE_LEN: usize = 6;
/// Lunghezza massima del nome in stanza, come sul server.
pub const MAX_NAME_CHARS: usize = 24;
/// Scarto oltre il quale un ascoltatore salta alla posizione del gruppo. Sotto, è rete e orologio.
const DRIFT_MS: u64 = 1500;
/// Ogni quanto chi suona rimanda la propria posizione, per correggere lo scarto nel tempo.
const HEARTBEAT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const UNREACHABLE: &str = "Il server non risponde";
const LOST: &str = "Connessione persa";

/// Stato di riproduzione condiviso: la coda in ordine di ascolto, il brano corrente e la posizione.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub queue: Vec<Uuid>,
    pub index: usize,
    pub position_ms: u64,
    pub playing: bool,
}

/// Ultimo stato inviato e quando, per decidere se rimandarlo.
#[derive(Debug, Clone)]
pub struct Sent {
    pub snapshot: Snapshot,
    pub at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Member {
    pub id: Uuid,
    pub name: String,
}

/// Messaggi che il client manda alla stanza.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Outgoing<'a> {
    State(&'a Snapshot),
}

/// Uno stato della stanza, con chi l'ha mandato. Il tempo del server non serve al client: la
/// posizione si confronta con quella locale nel momento in cui arriva.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Remote {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub from: String,
}

/// Messaggi che la stanza manda al client. I campi sconosciuti si ignorano.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Welcome { you: Uuid, members: Vec<Member>, state: Option<Remote> },
    Members { members: Vec<Member> },
    State(Remote),
    Error { code: String },
}

/// Ciò che l'app deve sapere della stanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Entrati nella stanza: il codice da condividere, l'identità in stanza e l'ultimo stato.
    Joined {
        code: String,
        you: Uuid,
        members: Vec<Member>,
        state: Option<Remote>,
    },
    Members(Vec<Member>),
    State(Remote),
    /// Il server ha rifiutato un messaggio; la connessione resta aperta.
    Rejected(String),
    /// La connessione è chiusa. Il testo è già pronto per l'utente.
    Closed(String),
}

/// Cosa fare all'avvio: creare una stanza nuova o entrare in una esistente.
pub enum Target {
    Create,
    Join(String),
}

/// Collegamento con la stanza: si invia lo stato locale e si legge quello del gruppo. Quando il
/// `Link` viene chiuso, il thread chiude il socket e il server libera il posto.
pub struct Link {
    commands: mpsc::UnboundedSender<Snapshot>,
    pub events: mpsc::UnboundedReceiver<Event>,
}

impl Link {
    pub fn start(api_base: &str, target: Target, name: String) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        let base = api_base.trim_end_matches('/').to_owned();
        thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                let _ = event_tx.send(Event::Closed("Impossibile avviare la jam".into()));
                return;
            };
            let reason = runtime.block_on(run(&base, target, &name, command_rx, &event_tx));
            let _ = event_tx.send(Event::Closed(reason));
        });
        Self { commands, events }
    }

    /// Invia lo stato locale alla stanza. Se il thread è già terminato, il comando si perde: la
    /// chiusura arriva comunque come `Event::Closed`.
    pub fn send(&self, snapshot: Snapshot) {
        let _ = self.commands.send(snapshot);
    }
}

/// Vita della connessione. Restituisce il motivo della chiusura.
async fn run(
    base: &str,
    target: Target,
    name: &str,
    mut commands: mpsc::UnboundedReceiver<Snapshot>,
    events: &mpsc::UnboundedSender<Event>,
) -> String {
    let code = match target {
        Target::Create => match create_room(base).await {
            Ok(code) => code,
            Err(reason) => return reason,
        },
        Target::Join(code) => code,
    };
    let url = match socket_url(base, &code, name) {
        Ok(url) => url,
        Err(reason) => return reason,
    };
    let mut socket = match timeout(CONNECT_TIMEOUT, connect_async(url.as_str())).await {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(err)) => return refused(&err),
        Err(_) => return UNREACHABLE.into(),
    };
    loop {
        tokio::select! {
            outgoing = commands.recv() => {
                // Senza comandi l'app ha chiuso il collegamento: si esce e il socket cade con noi.
                let Some(snapshot) = outgoing else { return "Jam chiusa".into() };
                let Ok(text) = serde_json::to_string(&Outgoing::State(&snapshot)) else { continue };
                if socket.send(Message::Text(text)).await.is_err() {
                    return LOST.into();
                }
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => forward(&text, &code, events),
                Some(Ok(Message::Close(_))) | None => return "Jam chiusa dal server".into(),
                Some(Err(_)) => return LOST.into(),
                Some(Ok(_)) => {}
            },
        }
    }
}

#[derive(Deserialize)]
struct Created {
    code: String,
}

async fn create_room(base: &str) -> Result<String, String> {
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/jam"))
        .send()
        .await
        .map_err(|_| UNREACHABLE.to_owned())?;
    if !response.status().is_success() {
        return Err("Impossibile creare la jam".into());
    }
    let created: Created = response.json().await.map_err(|_| "Risposta del server non valida".to_owned())?;
    Ok(created.code)
}

/// `http(s)://host/v1/jam/{code}/ws?name=...`, con lo schema WebSocket corrispondente.
fn socket_url(base: &str, code: &str, name: &str) -> Result<Url, String> {
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Il codice non è valido".into());
    }
    let mut url =
        Url::parse(&format!("{base}/v1/jam/{code}/ws")).map_err(|_| "Indirizzo non valido".to_owned())?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).map_err(|()| "Indirizzo non valido".to_owned())?;
    url.query_pairs_mut().append_pair("name", name);
    Ok(url)
}

/// Il motivo del rifiuto alla connessione, in parole per l'utente.
fn refused(err: &WsError) -> String {
    match err {
        WsError::Http(response) => match response.status().as_u16() {
            404 => "Nessuna jam con questo codice".into(),
            409 => "La jam è piena: ci sono già 8 persone".into(),
            _ => "Il server ha rifiutato la jam".into(),
        },
        _ => UNREACHABLE.into(),
    }
}

fn forward(text: &str, code: &str, events: &mpsc::UnboundedSender<Event>) {
    // Un messaggio che non si capisce si ignora: la stanza può aggiungere campi nuovi.
    let Ok(message) = serde_json::from_str::<Incoming>(text) else { return };
    let event = match message {
        Incoming::Welcome { you, members, state } => {
            Event::Joined { code: code.to_owned(), you, members, state }
        }
        Incoming::Members { members } => Event::Members(members),
        Incoming::State(remote) => Event::State(remote),
        Incoming::Error { code: reason } => Event::Rejected(rejection(&reason)),
    };
    let _ = events.send(event);
}

fn rejection(code: &str) -> String {
    match code {
        "invalid_state" => "Stato della jam non valido".into(),
        "rate_limited" => "Troppi cambiamenti in poco tempo: riprova tra un attimo".into(),
        _ => "Messaggio rifiutato dalla jam".into(),
    }
}

/// Decide se mandare lo stato adesso. Sì se è cambiato il contenuto o la riproduzione, se la
/// posizione non è quella prevista (un salto), o se chi suona non ha rimandato la posizione da un
/// po'. Durante la riproduzione la posizione prevista avanza con il tempo trascorso.
pub fn should_send(last: Option<&Sent>, now: &Snapshot, at: Instant) -> bool {
    let Some(last) = last else { return true };
    let sent = &last.snapshot;
    if sent.queue != now.queue || sent.index != now.index || sent.playing != now.playing {
        return true;
    }
    let elapsed = at.saturating_duration_since(last.at);
    if sent.playing {
        let expected = sent.position_ms.saturating_add(millis(elapsed));
        now.position_ms.abs_diff(expected) > DRIFT_MS || elapsed >= HEARTBEAT
    } else {
        now.position_ms.abs_diff(sent.position_ms) > DRIFT_MS
    }
}

/// La posizione del gruppo è lontana da quella locale: conviene saltare.
pub fn drifted(local_ms: u64, remote_ms: u64) -> bool {
    local_ms.abs_diff(remote_ms) > DRIFT_MS
}

pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(queue: &[Uuid], index: usize, position_ms: u64, playing: bool) -> Snapshot {
        Snapshot { queue: queue.to_vec(), index, position_ms, playing }
    }

    #[test]
    fn first_state_is_always_sent() {
        assert!(should_send(None, &snapshot(&[], 0, 0, false), Instant::now()));
    }

    #[test]
    fn a_change_of_track_or_queue_is_sent_at_once() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a, b], 0, 0, true), at };
        assert!(should_send(Some(&last), &snapshot(&[a, b], 1, 0, true), at));
        assert!(should_send(Some(&last), &snapshot(&[b, a], 0, 0, true), at));
    }

    #[test]
    fn pause_and_resume_are_sent_at_once() {
        let a = Uuid::new_v4();
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a], 0, 1000, true), at };
        assert!(should_send(Some(&last), &snapshot(&[a], 0, 1000, false), at));
    }

    #[test]
    fn steady_playback_follows_the_clock_without_sending() {
        let a = Uuid::new_v4();
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a], 0, 1000, true), at };
        let later = at + Duration::from_secs(2);
        assert!(!should_send(Some(&last), &snapshot(&[a], 0, 3050, true), later));
    }

    #[test]
    fn a_seek_is_sent_even_mid_track() {
        let a = Uuid::new_v4();
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a], 0, 1000, true), at };
        assert!(should_send(Some(&last), &snapshot(&[a], 0, 90_000, true), at + Duration::from_millis(250)));
    }

    #[test]
    fn a_seek_while_paused_is_sent() {
        let a = Uuid::new_v4();
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a], 0, 1000, false), at };
        assert!(should_send(Some(&last), &snapshot(&[a], 0, 20_000, false), at));
        assert!(!should_send(Some(&last), &snapshot(&[a], 0, 1200, false), at));
    }

    #[test]
    fn playing_sends_a_heartbeat_every_five_seconds() {
        let a = Uuid::new_v4();
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a], 0, 0, true), at };
        let later = at + HEARTBEAT;
        assert!(should_send(Some(&last), &snapshot(&[a], 0, millis(HEARTBEAT), true), later));
    }

    #[test]
    fn small_drift_is_tolerated_by_listeners() {
        assert!(!drifted(10_000, 11_000));
        assert!(drifted(10_000, 20_000));
    }

    #[test]
    fn socket_url_uses_ws_schemes_and_encodes_the_name() {
        let plain = socket_url("http://127.0.0.1:8080", "K7QXM3", "Luca & Sara").expect("url");
        assert_eq!(plain.as_str(), "ws://127.0.0.1:8080/v1/jam/K7QXM3/ws?name=Luca+%26+Sara");
        let tls = socket_url("https://api.example.org", "K7QXM3", "x").expect("url");
        assert_eq!(tls.scheme(), "wss");
    }

    #[test]
    fn a_bad_code_never_reaches_the_url() {
        assert!(socket_url("http://h", "../etc", "x").is_err());
        assert!(socket_url("http://h", "", "x").is_err());
    }

    #[test]
    fn incoming_state_parses_with_the_flattened_snapshot() {
        let a = Uuid::new_v4();
        let text = format!(
            r#"{{"type":"state","queue":["{a}"],"index":0,"position_ms":1500,"playing":true,"from":"Sara","at_ms":1}}"#
        );
        let Ok(Incoming::State(remote)) = serde_json::from_str::<Incoming>(&text) else {
            panic!("state must parse");
        };
        assert_eq!(remote.snapshot, Snapshot { queue: vec![a], index: 0, position_ms: 1500, playing: true });
        assert_eq!(remote.from, "Sara");
    }

    #[test]
    fn welcome_carries_the_last_state_or_none() {
        let you = Uuid::new_v4();
        let empty =
            format!(r#"{{"type":"welcome","you":"{you}","code":"K7QXM3","members":[],"state":null}}"#);
        let Ok(Incoming::Welcome { state, .. }) = serde_json::from_str::<Incoming>(&empty) else {
            panic!("welcome must parse");
        };
        assert!(state.is_none());

        let with_state = format!(
            r#"{{"type":"welcome","you":"{you}","code":"K7QXM3","members":[],"state":{{"queue":[],"index":0,"position_ms":0,"playing":false,"from":"Luca","at_ms":1}}}}"#
        );
        let Ok(Incoming::Welcome { state, .. }) = serde_json::from_str::<Incoming>(&with_state) else {
            panic!("welcome must parse");
        };
        assert_eq!(state.map(|remote| remote.from), Some("Luca".to_owned()));
    }

    #[test]
    fn outgoing_state_has_the_protocol_shape() {
        let a = Uuid::new_v4();
        let text = serde_json::to_string(&Outgoing::State(&snapshot(&[a], 0, 42, false))).expect("json");
        assert!(text.starts_with(r#"{"type":"state","queue":["#));
        assert!(text.contains(r#""position_ms":42"#));
    }

    async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
        timeout(Duration::from_secs(5), events.recv()).await.expect("event in time").expect("link open")
    }

    /// Prova dal vivo contro un backend acceso: stanza, ospite, stato condiviso, ottavo posto e nono
    /// rifiutato. Si lancia con `PSYTIFY_LIVE_API=http://127.0.0.1:8080 cargo test jam_live -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a running backend: set PSYTIFY_LIVE_API"]
    async fn a_room_is_shared_live() {
        let base = std::env::var("PSYTIFY_LIVE_API").expect("PSYTIFY_LIVE_API");
        let mut host = Link::start(&base, Target::Create, "host".into());
        let Event::Joined { code, state, .. } = next(&mut host.events).await else { panic!("host joins") };
        assert!(state.is_none(), "a new room has no state");

        let mut guest = Link::start(&base, Target::Join(code.clone()), "guest".into());
        assert!(matches!(next(&mut guest.events).await, Event::Joined { .. }));
        loop {
            if let Event::Members(list) = next(&mut host.events).await
                && list.len() == 2
            {
                break;
            }
        }

        let track = Uuid::new_v4();
        host.send(Snapshot { queue: vec![track], index: 0, position_ms: 4200, playing: true });
        let Event::State(remote) = next(&mut guest.events).await else { panic!("guest gets the state") };
        assert_eq!(remote.from, "host");
        assert_eq!(remote.snapshot.position_ms, 4200);

        let mut seats = vec![host, guest];
        for i in 0..6 {
            let mut seat = Link::start(&base, Target::Join(code.clone()), format!("p{i}"));
            assert!(matches!(next(&mut seat.events).await, Event::Joined { .. }));
            seats.push(seat);
        }
        let mut ninth = Link::start(&base, Target::Join(code), "nona".into());
        let Event::Closed(reason) = next(&mut ninth.events).await else { panic!("ninth is refused") };
        assert!(reason.contains("piena"), "{reason}");
    }
}
