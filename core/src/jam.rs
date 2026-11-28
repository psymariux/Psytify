//! Jam: listening together with other people, up to 8 per room. The link talks to the room
//! over WebSocket from a thread of its own, so neither the interface nor the core waits for the network: commands
//! (the local state to send) and events (the room's state) pass through two channels. The core's host
//! holds it (step 4.8), and waits for the events together with everything else: an idle jam
//! wakes nobody and needs no tick. The logic (roles, alignment, when to send the
//! state) is in `state/jam.rs`. The server carries no audio: each client resolves its own signed URLs.

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

/// Tracks in the shared queue: the server rejects a longer queue.
pub const MAX_QUEUE: usize = 500;
/// People per room: the screen shows it, the server enforces it.
pub const MAX_MEMBERS: usize = 8;
/// Length of a room's code.
pub const CODE_LEN: usize = 6;
/// Maximum length of the in-room name, as on the server.
pub const MAX_NAME_CHARS: usize = 24;
/// Drift beyond which a listener jumps to the group's position. Below it, it is network and clock.
const DRIFT_MS: u64 = 1500;
/// How often the one playing sends back their position, to correct the drift over time.
const HEARTBEAT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const UNREACHABLE: &str = "Il server non risponde";
pub(crate) const LOST: &str = "Connessione persa";

/// Shared playback state: the queue in listening order, the current track and the position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub queue: Vec<Uuid>,
    pub index: usize,
    pub position_ms: u64,
    pub playing: bool,
}

/// Last state sent and when, to decide whether to send it again.
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

/// Messages the client sends to the room.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Outgoing<'a> {
    State(&'a Snapshot),
}

/// A room state, with who sent it. The server's time is not needed by the client: the
/// position is compared with the local one at the moment it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Remote {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub from: String,
}

/// Messages the room sends to the client. Unknown fields are ignored.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Welcome { you: Uuid, members: Vec<Member>, state: Option<Remote> },
    Members { members: Vec<Member> },
    State(Remote),
    Error { code: String },
}

/// What the app must know about the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Joined the room: the code to share, the identity in the room and the last state.
    Joined {
        code: String,
        you: Uuid,
        members: Vec<Member>,
        state: Option<Remote>,
    },
    Members(Vec<Member>),
    State(Remote),
    /// The server rejected a message; the connection stays open.
    Rejected(String),
    /// The connection is closed. The text is already ready for the user.
    Closed(String),
}

/// What to do at startup: create a new room or join an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Create,
    Join(String),
}

/// Link to the room: the local state is sent and the group's is read. When the
/// `Link` is closed, the thread closes the socket and the server frees the place.
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

    /// Sends the local state to the room. If the thread has already ended, the command is lost; the
    /// closing still arrives as `Event::Closed`.
    pub fn send(&self, snapshot: Snapshot) {
        let _ = self.commands.send(snapshot);
    }
}

/// Life of the connection. Returns the reason for closing.
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
                // With no commands the app has closed the link: we exit and the socket goes down with us.
                let Some(snapshot) = outgoing else { return "Jam chiusa".into() };
                let Ok(text) = serde_json::to_string(&Outgoing::State(&snapshot)) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
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

/// `http(s)://host/v1/jam/{code}/ws?name=...`, with the matching WebSocket scheme.
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

/// The reason for refusing the connection, in words for the user.
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
    // A message that is not understood is ignored: the room may add new fields.
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

/// Decides whether to send the state now. Yes if the content or the playback has changed, if the
/// position is not the expected one (a jump), or if the one playing has not sent the position back for a
/// while. While playing, the expected position advances with the time elapsed.
pub fn should_send(last: Option<&Sent>, now: &Snapshot, at: Instant) -> bool {
    should_send_state(last, now.queue.iter().copied(), now.index, now.position_ms, now.playing, at)
}

/// As [`should_send`], but with the local state still loose: the queue as a stream of ids, so the
/// comparison does not build a `Vec` at each entry of the core (a jam listening sees four of them a
/// second) and the state is built only when it really has to be sent.
pub fn should_send_state(
    last: Option<&Sent>,
    queue: impl Iterator<Item = Uuid>,
    index: usize,
    position_ms: u64,
    playing: bool,
    at: Instant,
) -> bool {
    let Some(last) = last else { return true };
    let sent = &last.snapshot;
    if sent.index != index || sent.playing != playing || !queue.eq(sent.queue.iter().copied()) {
        return true;
    }
    let elapsed = at.saturating_duration_since(last.at);
    if sent.playing {
        let expected = sent.position_ms.saturating_add(millis(elapsed));
        position_ms.abs_diff(expected) > DRIFT_MS || elapsed >= HEARTBEAT
    } else {
        position_ms.abs_diff(sent.position_ms) > DRIFT_MS
    }
}

/// A room's code: [`CODE_LEN`] letters or digits. The server rejects everything else, and a
/// wrong code must not even reach the socket's address.
pub fn valid_code(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The group's position is far from the local one: it is worth jumping.
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
    fn a_queue_as_a_stream_of_ids_decides_like_the_snapshot() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let at = Instant::now();
        let last = Sent { snapshot: snapshot(&[a, b], 0, 1000, true), at };
        let later = at + Duration::from_secs(2);
        let same = |queue: &[Uuid], index, position| {
            should_send_state(Some(&last), queue.iter().copied(), index, position, true, later)
        };
        assert!(!same(&[a, b], 0, 3050), "stesso contenuto, posizione prevista");
        assert!(same(&[a], 0, 3050), "una coda più corta");
        assert!(same(&[a, b, a], 0, 3050), "una coda più lunga");
        assert!(same(&[b, a], 0, 3050), "un altro ordine");
        assert!(same(&[a, b], 1, 3050), "un altro brano");
        assert!(same(&[a, b], 0, 90_000), "un salto");
        assert!(should_send_state(None, [a].into_iter(), 0, 0, false, at), "il primo stato");
    }

    #[test]
    fn only_six_letters_or_digits_are_a_room_code() {
        assert!(valid_code("K7QXM3") && valid_code("abc123"));
        for code in ["", "K7QXM", "K7QXM3X", "K7-XM3", "K7 XM3", "../etc/", "K7QXM·"] {
            assert!(!valid_code(code), "{code:?}");
        }
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

    /// Live test against a running backend: room, guest, shared state, eighth place and ninth
    /// refused. Launched with `PSYTIFY_LIVE_API=http://127.0.0.1:8080 cargo test jam_live -- --ignored`.
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
