//! Jam: l'ascolto insieme agli amici (passo 4.8).
//!
//! **Il collegamento.** La stanza è un WebSocket che tiene l'host del core (`jam::Link`, su un thread
//! suo): il riduttore non apre la rete, chiede all'host di avviarlo, di mandargli lo stato e di
//! chiuderlo (`Effect::Jam`, [`JamOp`]), e gli eventi della stanza entrano nel core come
//! `Input::Jam`, aspettati dall'host insieme a tutto il resto. Non c'è nessun timer: una jam ferma
//! non sveglia il core, e il battito da 250 ms della riproduzione (che gira solo mentre suona) è la
//! sola cadenza che serve a rimandare la posizione ogni cinque secondi.
//!
//! **Chi suona e chi ascolta.** Non ci sono ruoli fissi: ogni membro manda alla stanza il proprio
//! stato quando cambia davvero (un altro brano, la coda, pausa e ripresa, un salto di posizione, il
//! battito ogni cinque secondi mentre suona: `jam::should_send_state`) e allinea il proprio al
//! primo stato che arriva dagli altri. Lo stato ricevuto diventa il riferimento: finché la
//! riproduzione locale lo segue non si rimanda indietro.
//!
//! **Il passaggio fra i domini.** Dopo ogni ingresso [`CoreState::handoff`](super::CoreState) chiama
//! [`Jam::settle`] (gli stati arrivati dalla stanza, con la coda locale sotto gli occhi: i brani che
//! mancano si chiedono al catalogo, poi `PlaybackCmd::Align`), applica a riproduzione ciò che il
//! dominio chiede ([`Jam::take_playback`]: il gruppo, l'allineamento, la coda svuotata) e solo dopo
//! chiama [`Jam::publish`], così lo stato mandato alla stanza è quello già allineato. Al dominio
//! sociale il codice della stanza arriva dalla vista ([`Social::set_jam`](super::social::Social)), e
//! il nome da mostrare in stanza viene dal profilo ([`Jam::set_profile`]): il frontend non fa da
//! tramite.
//!
//! Cambiare vista solo se qualcosa cambia davvero: gli stessi membri, un evento di una stanza che
//! non c'è più o uno stato che non cambia nulla non pubblicano niente.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, TryStreamExt};
use uuid::Uuid;

use crate::api::{ApiClient, Track};
use crate::config;
use crate::effect::{self, Effect};
use crate::error::AppError;
use crate::jam::{self, Member, Remote, Sent, Target};
use crate::player::PlaybackState;
use crate::state::playback::{Align, PlaybackCmd, PlaybackView};

/// Eventi tenuti nella vista: un frontend che legge a ogni pubblicazione non ne perde.
const EVENTS: usize = 32;
/// Brani del catalogo chiesti insieme, per la coda condivisa che manca.
const FETCH_PARALLEL: usize = 8;

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum JamCmd {
    /// Crea una stanza nuova. `name` è il nome da mostrare in stanza: vuoto, si usa quello del
    /// profilo (se c'è un accesso); al più [`jam::MAX_NAME_CHARS`] caratteri.
    Create { name: String },
    /// Entra in una stanza con il suo codice ([`jam::CODE_LEN`] lettere o cifre).
    Join { code: String, name: String },
    /// Esce dalla jam: la stanza libera il posto e avvisa gli altri.
    Leave,
}

/// Dove si è con la jam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JamPhase {
    /// Nessuna jam.
    #[default]
    Idle,
    /// Il collegamento è partito e la stanza non ha ancora risposto.
    Connecting,
    /// Si è in stanza: ha detto il suo codice e chi c'è.
    Open,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JamView {
    pub phase: JamPhase,
    /// Il codice da condividere, appena la stanza lo dice.
    pub code: Option<String>,
    /// L'identità di questo client in stanza.
    pub you: Option<Uuid>,
    pub members: Arc<[Member]>,
    /// Gli ultimi eventi, dal più vecchio: un frontend mostra quelli con `seq` nuovo, in ordine.
    pub events: Arc<VecDeque<JamRaised>>,
}

impl JamView {
    /// Una jam c'è: in connessione o aperta.
    pub fn is_active(&self) -> bool {
        self.phase != JamPhase::Idle
    }
}

/// Qualcosa da dire all'utente della jam, con i testi di sempre.
#[derive(Debug, Clone, PartialEq)]
pub enum JamEvent {
    /// Si è entrati nella stanza con questo codice.
    Joined(String),
    /// Si è usciti.
    Left,
    /// Una jam c'è già: se ne esce prima.
    AlreadyIn,
    /// Il codice non ha [`jam::CODE_LEN`] lettere o cifre.
    BadCode,
    /// Un altro ha cambiato brano (non scatta per lo stato che arriva con l'ingresso).
    TrackChanged { from: String },
    /// I brani della coda condivisa non si sono caricati.
    TracksFailed(AppError),
    /// Il catalogo non ha alcuni brani della coda condivisa.
    TracksMissing,
    /// La stanza ha rifiutato un messaggio; il collegamento resta aperto.
    Rejected(String),
    /// Il collegamento si è chiuso (il testo è già pronto).
    Closed(String),
}

impl JamEvent {
    /// Il testo da mostrare.
    pub fn message(&self) -> String {
        match self {
            Self::Joined(code) => format!("Sei nella jam {code}"),
            Self::Left => "Sei uscito dalla jam".into(),
            Self::AlreadyIn => "Sei già in una jam".into(),
            Self::BadCode => format!("Il codice ha {} lettere o cifre", jam::CODE_LEN),
            Self::TrackChanged { from } => format!("{from} ha cambiato brano"),
            Self::TracksFailed(err) => format!("Brani della jam non caricati: {err}"),
            Self::TracksMissing => "Alcuni brani della jam non sono disponibili".into(),
            Self::Rejected(reason) | Self::Closed(reason) => reason.clone(),
        }
    }

    /// L'avviso è un errore, non un'informazione.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::BadCode | Self::TracksFailed(_) | Self::TracksMissing | Self::Rejected(_) | Self::Closed(_)
        )
    }
}

/// Un [`JamEvent`] con il suo numero d'ordine, che cresce a ogni evento.
#[derive(Debug, Clone, PartialEq)]
pub struct JamRaised {
    pub seq: u64,
    pub event: JamEvent,
}

/// Ciò che l'host fa con il collegamento alla stanza, che tiene lui sul proprio thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JamOp {
    /// Avvia il collegamento: crea la stanza o entra in quella del codice. Un collegamento che c'era
    /// si chiude.
    Start { base: String, target: Target, name: String },
    /// Manda alla stanza lo stato locale.
    Send(jam::Snapshot),
    /// Chiude il collegamento (se c'è): la stanza libera il posto.
    Leave,
}

/// I lavori asincroni del dominio (rete), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// I brani del catalogo che la coda condivisa ha e quella locale no, per la richiesta `ticket`.
    Tracks { api: Result<ApiClient, AppError>, ids: Vec<Uuid>, ticket: u64 },
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    Tracks { ticket: u64, result: Result<Vec<Track>, AppError> },
}

/// Qualcosa arrivato dalla stanza, che aspetta il passaggio fra i domini per leggere la coda locale.
#[derive(Debug)]
enum Arrived {
    /// Uno stato del gruppo. `first` è quello che la stanza manda all'ingresso: non è «un altro ha
    /// cambiato brano».
    State { remote: Remote, first: bool },
    /// I brani chiesti per `remote` sono arrivati (o no).
    Tracks { remote: Remote, result: Result<Vec<Track>, AppError> },
}

/// I brani che si aspettano dal catalogo, e lo stato del gruppo a cui servono.
#[derive(Debug)]
struct Fetching {
    ticket: u64,
    remote: Remote,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Jam {
    pub(crate) view: Arc<JamView>,
    /// Il backend scelto nelle impostazioni e il client che ne viene (i brani della coda).
    server: Option<String>,
    api: Result<ApiClient, AppError>,
    /// Il nome del profilo, se c'è un accesso: il nome in stanza di chi non ne scrive uno.
    profile: Option<String>,
    /// La stanza ha detto il suo stato, o non ne ha: solo allora si manda il proprio.
    synced: bool,
    /// Ultimo stato scambiato con la stanza: il riferimento per decidere se rimandare.
    last_sent: Option<Sent>,
    /// Stati e brani arrivati, in ordine, da trattare al prossimo passaggio.
    arrived: VecDeque<Arrived>,
    /// Brani della coda condivisa in arrivo: nel frattempo non si manda niente.
    fetching: Option<Fetching>,
    tickets: u64,
    /// Ciò che si chiede alla riproduzione al prossimo passaggio.
    to_playback: Vec<PlaybackCmd>,
    /// Il core si chiude: niente più stati alla stanza (la riproduzione si ferma, e non deve
    /// fermare anche gli altri).
    closing: bool,
    /// Il numero dell'ultimo evento.
    events: u64,
}

impl Default for Jam {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Jam {
    /// Il dominio con il backend scelto nelle impostazioni.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            profile: None,
            synced: false,
            last_sent: None,
            arrived: VecDeque::new(),
            fetching: None,
            tickets: 0,
            to_playback: Vec::new(),
            closing: false,
            events: 0,
        }
    }

    fn view_mut(&mut self) -> &mut JamView {
        Arc::make_mut(&mut self.view)
    }

    /// Il codice della stanza in cui si è, se la stanza l'ha detto: lo legge il dominio sociale.
    pub(crate) fn code(&self) -> Option<&str> {
        self.view.code.as_deref()
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client dei brani si rifà. Una jam in
    /// corso resta sul server con cui è partita.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    /// Il nome del profilo (se c'è un accesso): si copia solo quando cambia.
    pub(crate) fn set_profile(&mut self, name: Option<&str>) {
        if self.profile.as_deref() != name {
            self.profile = name.map(str::to_owned);
        }
    }

    /// Il core si chiude: da qui la jam non manda più niente.
    pub(crate) fn close(&mut self) {
        self.closing = true;
    }

    fn raise(&mut self, event: JamEvent) {
        self.events += 1;
        let raised = JamRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Comandi ----------

    pub(crate) fn apply(&mut self, command: JamCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            JamCmd::Create { name } => self.start(Target::Create, &name, effects),
            JamCmd::Join { code, name } => self.start(Target::Join(code), &name, effects),
            JamCmd::Leave => {
                if self.view.is_active() {
                    // Chiudere il collegamento basta: la stanza libera il posto e avvisa gli altri.
                    self.reset();
                    effects.push(Effect::Jam(JamOp::Leave));
                    self.to_playback.push(PlaybackCmd::Group(false));
                    self.raise(JamEvent::Left);
                }
            }
        }
    }

    /// Avvia una jam: crea una stanza o entra in una esistente. Il collegamento gira su un thread
    /// proprio, quindi la richiesta torna subito; la stanza risponde con un evento.
    fn start(&mut self, target: Target, name: &str, effects: &mut Vec<Effect>) {
        if self.view.is_active() {
            self.raise(JamEvent::AlreadyIn);
            return;
        }
        if let Target::Join(code) = &target
            && !jam::valid_code(code)
        {
            self.raise(JamEvent::BadCode);
            return;
        }
        // Senza un nome scelto per la stanza, con un profilo ci si presenta con il proprio.
        let typed = name.trim();
        let name = match &self.profile {
            Some(profile) if typed.is_empty() => profile.as_str(),
            _ => typed,
        };
        let name: String = name.chars().take(jam::MAX_NAME_CHARS).collect();
        self.reset();
        self.view_mut().phase = JamPhase::Connecting;
        // La coda è del gruppo: la riproduzione automatica non aggiunge nulla da sola.
        self.to_playback.push(PlaybackCmd::Group(true));
        effects.push(Effect::Jam(JamOp::Start {
            base: config::api_url(self.server.as_deref()),
            target,
            name,
        }));
    }

    /// Dimentica la stanza: la vista torna a «nessuna jam» (gli eventi restano) e ogni attesa cade.
    fn reset(&mut self) {
        if self.view.is_active() || self.view.code.is_some() || !self.view.members.is_empty() {
            let view = self.view_mut();
            view.phase = JamPhase::Idle;
            view.code = None;
            view.you = None;
            view.members = Arc::default();
        }
        self.synced = false;
        self.last_sent = None;
        self.arrived.clear();
        self.fetching = None;
    }

    // ---------- La stanza ----------

    /// Un evento del collegamento. Uno che arriva a jam finita (il collegamento chiuso da noi) si
    /// butta.
    pub(crate) fn event(&mut self, event: jam::Event) {
        if !self.view.is_active() {
            return;
        }
        match event {
            jam::Event::Joined { code, you, members, state } => {
                let view = self.view_mut();
                view.phase = JamPhase::Open;
                view.code = Some(code.clone());
                view.you = Some(you);
                view.members = Arc::from(members);
                self.raise(JamEvent::Joined(code));
                match state {
                    Some(remote) => self.arrived.push_back(Arrived::State { remote, first: true }),
                    // Stanza nuova: il primo stato è il proprio.
                    None => self.synced = true,
                }
            }
            jam::Event::Members(members) => {
                if *self.view.members != *members {
                    self.view_mut().members = Arc::from(members);
                }
            }
            jam::Event::State(remote) => self.arrived.push_back(Arrived::State { remote, first: false }),
            jam::Event::Rejected(reason) => self.raise(JamEvent::Rejected(reason)),
            jam::Event::Closed(reason) => {
                self.reset();
                self.to_playback.push(PlaybackCmd::Group(false));
                self.raise(JamEvent::Closed(reason));
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, _effects: &mut Vec<Effect>) {
        match done {
            Done::Tracks { ticket, result } => {
                // Una risposta superata (un altro stato è arrivato, o si è usciti) si butta.
                let Some(fetching) = self.fetching.take_if(|fetching| fetching.ticket == ticket) else {
                    return;
                };
                self.arrived.push_back(Arrived::Tracks { remote: fetching.remote, result });
            }
        }
    }

    /// Il numero dell'ultima richiesta di brani, per i test che consegnano un esito a mano.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn fetch_ticket(&self) -> Option<u64> {
        self.fetching.as_ref().map(|fetching| fetching.ticket)
    }

    // ---------- Il passaggio fra i domini ----------

    /// Tratta ciò che è arrivato dalla stanza, leggendo la coda locale: i brani che mancano si
    /// chiedono al catalogo, poi la riproduzione si allinea al gruppo (`take_playback`).
    pub(crate) fn settle(&mut self, playback: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        while let Some(arrived) = self.arrived.pop_front() {
            if self.closing || !self.view.is_active() {
                self.arrived.clear();
                return;
            }
            match arrived {
                Arrived::State { remote, first } => self.remote_state(remote, first, playback, now, effects),
                Arrived::Tracks { remote, result } => match result {
                    Ok(fetched) => self.adopt(&remote, &fetched, playback),
                    Err(err) => self.raise(JamEvent::TracksFailed(err)),
                },
            }
        }
    }

    /// Lo stato del gruppo diventa quello locale: si chiedono i brani che mancano, poi si allinea.
    fn remote_state(
        &mut self,
        remote: Remote,
        first: bool,
        playback: &PlaybackView,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        self.synced = true;
        // Lo stato ricevuto è il riferimento: non va rispedito come se fosse locale.
        self.last_sent = Some(Sent { snapshot: remote.snapshot.clone(), at: now });
        // Uno stato nuovo supera i brani che si aspettavano per il vecchio.
        self.fetching = None;
        let snapshot = &remote.snapshot;
        if snapshot.queue.is_empty() {
            self.to_playback.push(PlaybackCmd::Clear);
            return;
        }
        let wanted = snapshot.queue.get(snapshot.index).copied();
        if !first && playback.queue.current().map(|track| track.id) != wanted {
            self.raise(JamEvent::TrackChanged { from: remote.from.clone() });
        }
        let known: HashSet<Uuid> = playback.queue.entries().map(|(_, track)| track.id).collect();
        let mut asked = HashSet::new();
        let missing: Vec<Uuid> =
            snapshot.queue.iter().copied().filter(|id| !known.contains(id) && asked.insert(*id)).collect();
        if missing.is_empty() {
            self.adopt(&remote, &[], playback);
            return;
        }
        self.tickets += 1;
        let ticket = self.tickets;
        self.fetching = Some(Fetching { ticket, remote });
        effects.push(Effect::Run(effect::Job::Jam(Job::Tracks {
            api: self.api.clone(),
            ids: missing,
            ticket,
        })));
    }

    /// Allinea la riproduzione al gruppo: la coda nell'ordine del gruppo (con i brani già in coda e
    /// quelli arrivati dal catalogo; lo shuffle si spegne, l'ordine lo decide chi suona per primo),
    /// poi lo stesso brano alla stessa posizione, oppure si cambia brano. Il resto, compreso il
    /// salto alla posizione condivisa quando l'audio del brano è pronto, lo fa la riproduzione.
    fn adopt(&mut self, remote: &Remote, fetched: &[Track], playback: &PlaybackView) {
        let snapshot = &remote.snapshot;
        let same = playback.queue.entries().map(|(_, track)| track.id).eq(snapshot.queue.iter().copied());
        let tracks = if same {
            None
        } else {
            // I brani già in coda vincono su quelli arrivati: è la stessa copia che il gruppo ha.
            let mut by_id: HashMap<Uuid, &Track> = HashMap::new();
            for track in playback.queue.entries().map(|(_, track)| track).chain(fetched) {
                by_id.entry(track.id).or_insert(track);
            }
            let ordered: Option<Vec<Track>> =
                snapshot.queue.iter().map(|id| by_id.get(id).map(|track| (*track).clone())).collect();
            let Some(ordered) = ordered else {
                self.raise(JamEvent::TracksMissing);
                return;
            };
            Some(ordered)
        };
        self.to_playback.push(PlaybackCmd::Align(Box::new(Align {
            tracks,
            index: snapshot.index,
            position: Duration::from_millis(snapshot.position_ms),
            playing: snapshot.playing,
        })));
    }

    /// Ciò che il dominio chiede alla riproduzione (il gruppo, l'allineamento, la coda svuotata): lo
    /// applica il passaggio fra i domini, nello stesso giro.
    pub(crate) fn take_playback(&mut self) -> Vec<PlaybackCmd> {
        std::mem::take(&mut self.to_playback)
    }

    /// Manda alla stanza lo stato locale, se è cambiato o se è tempo di rimandare la posizione. Non
    /// si manda nulla mentre il gruppo non ha ancora detto il suo stato, né mentre un brano arrivato
    /// dal gruppo è in caricamento: altrimenti si sovrascriverebbe la stanza con dati vecchi. Fuori
    /// da una jam costa un confronto; dentro, la coda si confronta senza copiarla e lo stato si
    /// costruisce solo quando parte.
    pub(crate) fn publish(&mut self, playback: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        if self.closing || !self.view.is_active() || !self.synced || self.fetching.is_some() {
            return;
        }
        if matches!(playback.state, PlaybackState::Loading { .. }) {
            return;
        }
        let Some(index) = playback.queue.position() else { return };
        // Una coda più lunga di quella che il server accetta non si condivide.
        if playback.queue.len() > jam::MAX_QUEUE {
            return;
        }
        let playing = playback.is_playing();
        let position_ms = jam::millis(playback.position(now));
        let ids = || playback.queue.entries().map(|(_, track)| track.id);
        if !jam::should_send_state(self.last_sent.as_ref(), ids(), index, position_ms, playing, now) {
            return;
        }
        let snapshot = jam::Snapshot { queue: ids().collect(), index, position_ms, playing };
        effects.push(Effect::Jam(JamOp::Send(snapshot.clone())));
        self.last_sent = Some(Sent { snapshot, at: now });
    }
}

impl Job {
    /// Esegue il lavoro: i brani del catalogo, pochi alla volta. L'ordine non conta, il dominio
    /// riallinea per id.
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Tracks { api, ids, ticket } => {
                let result = match api {
                    Ok(api) => {
                        futures_util::stream::iter(ids)
                            .map(|id| api.track(id))
                            .buffer_unordered(FETCH_PARALLEL)
                            .try_collect()
                            .await
                    }
                    Err(err) => Err(err),
                };
                Done::Tracks { ticket, result }
            }
        }
    }

    /// Se il lavoro va perso, il dominio non resta ad aspettare: fallisce come senza rete.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Tracks { ticket, .. } => Done::Tracks { ticket: *ticket, result: Err(AppError::Network) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::models::fixtures::track;
    use crate::command::Command;
    use crate::effect::{Done as AnyDone, TimerKey};
    use crate::jam::Snapshot as Shared;
    use crate::player::stream::StreamBuffer;
    use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone, Job as PlaybackJob, Timer};
    use crate::state::{CoreState, Input};

    const TICK: Duration = Duration::from_millis(250);

    fn id(n: u128) -> Uuid {
        track(n).id
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn shared(queue: &[u128], index: usize, position_ms: u64, playing: bool) -> Shared {
        Shared { queue: queue.iter().map(|n| id(*n)).collect(), index, position_ms, playing }
    }

    fn remote(from: &str, queue: &[u128], index: usize, position_ms: u64, playing: bool) -> Remote {
        Remote { snapshot: shared(queue, index, position_ms, playing), from: from.into() }
    }

    fn joined(code: &str, state: Option<Remote>) -> jam::Event {
        jam::Event::Joined {
            code: code.into(),
            you: Uuid::from_u128(77),
            members: vec![
                Member { id: Uuid::from_u128(77), name: "Prova".into() },
                Member { id: Uuid::from_u128(78), name: "Luca".into() },
            ],
            state,
        }
    }

    /// Il core intero (stesso passaggio fra i domini di quello vero) con il tempo in mano al test e
    /// l'audio finto: ogni `Play` parte, e il battito scocca a mano.
    struct Rig {
        state: CoreState,
        now: Instant,
        /// Ciò che la jam ha chiesto all'host, dall'ultima lettura.
        ops: Vec<JamOp>,
        /// I brani chiesti al catalogo: (numero della richiesta, id).
        fetches: Vec<(u64, Vec<Uuid>)>,
        /// I caricamenti chiesti: (brano, generazione).
        loads: Vec<(Uuid, u64)>,
        /// I timer armati in tutto, a parte il battito e il salvataggio delle impostazioni.
        other_timers: usize,
        ticking: bool,
        audio: Vec<AudioOp>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                state: CoreState::default(),
                now: Instant::now(),
                ops: Vec::new(),
                fetches: Vec::new(),
                loads: Vec::new(),
                other_timers: 0,
                ticking: false,
                audio: Vec::new(),
            }
        }

        /// Dà l'ingresso e rimette in giro ciò che l'audio risponderebbe, come fa l'host.
        fn feed(&mut self, input: Input) {
            let mut pending = VecDeque::from([input]);
            while let Some(input) = pending.pop_front() {
                for effect in self.state.apply(input, self.now) {
                    match effect {
                        Effect::Jam(op) => self.ops.push(op),
                        Effect::Run(effect::Job::Jam(Job::Tracks { ids, ticket, .. })) => {
                            self.fetches.push((ticket, ids));
                        }
                        Effect::Run(effect::Job::Playback(PlaybackJob::Load {
                            track_id,
                            generation,
                            ..
                        })) => self.loads.push((track_id, generation)),
                        Effect::Audio(op) => {
                            if matches!(op, AudioOp::Play { .. }) {
                                pending.push_back(Input::Audio(AudioFact::Started(Ok(()))));
                            }
                            self.audio.push(op);
                        }
                        Effect::Timer { key: TimerKey::Playback(Timer::Tick), .. } => self.ticking = true,
                        Effect::CancelTimer(TimerKey::Playback(Timer::Tick)) => self.ticking = false,
                        // Il salvataggio ritardato di `config.json` non è della jam.
                        Effect::Timer { key: TimerKey::Settings(_), .. } => {}
                        Effect::Timer { .. } => self.other_timers += 1,
                        _ => {}
                    }
                }
            }
        }

        fn jam(&mut self, command: JamCmd) {
            self.feed(Input::Command(Command::Jam(command)));
        }

        fn playback(&mut self, command: PlaybackCmd) {
            self.feed(Input::Command(Command::Playback(command)));
        }

        fn event(&mut self, event: jam::Event) {
            self.feed(Input::Jam(event));
        }

        fn elapse(&mut self, by: Duration) {
            self.now += by;
        }

        fn view(&self) -> Arc<JamView> {
            Arc::clone(&self.state.jam.view)
        }

        fn events(&self) -> Vec<JamEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }

        /// Gli stati mandati alla stanza dall'ultima lettura.
        fn sent(&mut self) -> Vec<Shared> {
            std::mem::take(&mut self.ops)
                .into_iter()
                .filter_map(|op| match op {
                    JamOp::Send(snapshot) => Some(snapshot),
                    _ => None,
                })
                .collect()
        }

        fn queue_ids(&self) -> Vec<Uuid> {
            self.state.playback.view.queue.entries().map(|(_, track)| track.id).collect()
        }

        fn position_ms(&self) -> u64 {
            jam::millis(self.state.playback.view.position(self.now))
        }

        /// I brani suonano dal primo: caricati, partiti.
        fn play(&mut self, queue: &[u128]) {
            let tracks: Vec<Track> = queue.iter().map(|n| track(*n)).collect();
            self.playback(PlaybackCmd::Play { tracks: tracks.into(), start: 0 });
            self.load_arrives();
            assert!(self.state.playback.view.is_playing(), "{:?}", self.state.playback.view.state);
        }

        /// Il server manda l'ultimo brano chiesto.
        fn load_arrives(&mut self) {
            let (_, generation) = self.loads.pop().expect("un caricamento chiesto");
            self.loads.clear();
            self.feed(Input::Done(AnyDone::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })));
        }

        /// Un battito della riproduzione: l'audio ha raggiunto `heard` dall'inizio del brano.
        fn beat(&mut self, heard: Duration) {
            self.elapse(TICK);
            assert!(self.ticking, "il battito deve essere armato");
            self.feed(Input::Timer(TimerKey::Playback(Timer::Tick)));
            self.feed(Input::Audio(AudioFact::Polled {
                position: heard,
                finished: false,
                seek_failed: false,
            }));
        }

        /// Battiti da `from` a `until`, uno ogni 250 ms, come fa un brano che suona.
        fn listen(&mut self, from: Duration, until: Duration) {
            let mut at = from;
            while at < until {
                at += TICK;
                self.beat(at);
            }
        }

        fn answer(&mut self, ticket: u64, result: Result<Vec<Track>, AppError>) {
            self.feed(Input::Done(AnyDone::Jam(Done::Tracks { ticket, result })));
        }

        /// Una jam aperta in una stanza nuova (senza stato), con `queue` che suona.
        fn open_room(queue: &[u128]) -> Self {
            let mut rig = Self::new();
            rig.play(queue);
            rig.jam(JamCmd::Create { name: "Prova".into() });
            rig.event(joined("K7QXM3", None));
            rig
        }
    }

    // ---------- Comandi ----------

    #[test]
    fn creating_a_jam_asks_the_host_for_a_room_and_puts_playback_in_the_group() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: "  Luca  ".into() });
        assert_eq!(
            std::mem::take(&mut rig.ops),
            [JamOp::Start { base: config::api_url(None), target: Target::Create, name: "Luca".into() }]
        );
        let view = rig.view();
        assert_eq!((view.phase, view.code.as_deref()), (JamPhase::Connecting, None));
        assert!(view.is_active() && view.events.is_empty());
        assert!(rig.state.playback.grouped(), "la coda è del gruppo: niente autoplay");
        assert_eq!(rig.other_timers, 0);
    }

    #[test]
    fn joining_sends_the_code_and_a_second_start_is_refused() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        assert!(matches!(
            rig.ops.as_slice(),
            [JamOp::Start { target: Target::Join(code), name, .. }] if code == "K7QXM3" && name.is_empty()
        ));
        rig.ops.clear();
        rig.jam(JamCmd::Create { name: "x".into() });
        rig.jam(JamCmd::Join { code: "AAAAAA".into(), name: "x".into() });
        assert!(rig.ops.is_empty(), "una jam c'è già: il collegamento non riparte");
        assert_eq!(rig.events(), [JamEvent::AlreadyIn, JamEvent::AlreadyIn]);
        assert_eq!(rig.view().phase, JamPhase::Connecting);
    }

    #[test]
    fn a_bad_code_never_reaches_the_host() {
        let mut rig = Rig::new();
        for code in ["", "K7QXM", "K7QXM3X", "K7-XM3", "K7QXM·"] {
            rig.jam(JamCmd::Join { code: code.into(), name: String::new() });
        }
        assert!(rig.ops.is_empty());
        assert!(!rig.view().is_active());
        assert_eq!(rig.events(), vec![JamEvent::BadCode; 5]);
        assert_eq!(JamEvent::BadCode.message(), "Il codice ha 6 lettere o cifre");
        assert!(JamEvent::BadCode.is_error());
        assert!(!rig.state.playback.grouped());
    }

    #[test]
    fn the_name_typed_wins_then_the_profile_and_both_are_cut_to_the_room_limit() {
        let name_of = |ops: &[JamOp]| match ops {
            [JamOp::Start { name, .. }] => name.clone(),
            other => panic!("{other:?}"),
        };
        let mut jam = Jam::default();
        let mut effects = Vec::new();
        jam.set_profile(Some("Sara Rossi"));
        jam.apply(JamCmd::Create { name: "   ".into() }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops), "Sara Rossi", "senza un nome scritto, quello del profilo");

        jam.apply(JamCmd::Leave, Instant::now(), &mut effects);
        effects.clear();
        jam.apply(JamCmd::Create { name: "Luca".into() }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops), "Luca", "il nome scritto vince sul profilo");

        jam.apply(JamCmd::Leave, Instant::now(), &mut effects);
        effects.clear();
        let long = "x".repeat(jam::MAX_NAME_CHARS + 20);
        jam.apply(JamCmd::Create { name: long }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops).chars().count(), jam::MAX_NAME_CHARS);
    }

    // ---------- Ingresso ----------

    #[test]
    fn the_room_answers_with_its_code_and_members() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: String::new() });
        rig.event(joined("K7QXM3", None));
        let view = rig.view();
        assert_eq!(view.phase, JamPhase::Open);
        assert_eq!(view.code.as_deref(), Some("K7QXM3"));
        assert_eq!(view.you, Some(Uuid::from_u128(77)));
        assert_eq!(view.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Prova", "Luca"]);
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())]);
        assert_eq!(JamEvent::Joined("K7QXM3".into()).message(), "Sei nella jam K7QXM3");
        assert!(!JamEvent::Joined("K7QXM3".into()).is_error());
    }

    /// BACKLOG 10: chi entra in una jam già avviata vede «Sei nella jam CODE», e lo stato che la
    /// stanza manda all'ingresso non è «un altro ha cambiato brano»: l'avviso non lo sostituisce.
    #[test]
    fn a_late_joiner_keeps_the_you_are_in_the_jam_notice() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, true))));
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())], "nessun «ha cambiato brano»");
        assert_eq!(
            rig.view().events.back().map(|raised| raised.event.message()),
            Some("Sei nella jam K7QXM3".to_owned()),
            "l'ultimo avviso è il codice"
        );
    }

    #[test]
    fn a_late_joiner_fetches_the_missing_tracks_then_aligns_to_the_shared_position() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, true))));

        // I brani del gruppo mancano: si chiedono al catalogo (una volta sola) e alla stanza non si
        // manda niente.
        assert_eq!(rig.fetches.len(), 1);
        assert_eq!(rig.fetches[0].1, [id(1), id(2)]);
        assert!(rig.sent().is_empty());
        assert!(rig.state.jam.fetch_ticket().is_some());

        // Arrivati in un ordine qualsiasi, la coda prende quella del gruppo e parte il brano
        // condiviso. Mentre si carica, la stanza non riceve nulla.
        let ticket = rig.fetches[0].0;
        rig.answer(ticket, Ok(vec![track(2), track(1)]));
        assert_eq!(rig.queue_ids(), [id(1), id(2)]);
        assert_eq!(rig.state.playback.view.queue.position(), Some(1));
        assert_eq!(rig.state.playback.view.state, PlaybackState::Loading { track_id: id(2) });
        assert!(rig.sent().is_empty(), "un brano del gruppo in caricamento non si annuncia");
        assert!(rig.state.jam.fetch_ticket().is_none());

        // L'audio pronto parte dalla posizione del gruppo, non da capo, e non si rimanda indietro.
        rig.load_arrives();
        assert!(rig.state.playback.view.is_playing());
        assert_eq!(rig.position_ms(), 12_000);
        assert!(rig.audio.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == secs(12))));
        assert!(rig.sent().is_empty(), "lo stato è quello appena ricevuto");
        rig.listen(secs(12), Duration::from_millis(12_500));
        assert!(rig.sent().is_empty(), "il battito che segue il gruppo non manda niente");
    }

    #[test]
    fn a_late_joiner_of_a_paused_jam_waits_at_the_shared_position_and_the_first_local_change_is_sent() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, false))));
        let ticket = rig.fetches[0].0;
        rig.answer(ticket, Ok(vec![track(1), track(2)]));
        rig.load_arrives();
        assert_eq!(rig.state.playback.view.state, PlaybackState::Paused, "il gruppo è in pausa");
        assert_eq!(rig.position_ms(), 12_000);
        assert!(rig.sent().is_empty());

        // Il primo cambio locale, invece, si annuncia: la ripresa, alla posizione condivisa.
        rig.playback(PlaybackCmd::Toggle);
        let sent = rig.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!((&sent[0].queue, sent[0].index, sent[0].playing), (&vec![id(1), id(2)], 1, true));
        assert!(sent[0].position_ms.abs_diff(12_000) <= 50, "{}", sent[0].position_ms);
    }

    #[test]
    fn a_state_whose_tracks_are_all_in_the_queue_aligns_without_asking_the_catalogue() {
        let mut rig = Rig::new();
        rig.play(&[1, 2, 3]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2, 3], 2, 40_000, true))));
        assert!(rig.fetches.is_empty());
        assert_eq!(rig.state.playback.view.queue.position(), Some(2));
        assert_eq!(rig.state.playback.view.state, PlaybackState::Loading { track_id: id(3) });
        assert!(rig.sent().is_empty());
    }

    #[test]
    fn a_remote_queue_with_other_tracks_and_a_known_one_fetches_only_what_is_missing() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        // Il gruppo ha 3 e 1 (2 non c'è, e 3 compare due volte): si chiede il solo 3, una volta.
        rig.event(joined("K7QXM3", Some(remote("Luca", &[3, 1, 3], 0, 0, true))));
        assert_eq!(rig.fetches.len(), 1);
        assert_eq!(rig.fetches[0].1, [id(3)]);
        rig.answer(rig.fetches[0].0, Ok(vec![track(3)]));
        assert_eq!(rig.queue_ids(), [id(3), id(1), id(3)]);
    }

    #[test]
    fn a_newer_state_supersedes_the_tracks_still_in_flight() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        let first = rig.fetches[0].0;
        rig.event(jam::Event::State(remote("Luca", &[3, 4], 1, 5_000, true)));
        assert_eq!(rig.fetches.len(), 2);
        let second = rig.fetches[1].0;
        assert_ne!(first, second);

        // La risposta del vecchio stato è superata: la coda non cambia e il resto aspetta.
        rig.answer(first, Ok(vec![track(1), track(2)]));
        assert!(rig.queue_ids().is_empty());
        assert!(rig.state.jam.fetch_ticket().is_some(), "si aspetta ancora il secondo");

        rig.answer(second, Ok(vec![track(3), track(4)]));
        assert_eq!(rig.queue_ids(), [id(3), id(4)]);
        assert_eq!(rig.state.playback.view.queue.position(), Some(1));
    }

    #[test]
    fn a_state_that_needs_nothing_cancels_the_wait_for_the_old_one() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[8, 9], 0, 0, true))));
        let stale = rig.fetches[0].0;
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 1, 3_000, true)));
        assert!(rig.state.jam.fetch_ticket().is_none());
        rig.answer(stale, Ok(vec![track(8), track(9)]));
        assert_eq!(rig.queue_ids(), [id(1), id(2)], "la risposta superata non tocca la coda");
    }

    #[test]
    fn a_failed_fetch_says_so_and_changes_nothing_and_missing_tracks_are_reported() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        rig.answer(rig.fetches[0].0, Err(AppError::Network));
        assert!(rig.queue_ids().is_empty());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::TracksFailed(AppError::Network));
        assert!(last.event.is_error());
        assert_eq!(
            last.event.message(),
            "Brani della jam non caricati: Server non raggiungibile. Controlla la connessione."
        );

        // Il catalogo manda solo uno dei due: la coda resta com'è, e lo si dice.
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 0, true)));
        rig.answer(rig.fetches[1].0, Ok(vec![track(1)]));
        assert!(rig.queue_ids().is_empty());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::TracksMissing);
        assert_eq!(last.event.message(), "Alcuni brani della jam non sono disponibili");
    }

    #[test]
    fn a_change_of_track_by_someone_else_is_announced_but_a_steady_heartbeat_is_not() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())]);
        rig.elapse(secs(5));
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 5_000, true)));
        assert_eq!(rig.events().len(), 1, "stesso brano: niente da dire");
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 1, 0, true)));
        assert_eq!(
            rig.view().events.back().map(|raised| raised.event.clone()),
            Some(JamEvent::TrackChanged { from: "Luca".into() })
        );
        assert_eq!(JamEvent::TrackChanged { from: "Luca".into() }.message(), "Luca ha cambiato brano");
    }

    #[test]
    fn an_empty_shared_queue_clears_the_local_one() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[], 0, 0, false))));
        assert!(rig.queue_ids().is_empty());
        assert!(rig.fetches.is_empty());
    }

    #[test]
    fn members_change_the_view_only_when_they_change_and_a_rejection_keeps_the_jam_open() {
        let mut rig = Rig::open_room(&[1]);
        let before = rig.view();
        let same = vec![
            Member { id: Uuid::from_u128(77), name: "Prova".into() },
            Member { id: Uuid::from_u128(78), name: "Luca".into() },
        ];
        rig.event(jam::Event::Members(same));
        assert!(Arc::ptr_eq(&before, &rig.view()), "gli stessi membri non cambiano la vista");
        rig.event(jam::Event::Members(vec![Member { id: Uuid::from_u128(77), name: "Prova".into() }]));
        assert_eq!(rig.view().members.len(), 1);

        rig.event(jam::Event::Rejected("Troppi cambiamenti in poco tempo: riprova tra un attimo".into()));
        let last = rig.view().events.back().cloned().expect("an event");
        assert!(last.event.is_error());
        assert_eq!(last.event.message(), "Troppi cambiamenti in poco tempo: riprova tra un attimo");
        assert_eq!(rig.view().phase, JamPhase::Open, "il collegamento resta aperto");
    }

    // ---------- Uscita ----------

    #[test]
    fn leaving_closes_the_link_leaves_the_group_and_a_late_event_is_ignored() {
        let mut rig = Rig::open_room(&[1]);
        rig.ops.clear();
        rig.jam(JamCmd::Leave);
        assert_eq!(std::mem::take(&mut rig.ops), [JamOp::Leave]);
        let view = rig.view();
        assert!(!view.is_active() && view.code.is_none() && view.you.is_none() && view.members.is_empty());
        assert_eq!(view.events.back().map(|raised| raised.event.clone()), Some(JamEvent::Left));
        assert_eq!(JamEvent::Left.message(), "Sei uscito dalla jam");
        assert!(!rig.state.playback.grouped(), "fuori dal gruppo l'autoplay torna");

        // Ciò che il collegamento aveva ancora in canna non riapre niente.
        let events = rig.events().len();
        rig.event(jam::Event::State(remote("Luca", &[1], 0, 0, true)));
        rig.event(jam::Event::Closed("Jam chiusa".into()));
        rig.event(joined("K7QXM3", None));
        assert!(!rig.view().is_active());
        assert_eq!(rig.events().len(), events);
        assert!(rig.ops.is_empty() && rig.fetches.is_empty());
        // Uscire di nuovo non fa niente.
        rig.jam(JamCmd::Leave);
        assert!(rig.ops.is_empty());
        assert_eq!(rig.events().len(), events);
    }

    #[test]
    fn the_room_closing_ends_the_jam_and_tells_why() {
        let mut rig = Rig::open_room(&[1]);
        rig.ops.clear();
        rig.event(jam::Event::Closed("Jam chiusa dal server".into()));
        assert!(!rig.view().is_active());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::Closed("Jam chiusa dal server".into()));
        assert!(last.event.is_error());
        assert_eq!(last.event.message(), "Jam chiusa dal server");
        assert!(!rig.state.playback.grouped());
        assert!(rig.ops.is_empty(), "il collegamento è già finito: non c'è altro da chiudere");

        // Si può ripartire da capo.
        rig.jam(JamCmd::Create { name: String::new() });
        assert!(matches!(rig.ops.as_slice(), [JamOp::Start { .. }]));
    }

    #[test]
    fn a_connection_that_never_opens_ends_the_jam_with_the_reason() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: String::new() });
        rig.event(jam::Event::Closed("Il server non risponde".into()));
        assert!(!rig.view().is_active());
        assert!(!rig.state.playback.grouped());
        assert_eq!(rig.events(), [JamEvent::Closed("Il server non risponde".into())]);
    }

    // ---------- Quando si manda lo stato ----------

    /// Fermo, una jam non manda nulla e non arma nessun timer; ogni cambio vero è un invio solo
    /// (e mai mentre il brano si carica), e il battito ogni cinque secondi mentre suona.
    #[test]
    fn the_local_state_is_sent_on_real_transitions_and_on_a_five_second_heartbeat() {
        let mut rig = Rig::open_room(&[1, 2]);
        // Entrati in una stanza nuova con la coda che suona: il primo stato è il proprio.
        let first = rig.sent();
        assert_eq!(first, [shared(&[1, 2], 0, 0, true)]);
        assert_eq!(rig.other_timers, 0, "la jam non arma nessun timer");

        // Quattro secondi di ascolto: nessun invio (la posizione segue l'orologio).
        rig.listen(Duration::ZERO, secs(4));
        assert!(rig.sent().is_empty());
        // Al quinto secondo, il battito.
        rig.listen(secs(4), secs(5));
        let beat = rig.sent();
        assert_eq!(beat.len(), 1);
        assert!(beat[0].playing && beat[0].position_ms.abs_diff(5_000) <= 300, "{beat:?}");
        rig.listen(secs(5), secs(9));
        assert!(rig.sent().is_empty());

        // Pausa: un invio solo. In pausa, nient'altro: il core non ha più battito.
        rig.playback(PlaybackCmd::Toggle);
        let paused = rig.sent();
        assert_eq!(paused.len(), 1);
        assert!(!paused[0].playing);
        assert!(!rig.ticking);
        rig.elapse(secs(60));
        rig.playback(PlaybackCmd::Volume(0.5));
        assert!(rig.sent().is_empty(), "un minuto di pausa non rimanda niente");

        // Un salto in pausa si manda; la ripresa anche.
        rig.playback(PlaybackCmd::SeekTo(secs(100)));
        let jumped = rig.sent();
        assert_eq!(jumped.len(), 1);
        assert_eq!(jumped[0].position_ms, 100_000);
        rig.playback(PlaybackCmd::Toggle);
        assert_eq!(rig.sent().len(), 1);
    }

    #[test]
    fn a_new_track_is_sent_once_when_it_plays_and_never_while_it_loads() {
        let mut rig = Rig::open_room(&[1, 2]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(2));
        rig.playback(PlaybackCmd::Next);
        assert!(matches!(rig.state.playback.view.state, PlaybackState::Loading { .. }));
        assert!(rig.sent().is_empty(), "in caricamento non si manda niente");
        rig.load_arrives();
        let sent = rig.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!((sent[0].index, sent[0].playing), (1, true));
        assert_eq!(sent[0].queue, [id(1), id(2)]);
    }

    #[test]
    fn a_change_of_the_queue_is_sent_and_a_queue_too_long_for_the_server_is_not() {
        let mut rig = Rig::open_room(&[1]);
        rig.sent();
        rig.playback(PlaybackCmd::Enqueue(Box::new(track(2))));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].queue, [id(1), id(2)]);

        let many: Vec<Track> = (1..=jam::MAX_QUEUE as u128 + 1).map(track).collect();
        // Dal sesto: il primo era l'ultimo suonato e si riascolta dal buffer, senza caricamento.
        rig.playback(PlaybackCmd::Play { tracks: many.into(), start: 5 });
        rig.load_arrives();
        assert!(rig.sent().is_empty(), "più di {} brani non si condividono", jam::MAX_QUEUE);
    }

    #[test]
    fn nothing_is_sent_before_the_room_has_told_its_state_nor_with_nothing_queued() {
        let mut rig = Rig::new();
        rig.play(&[1]);
        rig.jam(JamCmd::Create { name: String::new() });
        assert!(rig.sent().is_empty(), "la stanza non ha ancora risposto");
        rig.playback(PlaybackCmd::Toggle);
        assert!(rig.sent().is_empty());
        rig.playback(PlaybackCmd::Toggle);

        let mut empty = Rig::new();
        empty.jam(JamCmd::Create { name: String::new() });
        empty.event(joined("K7QXM3", None));
        assert!(empty.sent().is_empty(), "niente in coda, niente da condividere");
        empty.play(&[1]);
        assert_eq!(empty.sent().len(), 1, "la prima coda parte appena c'è");
    }

    /// Chiudere il core ferma la riproduzione, e a quel punto la stanza non deve sentire «in
    /// pausa»: sarebbe un modo di fermare l'ascolto degli altri.
    #[test]
    fn closing_the_core_does_not_tell_the_room_that_playback_stopped() {
        let mut rig = Rig::open_room(&[1]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(1));
        rig.feed(Input::Shutdown);
        assert!(!rig.state.playback.view.is_playing());
        assert!(rig.sent().is_empty());
    }

    #[test]
    fn a_remote_state_is_the_reference_and_a_local_jump_after_it_is_sent() {
        let mut rig = Rig::open_room(&[1, 2]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(1));
        // Il gruppo è a 31 s sullo stesso brano: ci si sposta e non si rimanda lo stesso stato.
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 31_000, true)));
        assert!(rig.sent().is_empty());
        assert_eq!(rig.position_ms(), 31_000);
        // Un salto locale lontano si annuncia.
        rig.playback(PlaybackCmd::SeekTo(secs(90)));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].position_ms, 90_000);
    }

    // ---------- Vista ----------

    #[test]
    fn events_keep_the_last_32_with_growing_numbers() {
        let mut rig = Rig::new();
        for _ in 0..40 {
            rig.jam(JamCmd::Join { code: "x".into(), name: String::new() });
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.front().map(|raised| raised.seq), Some(9));
        assert_eq!(view.events.back().map(|raised| raised.seq), Some(40));
    }

    #[test]
    fn a_job_that_is_lost_fails_like_a_missing_network() {
        let job = Job::Tracks { api: Err(AppError::Network), ids: vec![id(1)], ticket: 5 };
        let Done::Tracks { ticket, result } = job.lost();
        assert_eq!((ticket, result), (5, Err(AppError::Network)));
    }

    #[tokio::test]
    async fn a_fetch_with_no_client_reports_the_client_error() {
        let job = Job::Tracks { api: Err(AppError::InvalidBaseUrl("x".into())), ids: vec![id(1)], ticket: 2 };
        let Done::Tracks { ticket, result } = job.run().await;
        assert_eq!((ticket, result), (2, Err(AppError::InvalidBaseUrl("x".into()))));
    }
}
