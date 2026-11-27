//! Videoclip (passo 4.9): la metà «di affari» del video. La pipeline di GStreamer, i fotogrammi e il
//! widget restano nel frontend; qui sta tutto il resto.
//!
//! **Quale videoclip.** Un frontend dice che mostra il video ([`VideoCmd::Show`], come
//! `PlaybackCmd::Lyrics` per il testo): finché non lo dice il core non chiede niente al server. Con
//! il video mostrato e un brano che ne ha uno (`has_video`) il core chiede l'indirizzo
//! (`GET /v1/tracks/{id}/video-url`: la prima volta il server scarica il videoclip, a 360p subito e a
//! 720p in sottofondo) e lo pubblica in [`VideoView`]. Il frontend apre l'indirizzo
//! ([`Clip::url`]); un [`Clip::seq`] nuovo è un altro indirizzo da aprire.
//!
//! **Dal 360p al 720p.** Mentre il videoclip mostrato è sotto [`FULL_HEIGHT`] il core riarma un
//! timer ([`Timer::Upgrade`], [`UPGRADE_WAIT`], al massimo [`MAX_UPGRADES`] volte) e chiede di nuovo
//! l'indirizzo: se il server ha una versione più alta la pubblica con un `seq` nuovo, e il frontend la
//! apre di fianco e la sostituisce senza fermarsi. Il timer è armato solo con un videoclip mostrato
//! e non ancora completo: un core fermo, un brano senza video o il video nascosto non armano niente.
//!
//! **Quando non si apre.** Il frontend dice [`VideoCmd::Failed`] se l'indirizzo non si apre o lo stream
//! cade: il core riprova una volta con un indirizzo nuovo (può essere scaduto) e poi ferma il
//! videoclip di quel brano (`ClipState::Failed`) finché non cambia il brano o arriva
//! [`VideoCmd::Retry`]. Un aggiornamento che non si apre non ferma niente: resta la versione che
//! suonava e il core riprova più tardi.
//!
//! **In anticipo.** Con [`VideoCmd::PrepareAhead`] il server prepara i videoclip del brano che parte e
//! dei [`AHEAD`] successivi (`POST /v1/tracks/{id}/video/prepare`, una volta per brano, con un
//! elenco limitato dei brani già chiesti).
//!
//! **L'allineamento all'audio.** Le decisioni di sincronia sono funzioni pure ([`Alignment`]): il
//! frontend misura il suo player ([`Measure`]) e applica ciò che torna ([`Adjust`]); la posizione
//! da raggiungere la dà l'orologio della riproduzione (`clock.at(now)`), la soglia di scarto
//! ([`DRIFT`]), la pausa fra due riallineamenti ([`RESYNC_PAUSE`]) e l'anticipo ([`MAX_LEAD`]).
//! Nessun comando e nessuna pubblicazione per questo: il frontend lo chiede al suo Tick, che c'è già
//! mentre suona.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{ApiClient, SignedUrl};
use crate::auth::Secret;
use crate::config;
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::state::playback::PlaybackView;

/// Brani della coda, dopo quello corrente, di cui il server prepara il videoclip.
pub const AHEAD: usize = 2;
/// Altezza della versione completa: sotto, si aspetta che il server la prepari.
pub const FULL_HEIGHT: u32 = 720;
/// Ogni quanto si chiede se la versione completa è pronta, e quante volte al massimo.
pub const UPGRADE_WAIT: Duration = Duration::from_secs(6);
pub const MAX_UPGRADES: u32 = 10;
/// Scarto tollerato fra video e audio prima di riallineare il video.
pub const DRIFT: Duration = Duration::from_millis(200);
/// Dopo un riallineamento si lascia al video il tempo di ripartire.
pub const RESYNC_PAUSE: Duration = Duration::from_secs(2);
/// Massimo anticipo con cui si mira un riallineamento (vedi [`Alignment`]).
pub const MAX_LEAD: Duration = Duration::from_millis(1500);
/// Quanto si aspetta, sul thread che apre il video, che il seek iniziale arrivi a destinazione.
pub const SETTLE: Duration = Duration::from_millis(600);
/// Brani di cui si ricorda di aver chiesto la preparazione (scarto: il meno recente).
const PREPARED: usize = 128;

// ---------- Allineamento all'audio: funzioni pure, del frontend ----------

/// Ciò che il frontend misura sul suo player a ogni battito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    /// Dove si trova il video.
    pub position: Duration,
    /// La durata del video; zero se non la conosce ancora.
    pub duration: Duration,
    /// Il player è in pausa.
    pub paused: bool,
    /// Il video è arrivato in fondo.
    pub ended: bool,
}

/// Ciò che il frontend fa al suo player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Adjust {
    /// `Some(true)`: metti in pausa; `Some(false)`: fai ripartire.
    pub pause: Option<bool>,
    /// Porta il video a questa posizione.
    pub seek: Option<Duration>,
}

impl Adjust {
    /// Non c'è niente da fare.
    pub fn is_none(&self) -> bool {
        self.pause.is_none() && self.seek.is_none()
    }
}

/// Lo stato del riallineamento di un player: l'ultimo riallineamento e l'anticipo con cui si mira.
/// Lo tiene il frontend insieme al suo player (uno per videoclip aperto); qui stanno le regole.
#[derive(Debug, Clone, Copy)]
pub struct Alignment {
    /// Ultimo riallineamento all'audio: non lo si insegue a ogni battito.
    synced_at: Instant,
    /// Il seek stesso costa tempo (il video si ferma a bufferizzare mentre l'audio va avanti): si
    /// mira avanti di quanto il video è rimasto indietro le volte prima, o si insegue all'infinito.
    lead: Duration,
}

impl Alignment {
    /// Un player appena aperto, adesso.
    pub fn new(now: Instant) -> Self {
        Self { synced_at: now, lead: Duration::ZERO }
    }

    /// L'anticipo con cui si mira adesso.
    pub fn lead(&self) -> Duration {
        self.lead
    }

    /// La riproduzione è cambiata da `previous` a `view`: che cosa deve fare il video per seguirla.
    /// Un brano che parte o un salto nel brano portano il video alla posizione dell'audio; una
    /// pausa lo ferma; la ripresa lo fa ripartire dalla posizione dell'audio. `None` se non è
    /// cambiato niente che lo riguardi.
    pub fn follow(&mut self, previous: &PlaybackView, view: &PlaybackView, now: Instant) -> Option<Adjust> {
        let playing = view.is_playing();
        let moved = view.started != previous.started || view.seeks != previous.seeks;
        if !moved && view.state == previous.state {
            return None;
        }
        // Un salto, o la ripresa dopo una pausa: il video va dove sta l'audio. Una pausa lo ferma e
        // basta.
        let seek = (moved || playing).then(|| {
            self.synced_at = now;
            view.position(now)
        });
        Some(Adjust { pause: Some(!playing), seek })
    }

    /// Al battito, o con `force` appena aperto un player: se il video si è allontanato dall'audio
    /// (rete lenta, decoder in ritardo) lo si riporta dov'è l'audio, al massimo ogni
    /// [`RESYNC_PAUSE`]. Pausa e ripresa seguono sempre la riproduzione. Un video più corto
    /// dell'audio resta sull'ultimo fotogramma.
    pub fn align(&mut self, playback: &PlaybackView, video: Measure, force: bool, now: Instant) -> Adjust {
        let playing = playback.is_playing();
        if !force && (!playing || now.saturating_duration_since(self.synced_at) < RESYNC_PAUSE) {
            return Adjust::default();
        }
        let mut adjust = Adjust::default();
        if video.paused == playing {
            adjust.pause = Some(!playing);
        }
        let audio = playback.position(now);
        if (!video.duration.is_zero() && audio >= video.duration) || video.ended {
            return adjust;
        }
        if video.position.abs_diff(audio) > DRIFT {
            self.lead = if video.position < audio {
                (self.lead + (audio - video.position)).min(MAX_LEAD)
            } else {
                self.lead.saturating_sub(video.position - audio)
            };
            adjust.seek = Some(audio + self.lead);
            self.synced_at = now;
        }
        adjust
    }
}

// ---------- Il dominio ----------

/// I comandi del dominio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoCmd {
    /// Un frontend mostra (o smette di mostrare) il videoclip del brano in corso: il pannello «In
    /// riproduzione» sul video, la vista a tutto schermo. Il core lo chiede al server solo finché
    /// serve.
    Show(bool),
    /// Chi ascolta vuole i videoclip: il server prepara quelli del brano che parte e dei prossimi.
    /// Da spento non si prepara niente. Il brano che è già in coda all'avvio si prepara solo
    /// quando un brano comincia a suonare.
    PrepareAhead(bool),
    /// Il videoclip con quel `seq` non si apre o lo stream è caduto. Una volta il core ne chiede un
    /// altro indirizzo, la seconda ferma il videoclip di quel brano (un `seq` che non è più quello
    /// pubblicato si ignora).
    Failed { seq: u64 },
    /// Toglie il blocco di un videoclip che non si apriva («Riprova», o chi cambia la scelta del
    /// video): se è ancora mostrato si chiede di nuovo.
    Retry,
    /// Lo toglie e lo chiede di nuovo, anche se andava: il frontend ha cambiato il modo di
    /// decodificarlo.
    Reopen,
}

/// Un videoclip da aprire: l'indirizzo firmato, l'altezza e il numero d'ordine.
#[derive(Clone, PartialEq, Eq)]
pub struct Clip {
    /// Cresce a ogni indirizzo nuovo: il frontend apre quello che non ha ancora aperto.
    pub seq: u64,
    /// L'altezza della versione servita (360 la prima, 720 la completa); `None` per un videoclip
    /// salvato prima delle due versioni, che è completo.
    pub height: Option<u32>,
    url: Secret,
}

impl Clip {
    /// L'indirizzo da aprire. Firmato e a scadenza: non va nei log (il `Debug` non lo stampa).
    pub fn url(&self) -> &str {
        self.url.expose()
    }

    /// È la versione completa: non c'è altro da aspettare.
    pub fn is_full(&self) -> bool {
        self.height.is_none_or(|height| height >= FULL_HEIGHT)
    }
}

impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clip").field("seq", &self.seq).field("height", &self.height).finish_non_exhaustive()
    }
}

/// A che punto è il videoclip di [`VideoView::track`].
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ClipState {
    /// Nessuno lo mostra, o il brano non ha un videoclip.
    #[default]
    Idle,
    /// Il server lo sta preparando (la prima volta lo scarica: può volerci mezzo minuto).
    Loading,
    /// L'indirizzo da aprire; con un `seq` nuovo, una versione migliore da affiancare e sostituire.
    Ready(Clip),
    /// Non si è aperto, nemmeno al secondo tentativo (o il server non l'ha dato): il frontend
    /// mostra «Riprova» e manda [`VideoCmd::Retry`].
    Failed(AppError),
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoView {
    /// Il brano di cui si parla (`None` a riposo).
    pub track: Option<Uuid>,
    pub state: ClipState,
    /// Cresce a ogni videoclip che si ferma: un frontend avvisa l'ascoltatore quando il numero è
    /// nuovo (l'errore è in `state`).
    pub failures: u64,
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// È ora di chiedere se la versione completa è pronta. Armato solo con un videoclip mostrato e
    /// ancora sotto la versione completa.
    Upgrade,
}

/// I lavori asincroni del dominio (rete), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// L'indirizzo del videoclip di `track` per la richiesta `ticket`. La prima volta il server
    /// scarica il videoclip e risponde dopo (fino a due minuti).
    Url { api: Result<ApiClient, AppError>, track: Uuid, ticket: u64 },
    /// Il server prepara il videoclip di `id`, che suonerà presto: risponde subito.
    Prepare { api: Result<ApiClient, AppError>, id: Uuid },
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    Url {
        ticket: u64,
        result: Result<SignedUrl, AppError>,
    },
    /// Come è andata la preparazione non cambia niente: il server la fa o no.
    Prepared,
}

/// Dove si è con il videoclip del brano in corso.
#[derive(Debug, Default)]
enum Stage {
    #[default]
    Idle,
    Loading {
        track: Uuid,
    },
    Ready {
        track: Uuid,
        clip: Clip,
        /// La versione che questo indirizzo ha sostituito: se il frontend non lo apre, resta lei.
        replaced: Option<Clip>,
    },
    Failed {
        track: Uuid,
        error: AppError,
    },
}

impl Stage {
    fn track(&self) -> Option<Uuid> {
        match self {
            Self::Idle => None,
            Self::Loading { track } | Self::Ready { track, .. } | Self::Failed { track, .. } => Some(*track),
        }
    }
}

/// La richiesta dell'indirizzo in volo.
#[derive(Debug, Clone, Copy)]
struct Awaiting {
    ticket: u64,
    /// Chiede una versione migliore di quella mostrata, non il primo indirizzo.
    upgrade: bool,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Video {
    pub(crate) view: Arc<VideoView>,
    /// Il backend scelto nelle impostazioni e il client che ne viene.
    server: Option<String>,
    api: Result<ApiClient, AppError>,
    /// Un frontend mostra il videoclip.
    shown: bool,
    /// Chi ascolta vuole i videoclip: si preparano quelli in arrivo.
    ahead: bool,
    /// `PrepareAhead(true)` è appena arrivato: si prepara al prossimo passaggio.
    just_enabled: bool,
    /// L'ultimo `started` visto della riproduzione: cambia quando un brano comincia a suonare.
    started: u64,
    stage: Stage,
    /// La vista non è più quella dello stadio.
    dirty: bool,
    awaiting: Option<Awaiting>,
    ticket: u64,
    /// L'ultimo `seq` dato a un indirizzo.
    seq: u64,
    /// L'indirizzo nuovo dopo un guasto è già stato chiesto una volta per questo videoclip.
    retried: bool,
    /// Richieste della versione completa già fatte per questo videoclip.
    upgrades: u32,
    /// Il timer dell'aggiornamento è armato.
    armed: bool,
    failures: u64,
    /// Brani di cui si è già chiesto la preparazione, dal meno recente.
    prepared: VecDeque<Uuid>,
}

impl Default for Video {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Video {
    /// Il dominio con il backend scelto nelle impostazioni.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            shown: false,
            ahead: false,
            just_enabled: false,
            started: 0,
            stage: Stage::Idle,
            dirty: false,
            awaiting: None,
            ticket: 0,
            seq: 0,
            retried: false,
            upgrades: 0,
            armed: false,
            failures: 0,
            prepared: VecDeque::new(),
        }
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client si rifà.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    pub(crate) fn apply(&mut self, command: VideoCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            VideoCmd::Show(on) => self.shown = on,
            VideoCmd::PrepareAhead(on) => {
                self.just_enabled = on && !self.ahead;
                self.ahead = on;
            }
            VideoCmd::Failed { seq } => self.failed(seq, effects),
            VideoCmd::Retry => {
                if matches!(self.stage, Stage::Failed { .. }) {
                    self.set(Stage::Idle);
                }
            }
            VideoCmd::Reopen => {
                if matches!(self.stage, Stage::Loading { .. } | Stage::Ready { .. }) {
                    self.drop_clip(effects);
                }
            }
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Upgrade => {
                self.armed = false;
                let Stage::Ready { track, clip, .. } = &self.stage else { return };
                if clip.is_full() || self.upgrades >= MAX_UPGRADES {
                    return;
                }
                let track = *track;
                self.upgrades += 1;
                self.request(track, true, effects);
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Url { ticket, result } => {
                // Una risposta che non è dell'ultima richiesta (un altro brano, il video nascosto, un
                // altro tentativo) non vale più.
                let Some(awaiting) = self.awaiting.filter(|awaiting| awaiting.ticket == ticket) else {
                    return;
                };
                self.awaiting = None;
                if awaiting.upgrade {
                    self.upgraded(result, effects);
                } else {
                    self.opened(result, effects);
                }
            }
            Done::Prepared => {}
        }
    }

    /// Dopo ogni ingresso: il videoclip che serve al brano in corso, e ciò che il server deve
    /// preparare. Senza novità confronta e basta: un core fermo non arma niente e non pubblica.
    pub(crate) fn settle(&mut self, playback: &PlaybackView, effects: &mut Vec<Effect>) {
        let current = playback.queue.current().filter(|track| track.has_video).map(|track| track.id);
        self.settle_clip(current, effects);
        self.settle_ahead(playback, effects);
        self.show();
    }

    // ---------- Il videoclip del brano in corso ----------

    fn settle_clip(&mut self, current: Option<Uuid>, effects: &mut Vec<Effect>) {
        let held = self.stage.track();
        let failed = matches!(self.stage, Stage::Failed { .. });
        // Un videoclip che non si apriva resta fermo finché il brano non cambia.
        if failed && held == current {
            return;
        }
        let wanted = current.filter(|_| self.shown);
        if wanted == held && !failed {
            return;
        }
        self.drop_clip(effects);
        if let Some(track) = wanted {
            self.open(track, false, effects);
        }
    }

    /// Toglie il videoclip: niente timer, niente risposta da aspettare.
    fn drop_clip(&mut self, effects: &mut Vec<Effect>) {
        self.disarm(effects);
        self.awaiting = None;
        self.retried = false;
        self.upgrades = 0;
        if !matches!(self.stage, Stage::Idle) {
            self.set(Stage::Idle);
        }
    }

    /// Chiede l'indirizzo del videoclip di `track` da capo.
    fn open(&mut self, track: Uuid, retried: bool, effects: &mut Vec<Effect>) {
        self.retried = retried;
        self.upgrades = 0;
        self.set(Stage::Loading { track });
        self.request(track, false, effects);
    }

    fn request(&mut self, track: Uuid, upgrade: bool, effects: &mut Vec<Effect>) {
        self.ticket += 1;
        self.awaiting = Some(Awaiting { ticket: self.ticket, upgrade });
        let job = Job::Url { api: self.api.clone(), track, ticket: self.ticket };
        effects.push(Effect::Run(effect::Job::Video(job)));
    }

    /// Il primo indirizzo (o quello nuovo dopo un guasto) è arrivato.
    fn opened(&mut self, result: Result<SignedUrl, AppError>, effects: &mut Vec<Effect>) {
        let Stage::Loading { track } = self.stage else { return };
        match result {
            Ok(signed) => {
                let clip = self.clip(signed);
                self.set(Stage::Ready { track, clip, replaced: None });
                self.arm(effects);
            }
            Err(error) => self.fail(track, error, effects),
        }
    }

    /// La risposta alla richiesta di una versione migliore. Se non lo è, o la richiesta è fallita,
    /// si riprova più tardi, finché le volte non finiscono.
    fn upgraded(&mut self, result: Result<SignedUrl, AppError>, effects: &mut Vec<Effect>) {
        let Stage::Ready { track, clip, .. } = &self.stage else { return };
        let (track, shown) = (*track, clip.height);
        // Senza altezza è un videoclip salvato prima delle due versioni: è completo.
        let better = |signed: &SignedUrl| match (signed.height, shown) {
            (Some(offered), Some(shown)) => offered > shown,
            _ => true,
        };
        if let Ok(signed) = result
            && better(&signed)
        {
            let replaced = match std::mem::take(&mut self.stage) {
                Stage::Ready { clip, .. } => Some(clip),
                _ => None,
            };
            let clip = self.clip(signed);
            self.set(Stage::Ready { track, clip, replaced });
        }
        self.arm(effects);
    }

    fn clip(&mut self, signed: SignedUrl) -> Clip {
        self.seq += 1;
        Clip { seq: self.seq, height: signed.height, url: Secret::new(signed.url) }
    }

    /// Il frontend non è riuscito ad aprire (o a tenere aperto) il videoclip `seq`.
    fn failed(&mut self, seq: u64, effects: &mut Vec<Effect>) {
        let Stage::Ready { track, clip, replaced } = &mut self.stage else { return };
        if clip.seq != seq {
            return;
        }
        let track = *track;
        if let Some(previous) = replaced.take() {
            // L'aggiornamento non si è aperto: resta la versione che già suonava.
            self.set(Stage::Ready { track, clip: previous, replaced: None });
            self.arm(effects);
        } else if self.retried {
            self.fail(track, AppError::Video, effects);
        } else {
            // Un indirizzo scaduto o uno stream caduto: una volta se ne chiede uno nuovo.
            self.disarm(effects);
            self.open(track, true, effects);
        }
    }

    fn fail(&mut self, track: Uuid, error: AppError, effects: &mut Vec<Effect>) {
        self.disarm(effects);
        self.awaiting = None;
        self.failures += 1;
        self.set(Stage::Failed { track, error });
    }

    /// Arma il timer dell'aggiornamento se il videoclip mostrato non è ancora completo.
    fn arm(&mut self, effects: &mut Vec<Effect>) {
        let Stage::Ready { clip, .. } = &self.stage else { return };
        if clip.is_full() || self.upgrades >= MAX_UPGRADES {
            return;
        }
        self.armed = true;
        effects.push(Effect::Timer { key: TimerKey::Video(Timer::Upgrade), after: UPGRADE_WAIT });
    }

    fn disarm(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.armed) {
            effects.push(Effect::CancelTimer(TimerKey::Video(Timer::Upgrade)));
        }
    }

    fn set(&mut self, stage: Stage) {
        self.stage = stage;
        self.dirty = true;
    }

    /// La vista segue lo stadio, ma cambia (e si ripubblica) solo se qualcosa è diverso davvero.
    fn show(&mut self) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let (track, state) = match &self.stage {
            Stage::Idle => (None, ClipState::Idle),
            Stage::Loading { track } => (Some(*track), ClipState::Loading),
            Stage::Ready { track, clip, .. } => (Some(*track), ClipState::Ready(clip.clone())),
            Stage::Failed { track, error } => (Some(*track), ClipState::Failed(error.clone())),
        };
        if self.view.track != track || self.view.state != state || self.view.failures != self.failures {
            let view = Arc::make_mut(&mut self.view);
            view.track = track;
            view.state = state;
            view.failures = self.failures;
        }
    }

    // ---------- In anticipo ----------

    /// Quando un brano comincia a suonare (o chi ascolta accende i videoclip dopo che uno è già
    /// partito) il server prepara quello in corso e i prossimi. Ogni brano una volta.
    fn settle_ahead(&mut self, playback: &PlaybackView, effects: &mut Vec<Effect>) {
        let begun = std::mem::replace(&mut self.started, playback.started) != playback.started;
        let enabled = std::mem::take(&mut self.just_enabled);
        if !self.ahead || playback.started == 0 || !(begun || enabled) {
            return;
        }
        let queue = &playback.queue;
        let ahead = queue.current().into_iter().chain(queue.upcoming().take(AHEAD).map(|(_, track)| track));
        let ids: Vec<Uuid> =
            ahead.filter(|track| track.has_video && !track.video_ready).map(|track| track.id).collect();
        for id in ids {
            if self.remember(id) {
                effects.push(Effect::Run(effect::Job::Video(Job::Prepare { api: self.api.clone(), id })));
            }
        }
    }

    /// Ricorda che si è chiesto `id`; `false` se c'era già (e lo rimette fra i più recenti).
    fn remember(&mut self, id: Uuid) -> bool {
        let known = self.prepared.iter().position(|prepared| *prepared == id);
        if let Some(index) = known {
            self.prepared.remove(index);
        } else if self.prepared.len() >= PREPARED {
            self.prepared.pop_front();
        }
        self.prepared.push_back(id);
        known.is_none()
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Url { api, track, ticket } => {
                let result = match api {
                    Ok(api) => api.video_url(track).await,
                    Err(err) => Err(err),
                };
                Done::Url { ticket, result }
            }
            Self::Prepare { api, id } => {
                if let Ok(api) = api {
                    let _ = api.prepare_video(id).await;
                }
                Done::Prepared
            }
        }
    }

    /// Se il lavoro va perso, il dominio non resta ad aspettare: l'indirizzo non è arrivato.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Url { ticket, .. } => Done::Url { ticket: *ticket, result: Err(AppError::Network) },
            Self::Prepare { .. } => Done::Prepared,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::Track;
    use crate::api::models::fixtures::track;
    use crate::command::{Command, PlaybackCmd};
    use crate::effect::Done as AnyDone;
    use crate::player::stream::StreamBuffer;
    use crate::player::{Clock, PlaybackState};
    use crate::snapshot::{DomainVersions, Snapshot};
    use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone, Job as PlaybackJob};
    use crate::state::{CoreState, Input};

    fn id(n: u128) -> Uuid {
        track(n).id
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// Un brano del catalogo con il suo videoclip, ancora da scaricare sul server.
    fn clip_track(n: u128) -> Track {
        Track { has_video: true, ..track(n) }
    }

    fn signed(height: Option<u32>) -> SignedUrl {
        SignedUrl {
            kind: "video".into(),
            url: "https://media.test/v1/media?t=firmato".into(),
            expires_at: "2026-11-24T10:00:00Z".into(),
            height,
            format: None,
            quality: None,
        }
    }

    /// Il core intero (stesso passaggio fra i domini di quello vero) con il tempo in mano al test, un
    /// audio finto e le richieste al server raccolte: niente rete, niente orologio.
    struct Rig {
        state: CoreState,
        snapshot: Snapshot,
        now: Instant,
        /// Gli indirizzi chiesti al server, dall'ultima lettura: (brano, numero della richiesta).
        urls: Vec<(Uuid, u64)>,
        /// Le preparazioni chieste, dall'ultima lettura.
        prepares: Vec<Uuid>,
        /// Il timer dell'aggiornamento è armato, e quante volte è stato armato in tutto.
        upgrade: bool,
        arms: usize,
        loads: Vec<(Uuid, u64)>,
    }

    impl Rig {
        fn new() -> Self {
            let state = CoreState::default();
            let snapshot = state.snapshot();
            Self {
                state,
                snapshot,
                now: Instant::now(),
                urls: Vec::new(),
                prepares: Vec::new(),
                upgrade: false,
                arms: 0,
                loads: Vec::new(),
            }
        }

        fn feed(&mut self, input: Input) {
            let mut pending = std::collections::VecDeque::from([input]);
            while let Some(input) = pending.pop_front() {
                for effect in self.state.apply(input, self.now) {
                    match effect {
                        Effect::Run(effect::Job::Video(Job::Url { track, ticket, .. })) => {
                            self.urls.push((track, ticket));
                        }
                        Effect::Run(effect::Job::Video(Job::Prepare { id, .. })) => self.prepares.push(id),
                        Effect::Run(effect::Job::Playback(PlaybackJob::Load {
                            track_id,
                            generation,
                            ..
                        })) => self.loads.push((track_id, generation)),
                        Effect::Audio(op) => {
                            if matches!(op, AudioOp::Play { .. }) {
                                pending.push_back(Input::Audio(AudioFact::Started(Ok(()))));
                            }
                        }
                        Effect::Timer { key: TimerKey::Video(Timer::Upgrade), after } => {
                            assert_eq!(after, UPGRADE_WAIT);
                            self.upgrade = true;
                            self.arms += 1;
                        }
                        Effect::CancelTimer(TimerKey::Video(Timer::Upgrade)) => self.upgrade = false,
                        _ => {}
                    }
                }
            }
            if let Some(next) = self.state.publish(&self.snapshot) {
                self.snapshot = next;
            }
        }

        fn video(&mut self, command: VideoCmd) {
            self.feed(Input::Command(Command::Video(command)));
        }

        fn playback(&mut self, command: PlaybackCmd) {
            self.feed(Input::Command(Command::Playback(command)));
        }

        /// I brani in coda, il primo suona.
        fn play(&mut self, tracks: Vec<Track>) {
            self.playback(PlaybackCmd::Play { tracks: tracks.into(), start: 0 });
            self.load_arrives();
        }

        fn next(&mut self) {
            self.playback(PlaybackCmd::Next);
            self.load_arrives();
        }

        fn load_arrives(&mut self) {
            let (_, generation) = self.loads.pop().expect("un caricamento chiesto");
            self.loads.clear();
            self.feed(Input::Done(AnyDone::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })));
        }

        /// Il server risponde alla richiesta numero `ticket`.
        fn answer(&mut self, ticket: u64, result: Result<SignedUrl, AppError>) {
            self.feed(Input::Done(AnyDone::Video(Done::Url { ticket, result })));
        }

        /// L'ultima richiesta in volo riceve la risposta.
        fn answer_last(&mut self, result: Result<SignedUrl, AppError>) {
            let (_, ticket) = *self.urls.last().expect("una richiesta");
            self.answer(ticket, result);
        }

        /// Scade il timer dell'aggiornamento, armato da `UPGRADE_WAIT`.
        fn fire_upgrade(&mut self) {
            assert!(self.upgrade, "il timer deve essere armato");
            self.upgrade = false;
            self.now += UPGRADE_WAIT;
            self.feed(Input::Timer(TimerKey::Video(Timer::Upgrade)));
        }

        fn view(&self) -> Arc<VideoView> {
            Arc::clone(&self.snapshot.video)
        }

        fn state(&self) -> ClipState {
            self.view().state.clone()
        }

        fn clip(&self) -> Clip {
            match self.state() {
                ClipState::Ready(clip) => clip,
                other => panic!("un videoclip pronto, non {other:?}"),
            }
        }

        fn published(&self) -> u64 {
            self.snapshot.versions.video
        }

        /// Un brano con il videoclip suona e il video è mostrato; l'indirizzo a 360p è arrivato.
        fn showing_360() -> Self {
            let mut rig = Self::new();
            rig.play(vec![clip_track(1), clip_track(2)]);
            rig.video(VideoCmd::Show(true));
            rig.answer_last(Ok(signed(Some(360))));
            rig
        }
    }

    // ---------- Quale videoclip ----------

    #[test]
    fn nothing_is_asked_while_the_video_is_hidden() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.next();
        assert!(rig.urls.is_empty() && rig.prepares.is_empty());
        assert_eq!(rig.view().state, ClipState::Idle);
        assert_eq!((rig.published(), rig.arms), (0, 0));
    }

    #[test]
    fn showing_the_video_asks_for_the_clip_of_the_current_track() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        assert_eq!(rig.urls, [(id(1), 1)]);
        let view = rig.view();
        assert_eq!((view.track, &view.state), (Some(id(1)), &ClipState::Loading));
        assert!(!rig.upgrade, "ancora niente da aggiornare");
        // Mostrarlo di nuovo non rifà la richiesta.
        rig.video(VideoCmd::Show(true));
        assert_eq!(rig.urls.len(), 1);
    }

    #[test]
    fn a_track_without_a_clip_asks_for_nothing() {
        let mut rig = Rig::new();
        rig.play(vec![track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        assert!(rig.urls.is_empty());
        assert_eq!((rig.published(), rig.view().state.clone()), (0, ClipState::Idle));
        // Il brano dopo ne ha uno: la richiesta parte da sé, e quando il brano senza video torna, il
        // videoclip se ne va.
        rig.next();
        assert_eq!(rig.urls, [(id(2), 1)]);
        rig.playback(PlaybackCmd::Previous);
        rig.load_arrives();
        assert_eq!(rig.view().state, ClipState::Idle);
        assert!(rig.view().track.is_none());
    }

    #[test]
    fn the_clip_arrives_and_is_published_with_an_address_to_open() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        rig.answer_last(Ok(signed(Some(360))));
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height, clip.is_full()), (1, Some(360), false));
        assert_eq!(clip.url(), "https://media.test/v1/media?t=firmato");
        assert_eq!(rig.view().track, Some(id(1)));
        // L'indirizzo è firmato: nei log non compare.
        let debug = format!("{:?}", rig.view());
        assert!(!debug.contains("media.test") && !debug.contains("firmato"), "{debug}");
    }

    #[test]
    fn another_track_takes_the_clip_and_an_old_answer_is_dropped() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        rig.next();
        assert_eq!(rig.urls, [(id(1), 1), (id(2), 2)]);
        // La risposta del primo brano arriva tardi: non vale più.
        rig.answer(1, Ok(signed(Some(360))));
        assert_eq!(rig.view().state, ClipState::Loading);
        rig.answer(2, Ok(signed(Some(720))));
        assert_eq!((rig.view().track, rig.clip().seq), (Some(id(2)), 1));
    }

    #[test]
    fn hiding_the_video_drops_the_clip_and_the_timer() {
        let mut rig = Rig::showing_360();
        assert!(rig.upgrade);
        rig.video(VideoCmd::Show(false));
        assert!(!rig.upgrade, "il timer si annulla");
        assert_eq!((rig.view().state.clone(), rig.view().track), (ClipState::Idle, None));
        // Una risposta in volo non lo riporta indietro.
        rig.video(VideoCmd::Show(true));
        rig.video(VideoCmd::Show(false));
        let before = rig.published();
        rig.answer_last(Ok(signed(Some(360))));
        assert_eq!((rig.state(), rig.published()), (ClipState::Idle, before));
        assert!(!rig.upgrade);
    }

    // ---------- Dal 360p al 720p ----------

    #[test]
    fn the_full_version_is_asked_every_six_seconds_up_to_ten_times() {
        let mut rig = Rig::showing_360();
        assert_eq!((rig.urls.len(), rig.arms), (1, 1));
        let published = rig.published();
        // Il server non ha ancora il 720p: dieci domande, e poi basta.
        for asked in 1..=MAX_UPGRADES as usize {
            rig.fire_upgrade();
            assert_eq!(rig.urls.len(), 1 + asked);
            rig.answer_last(Ok(signed(Some(360))));
            assert_eq!(rig.upgrade, asked < MAX_UPGRADES as usize, "dopo {asked} domande");
        }
        assert_eq!(rig.urls.len(), 11);
        assert_eq!(rig.arms, 10, "all'apertura e dopo ogni risposta che non basta, tranne l'ultima");
        assert_eq!(rig.clip().seq, 1, "il videoclip è sempre lo stesso");
        assert_eq!(rig.published(), published, "nessuna risposta inutile si pubblica");
    }

    #[test]
    fn a_better_version_is_published_once_with_a_new_sequence_and_stops_the_timer() {
        let mut rig = Rig::showing_360();
        let published = rig.published();
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height, clip.is_full()), (2, Some(720), true));
        assert_eq!(rig.published(), published + 1, "una pubblicazione sola");
        assert!(!rig.upgrade, "la versione completa non aspetta altro");
        assert_eq!(rig.urls.len(), 2);
    }

    #[test]
    fn a_failed_upgrade_request_is_asked_again_and_a_clip_without_height_is_full() {
        let mut rig = Rig::showing_360();
        let published = rig.published();
        rig.fire_upgrade();
        rig.answer_last(Err(AppError::Network));
        assert!(rig.upgrade, "la rete è caduta: si riprova al prossimo giro");
        assert_eq!((rig.state(), rig.published()), (ClipState::Ready(rig.clip()), published));
        rig.fire_upgrade();
        // Senza altezza è un videoclip salvato prima delle due versioni: è completo.
        rig.answer_last(Ok(signed(None)));
        assert_eq!((rig.clip().seq, rig.clip().is_full()), (2, true));
        assert!(!rig.upgrade);
    }

    #[test]
    fn a_complete_clip_from_the_start_arms_nothing() {
        for height in [Some(720), Some(1080), None] {
            let mut rig = Rig::new();
            rig.play(vec![clip_track(1)]);
            rig.video(VideoCmd::Show(true));
            rig.answer_last(Ok(signed(height)));
            assert_eq!(rig.arms, 0, "{height:?}");
            assert!(rig.clip().is_full());
        }
    }

    #[test]
    fn leaving_the_track_while_the_upgrade_is_asked_drops_the_answer() {
        let mut rig = Rig::showing_360();
        rig.fire_upgrade();
        let (_, upgrade_ticket) = *rig.urls.last().unwrap();
        rig.next();
        assert!(!rig.upgrade, "il timer del brano di prima non c'è più");
        rig.answer(upgrade_ticket, Ok(signed(Some(720))));
        assert_eq!((rig.view().track, rig.view().state.clone()), (Some(id(2)), ClipState::Loading));
    }

    /// Prestazioni (passo 4.9): con il video nascosto il core non arma niente e non pubblica niente,
    /// anche con i comandi che non cambiano niente; un videoclip pubblica solo la versione del suo
    /// dominio, e una versione migliore una volta sola.
    #[test]
    fn only_the_video_version_moves_and_an_idle_core_moves_nothing() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        let idle = rig.snapshot.versions;
        for command in [
            VideoCmd::Show(false),
            VideoCmd::Retry,
            VideoCmd::Reopen,
            VideoCmd::Failed { seq: 1 },
            VideoCmd::PrepareAhead(true),
            VideoCmd::PrepareAhead(false),
        ] {
            rig.video(command);
        }
        assert_eq!(rig.snapshot.versions, idle, "nessuna pubblicazione");
        assert_eq!((rig.arms, rig.urls.len()), (0, 0), "niente armato, niente chiesto");

        rig.video(VideoCmd::Show(true));
        let loading = rig.snapshot.versions;
        assert_eq!(loading, DomainVersions { video: idle.video + 1, ..idle });
        rig.answer_last(Ok(signed(Some(360))));
        let ready = rig.snapshot.versions;
        assert_eq!(ready, DomainVersions { video: loading.video + 1, ..idle });
        // Un'attesa che non porta niente di nuovo non pubblica; il 720p sì, una volta, e da solo.
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(360))));
        assert_eq!(rig.snapshot.versions, ready);
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.snapshot.versions, DomainVersions { video: ready.video + 1, ..idle });
        assert!(!rig.upgrade);
        // Nascosto di nuovo: una pubblicazione, e il core torna a riposo.
        rig.video(VideoCmd::Show(false));
        assert_eq!(rig.snapshot.versions, DomainVersions { video: ready.video + 2, ..idle });
        assert_eq!(rig.view().state, ClipState::Idle);
    }

    // ---------- Quando non si apre ----------

    #[test]
    fn a_clip_that_does_not_open_is_asked_again_once_then_blocked() {
        let mut rig = Rig::showing_360();
        let first = rig.clip().seq;
        rig.video(VideoCmd::Failed { seq: first });
        assert_eq!(rig.state(), ClipState::Loading, "un indirizzo nuovo, può essere scaduto");
        assert!(!rig.upgrade);
        assert_eq!(rig.urls.len(), 2);
        rig.answer_last(Ok(signed(Some(360))));
        let second = rig.clip().seq;
        assert_ne!(first, second);
        assert!(rig.upgrade, "il videoclip nuovo riprende l'attesa del 720p");
        // Il secondo guasto ferma il videoclip di quel brano.
        rig.video(VideoCmd::Failed { seq: second });
        assert_eq!(rig.state(), ClipState::Failed(AppError::Video));
        assert_eq!((rig.view().track, rig.view().failures), (Some(id(1)), 1));
        assert!(!rig.upgrade);
        // Mostrarlo di nuovo o nasconderlo non lo riprova.
        rig.video(VideoCmd::Show(false));
        rig.video(VideoCmd::Show(true));
        assert_eq!((rig.urls.len(), rig.state()), (2, ClipState::Failed(AppError::Video)));
    }

    #[test]
    fn a_failure_of_a_clip_that_is_not_the_published_one_is_ignored() {
        let mut rig = Rig::showing_360();
        let before = (rig.published(), rig.urls.len());
        rig.video(VideoCmd::Failed { seq: 99 });
        assert_eq!((rig.published(), rig.urls.len()), before);
        // E senza videoclip non c'è niente da guastare.
        let mut idle = Rig::new();
        idle.video(VideoCmd::Failed { seq: 1 });
        assert_eq!(idle.published(), 0);
    }

    #[test]
    fn a_blocked_clip_is_tried_again_by_another_track_or_by_retry() {
        let mut rig = Rig::showing_360();
        rig.video(VideoCmd::Failed { seq: 1 });
        rig.answer_last(Ok(signed(Some(360))));
        rig.video(VideoCmd::Failed { seq: 2 });
        assert!(matches!(rig.state(), ClipState::Failed(_)));
        // «Riprova»: si chiede da capo, con un'altra possibilità di riprovare.
        rig.video(VideoCmd::Retry);
        assert_eq!(rig.state(), ClipState::Loading);
        assert_eq!(rig.urls.len(), 3);
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 3);
        rig.video(VideoCmd::Failed { seq: 3 });
        assert_eq!(rig.state(), ClipState::Loading, "il riprovare è di nuovo da fare");
        rig.answer_last(Ok(signed(Some(720))));
        rig.video(VideoCmd::Failed { seq: 4 });
        assert!(matches!(rig.state(), ClipState::Failed(_)));
        // Un altro brano ha il suo videoclip.
        rig.next();
        assert_eq!((rig.view().track, rig.state()), (Some(id(2)), ClipState::Loading));
        assert_eq!(rig.view().failures, 2, "i guasti già contati restano contati");
    }

    #[test]
    fn retry_on_a_working_clip_changes_nothing() {
        let mut rig = Rig::showing_360();
        let before = (rig.published(), rig.urls.len(), rig.arms);
        rig.video(VideoCmd::Retry);
        assert_eq!((rig.published(), rig.urls.len(), rig.arms), before);
    }

    #[test]
    fn the_server_refusing_the_clip_blocks_it_with_its_error() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        let refusal = AppError::Api { status: 404, code: "not_found".into(), message: "Non trovato.".into() };
        rig.answer_last(Err(refusal.clone()));
        assert_eq!(rig.state(), ClipState::Failed(refusal));
        assert_eq!(rig.view().failures, 1);
        assert_eq!(rig.urls.len(), 1, "un rifiuto del server non si ripete da solo");
        // Ma «Riprova» lo chiede da capo.
        rig.video(VideoCmd::Retry);
        assert_eq!(rig.state(), ClipState::Loading);
    }

    #[test]
    fn an_upgrade_that_does_not_open_leaves_the_old_clip_and_asks_again_later() {
        let mut rig = Rig::showing_360();
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 2);
        // Il frontend non apre il 720p: continua con il 360p che suonava.
        rig.video(VideoCmd::Failed { seq: 2 });
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height), (1, Some(360)));
        assert!(rig.upgrade, "e il 720p si richiede più tardi");
        assert_eq!(rig.view().failures, 0, "un aggiornamento mancato non è un guasto");
        // Il tentativo dopo ritrova il 720p con un indirizzo nuovo.
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 3);
        assert!(!rig.upgrade);
    }

    #[test]
    fn reopen_asks_for_the_clip_again_but_only_while_it_is_there() {
        let mut idle = Rig::new();
        idle.play(vec![clip_track(1)]);
        idle.video(VideoCmd::Reopen);
        assert_eq!((idle.published(), idle.urls.len()), (0, 0));

        let mut rig = Rig::showing_360();
        rig.video(VideoCmd::Reopen);
        assert_eq!(rig.state(), ClipState::Loading);
        assert_eq!(rig.urls.len(), 2);
        assert!(!rig.upgrade);
        rig.answer_last(Ok(signed(Some(360))));
        assert!(rig.upgrade);
    }

    #[test]
    fn a_lost_job_is_a_network_failure_not_a_hang() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        let (_, ticket) = rig.urls[0];
        let lost = Job::Url { api: Err(AppError::Network), track: id(1), ticket }.lost();
        rig.feed(Input::Done(AnyDone::Video(lost)));
        assert_eq!(rig.state(), ClipState::Failed(AppError::Network));
        assert!(matches!(Job::Prepare { api: Err(AppError::Network), id: id(1) }.lost(), Done::Prepared));
    }

    // ---------- In anticipo ----------

    fn queue_of_clips() -> Vec<Track> {
        // Il 3 ha il videoclip già pronto, il 5 non ne ha: non si preparano.
        vec![
            clip_track(1),
            clip_track(2),
            Track { video_ready: true, ..clip_track(3) },
            clip_track(4),
            track(5),
            clip_track(6),
        ]
    }

    #[test]
    fn the_server_prepares_the_clips_of_the_track_and_the_next_two_once() {
        let mut rig = Rig::new();
        rig.video(VideoCmd::PrepareAhead(true));
        rig.play(queue_of_clips());
        // Il brano corrente e i due dopo: il 3 è già pronto, quindi 1 e 2.
        assert_eq!(std::mem::take(&mut rig.prepares), [id(1), id(2)]);
        rig.next();
        // Corrente 2 (già chiesto), poi 3 (pronto) e 4.
        assert_eq!(std::mem::take(&mut rig.prepares), [id(4)]);
        rig.next();
        assert!(rig.prepares.is_empty(), "3 pronto, 4 già chiesto, 5 senza video");
        rig.next();
        rig.next();
        assert_eq!(std::mem::take(&mut rig.prepares), [id(6)]);
        assert!(rig.urls.is_empty(), "preparare non è mostrare");
        assert_eq!(rig.published(), 0);
    }

    #[test]
    fn nothing_is_prepared_unless_the_listener_wants_clips() {
        let mut rig = Rig::new();
        rig.play(queue_of_clips());
        rig.next();
        assert!(rig.prepares.is_empty());
        // Acceso a brano già partito: si prepara subito quello in corso e i prossimi.
        rig.video(VideoCmd::PrepareAhead(true));
        assert_eq!(std::mem::take(&mut rig.prepares), [id(2), id(4)]);
        // Rimandato lo stesso: non cambia niente.
        rig.video(VideoCmd::PrepareAhead(true));
        assert!(rig.prepares.is_empty());
        // Spento, un altro brano che parte non prepara niente; riacceso, riprende dal resto.
        rig.video(VideoCmd::PrepareAhead(false));
        rig.next();
        rig.next();
        assert!(rig.prepares.is_empty());
        rig.video(VideoCmd::PrepareAhead(true));
        assert_eq!(std::mem::take(&mut rig.prepares), [id(6)]);
    }

    #[test]
    fn the_restored_queue_is_not_prepared_until_something_plays() {
        let mut rig = Rig::new();
        // La coda ripresa dall'ultima sessione c'è, ma niente è partito: all'avvio il server non
        // scarica tre videoclip che nessuno ha chiesto.
        rig.playback(PlaybackCmd::Enqueue(Box::new(clip_track(1))));
        rig.playback(PlaybackCmd::Enqueue(Box::new(clip_track(2))));
        rig.video(VideoCmd::PrepareAhead(true));
        assert!(rig.prepares.is_empty());
        assert_eq!(rig.state.playback.view.started, 0);
    }

    #[test]
    fn the_prepared_tracks_are_a_bounded_list_that_forgets_the_oldest() {
        let mut video = Video::default();
        assert!(video.remember(id(1)));
        assert!(!video.remember(id(1)), "già chiesto");
        for n in 2..=PREPARED as u128 {
            assert!(video.remember(id(n)));
        }
        assert_eq!(video.prepared.len(), PREPARED);
        // Uno in più: esce il meno recente (l'1, rimesso in coda prima che arrivassero gli altri).
        assert!(video.remember(id(1000)));
        assert_eq!(video.prepared.len(), PREPARED);
        assert!(video.remember(id(1)), "il meno recente è stato dimenticato e si richiede");
        assert!(!video.remember(id(1000)));
        // Rivedere un brano lo rimette fra i più recenti: il 3 resta, il 2 (ormai il più vecchio) esce.
        assert!(!video.remember(id(3)));
        assert!(video.remember(id(2000)));
        assert!(video.remember(id(2)));
        assert!(!video.remember(id(3)));
    }

    // ---------- Allineamento all'audio ----------

    fn view_at(position: Duration, playing: bool, at: Instant) -> PlaybackView {
        let (state, clock) = if playing {
            (PlaybackState::Playing, Clock::running(position, at))
        } else {
            (PlaybackState::Paused, Clock::paused(position, at))
        };
        PlaybackView { state, clock, ..PlaybackView::default() }
    }

    fn measure(position: Duration) -> Measure {
        Measure { position, duration: secs(200), paused: false, ended: false }
    }

    #[test]
    fn a_video_on_the_audio_is_left_alone() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let later = start + secs(3);
        let audio = view_at(secs(10), true, start);
        // Audio a 13 s: il video a 13,15 s è dentro la soglia.
        let adjust = alignment.align(&audio, measure(secs(13) + Duration::from_millis(150)), false, later);
        assert!(adjust.is_none());
        assert_eq!(alignment.lead(), Duration::ZERO);
    }

    #[test]
    fn a_video_behind_the_audio_is_moved_and_aims_further_ahead_each_time() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(10), true, start);
        // Dopo 3 s l'audio è a 13 s e il video a 12: indietro di 1 s. Si mira 1 s avanti.
        let at = start + secs(3);
        let adjust = alignment.align(&audio, measure(secs(12)), false, at);
        assert_eq!(adjust, Adjust { pause: None, seek: Some(secs(14)) });
        assert_eq!(alignment.lead(), secs(1));
        // Troppo presto: il video non è ancora ripartito, non lo si insegue.
        let adjust = alignment.align(&audio, measure(secs(12)), false, at + Duration::from_millis(1900));
        assert!(adjust.is_none());
        // Dopo due secondi (audio a 15 s) è ancora indietro di 0,5 s: l'anticipo cresce.
        let at = at + RESYNC_PAUSE;
        let adjust = alignment.align(&audio, measure(secs(14) + Duration::from_millis(500)), false, at);
        assert_eq!(alignment.lead(), Duration::from_millis(1500));
        assert_eq!(adjust.seek, Some(secs(15) + Duration::from_millis(1500)));
    }

    #[test]
    fn the_lead_never_grows_past_the_cap() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(100), true, start);
        let mut at = start;
        for _ in 0..5 {
            at += secs(3);
            alignment.align(&audio, measure(secs(10)), false, at);
        }
        assert_eq!(alignment.lead(), MAX_LEAD);
    }

    #[test]
    fn a_video_ahead_of_the_audio_shortens_the_lead() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(10), true, start);
        let at = start + secs(3);
        alignment.align(&audio, measure(secs(12)), false, at);
        assert_eq!(alignment.lead(), secs(1));
        // Audio a 15 s; il video a 15,4 s è 400 ms avanti: l'anticipo scende a 600 ms.
        let at = at + secs(2);
        let adjust = alignment.align(&audio, measure(secs(15) + Duration::from_millis(400)), false, at);
        assert_eq!(alignment.lead(), Duration::from_millis(600));
        assert_eq!(adjust.seek, Some(secs(15) + Duration::from_millis(600)));
        // E non scende sotto zero.
        let at = at + secs(2);
        alignment.align(&audio, measure(secs(17) + Duration::from_secs(3)), false, at);
        assert_eq!(alignment.lead(), Duration::ZERO);
    }

    #[test]
    fn the_beat_waits_while_paused_but_a_fresh_player_aligns_at_once() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(10));
        let paused = view_at(secs(10), false, start);
        let moving = Measure { paused: false, ..measure(secs(30)) };
        // Al battito, con l'audio in pausa, non si fa niente (la pausa la segue `follow`).
        assert!(alignment.align(&paused, moving, false, start).is_none());
        // Un player appena aperto si mette subito come l'audio: in pausa e alla stessa posizione.
        let adjust = alignment.align(&paused, moving, true, start);
        assert_eq!(adjust, Adjust { pause: Some(true), seek: Some(secs(10)) });
        // Mentre suona, un video fermo riparte.
        let playing = view_at(secs(10), true, start);
        let stopped = Measure { paused: true, ..measure(secs(10)) };
        let adjust = alignment.align(&playing, stopped, true, start);
        assert_eq!(adjust, Adjust { pause: Some(false), seek: None });
    }

    #[test]
    fn a_video_shorter_than_the_audio_stays_on_its_last_frame() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(10));
        let audio = view_at(secs(190), true, start);
        let short = Measure { duration: secs(180), ..measure(secs(179)) };
        assert!(alignment.align(&audio, short, false, start).is_none());
        // Il video arrivato in fondo non si riporta indietro, ma la pausa resta quella dell'audio.
        let ended = Measure { ended: true, paused: true, ..measure(secs(20)) };
        let playing = view_at(secs(20), true, start);
        assert_eq!(alignment.align(&playing, ended, false, start).seek, None);
    }

    #[test]
    fn the_video_follows_the_playback_changes() {
        let start = Instant::now();
        let at = start + secs(5);
        let mut alignment = Alignment::new(start);
        let base = view_at(secs(10), true, start);

        // Niente di nuovo: niente da fare.
        assert_eq!(alignment.follow(&base, &base.clone(), at), None);

        // Un brano che parte: il video va alla posizione dell'audio (15 s a `at`).
        let started = PlaybackView { started: 1, ..base.clone() };
        assert_eq!(
            alignment.follow(&base, &started, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
        // Un salto nel brano, da fermo o no, porta il video lì.
        let sought = PlaybackView { seeks: 1, ..started.clone() };
        assert_eq!(
            alignment.follow(&started, &sought, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
        let sought_paused = PlaybackView { seeks: 2, ..view_at(secs(40), false, at) };
        assert_eq!(
            alignment.follow(&sought, &sought_paused, at),
            Some(Adjust { pause: Some(true), seek: Some(secs(40)) })
        );
        // Una pausa ferma il video e basta; la ripresa lo rimette dov'è l'audio.
        let paused = PlaybackView { started: 1, ..view_at(secs(15), false, at) };
        assert_eq!(alignment.follow(&started, &paused, at), Some(Adjust { pause: Some(true), seek: None }));
        let resumed = PlaybackView { started: 1, ..view_at(secs(15), true, at) };
        assert_eq!(
            alignment.follow(&paused, &resumed, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
    }

    #[test]
    fn a_move_starts_the_wait_before_the_next_chase() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(60));
        let base = view_at(secs(10), true, start);
        let started = PlaybackView { started: 1, ..base.clone() };
        alignment.follow(&base, &started, start);
        // Appena spostato: il video è ancora indietro, ma si aspetta che riparta.
        let behind = measure(secs(5));
        assert!(alignment.align(&started, behind, false, start + Duration::from_millis(500)).is_none());
        assert!(alignment.align(&started, behind, false, start + RESYNC_PAUSE).seek.is_some());
    }
}
