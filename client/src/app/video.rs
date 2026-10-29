//! Il videoclip del brano in riproduzione: quando aprirlo, come tenerlo allineato all'audio e come
//! passare alla versione migliore. Il server dà subito una versione a 360p e poco dopo quella a
//! 720p; il client apre la prima, poi chiede di nuovo l'URL finché non arriva la seconda e la
//! sostituisce senza fermarsi. GStreamer apre il video su un thread a parte (può bloccare fino a
//! 5 secondi): la UI non si ferma mai.

use std::time::Duration;
#[cfg(feature = "video")]
use std::time::Instant;

use iced::Task;
use uuid::Uuid;

#[cfg(feature = "video")]
use super::Notice;
use super::{App, Message, SideTab};
#[cfg(feature = "video")]
use crate::api::ApiClient;
#[cfg(feature = "video")]
use crate::error::AppError;

/// Brani della coda, dopo quello corrente, di cui il server prepara il videoclip.
const VIDEO_AHEAD: usize = 2;
/// Altezza della versione completa: sotto, si aspetta che il server la prepari.
#[cfg(feature = "video")]
const FULL_HEIGHT: u32 = 720;
/// Ogni quanto si chiede se la versione completa è pronta, e quante volte al massimo.
#[cfg(feature = "video")]
const UPGRADE_WAIT: Duration = Duration::from_secs(6);
#[cfg(feature = "video")]
const MAX_UPGRADES: u32 = 10;
/// Scarto tollerato fra video e audio prima di riallineare il video.
#[cfg(feature = "video")]
const VIDEO_DRIFT: Duration = Duration::from_millis(200);
/// Dopo un riallineamento si lascia al video il tempo di ripartire.
#[cfg(feature = "video")]
const RESYNC_PAUSE: Duration = Duration::from_secs(2);
/// Massimo anticipo con cui si mira un riallineamento (vedi `VideoView::lead`).
#[cfg(feature = "video")]
const MAX_LEAD: Duration = Duration::from_millis(1500);
/// Quanto si aspetta, sul thread che apre il video, che il seek iniziale arrivi a destinazione.
#[cfg(feature = "video")]
const SETTLE: Duration = Duration::from_millis(600);

/// Videoclip aperto: il player GStreamer e il brano a cui appartiene.
#[cfg(feature = "video")]
pub struct VideoView {
    pub track_id: Uuid,
    pub player: Option<iced_video_player::Video>,
    /// Altezza della versione aperta (`None`: completa, o non ancora aperta).
    pub height: Option<u32>,
    /// Un URL scaduto o uno stream caduto si riprovano una volta sola con un URL nuovo.
    pub retried: bool,
    /// Richieste della versione completa già fatte.
    upgrades: u32,
    /// Ultimo riallineamento all'audio: non lo si insegue a ogni Tick.
    synced_at: Instant,
    /// Il seek stesso costa tempo (il video si ferma a bufferizzare mentre l'audio va avanti): si
    /// mira avanti di quanto il video è rimasto indietro le volte prima, o si insegue all'infinito.
    lead: Duration,
}

#[cfg(feature = "video")]
impl VideoView {
    fn new(track_id: Uuid, retried: bool) -> Self {
        Self {
            track_id,
            player: None,
            height: None,
            retried,
            upgrades: 0,
            synced_at: Instant::now(),
            lead: Duration::ZERO,
        }
    }

    fn wants_upgrade(&self) -> bool {
        self.player.is_some() && self.height.is_some_and(|h| h < FULL_HEIGHT) && self.upgrades < MAX_UPGRADES
    }
}

/// Un player aperto su un altro thread. Viaggia in un messaggio, che deve essere clonabile: chi lo
/// riceve lo prende una volta sola.
#[cfg(feature = "video")]
#[derive(Clone)]
pub struct OpenedVideo {
    player: std::sync::Arc<std::sync::Mutex<Option<iced_video_player::Video>>>,
    height: Option<u32>,
}

#[cfg(feature = "video")]
impl std::fmt::Debug for OpenedVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedVideo").field("height", &self.height).finish_non_exhaustive()
    }
}

#[cfg(feature = "video")]
impl OpenedVideo {
    fn take(&self) -> Option<iced_video_player::Video> {
        self.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

/// La posizione dell'audio in un istante: il thread che apre il video la proietta in avanti.
#[cfg(feature = "video")]
#[derive(Debug, Clone, Copy)]
struct Clock {
    position: Duration,
    at: Instant,
    playing: bool,
}

#[cfg(feature = "video")]
impl Clock {
    fn now(self) -> Duration {
        if self.playing { self.position + self.at.elapsed() } else { self.position }
    }
}

/// Plugin dei decoder NVIDIA (NVDEC via CUDA).
#[cfg(feature = "video")]
const NVIDIA_DECODERS: &str = "nvcodec";
/// Rango dei decoder NVIDIA scelti dall'utente: sopra quelli della GPU integrata (VA-API e D3D11
/// hanno `PRIMARY + 1`).
#[cfg(feature = "video")]
const DEDICATED_RANK: i32 = 258;

/// Sceglie chi decodifica i videoclip, prima di ogni player. Sui portatili ibridi GStreamer sceglie
/// NVDEC a pari rango con il decoder della GPU integrata: sveglia la GPU dedicata e apre un
/// contesto CUDA (~130 MB di VRAM, ~60 MB di RAM) per un videoclip a 360p o 720p. Di norma i
/// decoder NVIDIA hanno rango zero e il plugin non si carica (decodifica la GPU integrata, o la
/// CPU); con `dedicated` passano davanti. Bloccante: la prima volta legge il registro di GStreamer.
#[cfg(feature = "video")]
fn choose_decoders(dedicated: bool) {
    use gstreamer::prelude::PluginFeatureExtManual;
    if gstreamer::init().is_err() {
        return;
    }
    let rank = if dedicated { gstreamer::Rank::from(DEDICATED_RANK) } else { gstreamer::Rank::NONE };
    for feature in gstreamer::Registry::get().features_by_plugin(NVIDIA_DECODERS) {
        feature.set_rank(rank);
    }
}

/// La scheda video dedicata che può decodificare i videoclip, col suo nome: c'è se GStreamer ha i
/// decoder NVIDIA (il plugin li registra solo con un driver e una GPU funzionanti). Intanto applica
/// la scelta salvata. Bloccante.
#[cfg(feature = "video")]
pub fn detect_dedicated_gpu(dedicated: bool) -> Option<String> {
    use gstreamer::prelude::GstObjectExt;
    choose_decoders(dedicated);
    let has_decoder = gstreamer::Registry::get()
        .features_by_plugin(NVIDIA_DECODERS)
        .iter()
        .any(|feature| feature.name().ends_with("dec"));
    has_decoder.then(nvidia_model)
}

#[cfg(not(feature = "video"))]
pub fn detect_dedicated_gpu(_dedicated: bool) -> Option<String> {
    None
}

/// Il modello della GPU NVIDIA, dal driver su Linux («NVIDIA GeForce RTX 5060 Laptop GPU»).
#[cfg(feature = "video")]
fn nvidia_model() -> String {
    std::fs::read_dir("/proc/driver/nvidia/gpus")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|gpu| std::fs::read_to_string(gpu.path().join("information")).ok())
        .find_map(|info| model_line(&info))
        .unwrap_or_else(|| "Scheda video NVIDIA".to_owned())
}

#[cfg(feature = "video")]
fn model_line(info: &str) -> Option<String> {
    info.lines()
        .find_map(|line| line.strip_prefix("Model:"))
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

/// Chiude un player su un thread a parte: fermare la pipeline può aspettare la rete.
#[cfg(feature = "video")]
fn discard(player: iced_video_player::Video) {
    let _ = std::thread::Builder::new().name("video-close".into()).spawn(move || drop(player));
}

/// Chiede l'URL del videoclip e lo apre già alla posizione dell'audio. Con `better_than`, se il
/// server non ha ancora una versione più alta, `None` senza aprire nulla.
#[cfg(feature = "video")]
async fn open(
    api: Result<ApiClient, AppError>,
    track_id: Uuid,
    clock: Clock,
    better_than: Option<u32>,
    dedicated: bool,
) -> Result<Option<OpenedVideo>, AppError> {
    #[cfg(feature = "tour")]
    let asked = Instant::now();
    let signed = api?.video_url(track_id).await?;
    #[cfg(feature = "tour")]
    eprintln!("tour: video-url {:?} height {:?}", asked.elapsed(), signed.height);
    // Senza altezza è un videoclip salvato prima delle due versioni: è completo.
    if let Some(current) = better_than
        && signed.height.is_some_and(|height| height <= current)
    {
        return Ok(None);
    }
    let height = signed.height;
    let url = url::Url::parse(&signed.url).map_err(|_| AppError::Video)?;
    let player = tokio::task::spawn_blocking(move || {
        // L'errore di GStreamer può contenere l'URL firmato: non lo si conserva.
        choose_decoders(dedicated);
        let mut player = iced_video_player::Video::new(&url).map_err(|_| AppError::Video)?;
        player.set_paused(!clock.playing);
        let target = clock.now();
        if player.seek(target).is_ok() {
            let deadline = Instant::now() + SETTLE;
            while Instant::now() < deadline && player.position().abs_diff(clock.now()) > VIDEO_DRIFT {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(feature = "tour")]
        eprintln!("tour: video open+seek {:?} at {:?}", asked.elapsed(), player.position());
        Ok::<_, AppError>(player)
    })
    .await
    .map_err(|_| AppError::Video)??;
    Ok(Some(OpenedVideo { player: std::sync::Arc::new(std::sync::Mutex::new(Some(player))), height }))
}

impl App {
    /// Il brano corrente ha un videoclip e il client sa riprodurlo.
    pub fn video_available(&self) -> bool {
        cfg!(feature = "video") && self.queue.current().is_some_and(|t| t.has_video)
    }

    /// Il videoclip del brano corrente si sta preparando: il server lo scarica (la prima volta),
    /// poi GStreamer lo apre.
    pub fn video_loading(&self) -> bool {
        self.wanted_video().is_some() && self.video_player_missing()
    }

    /// Il videoclip del brano corrente non si è aperto: si mostra la copertina con «Riprova».
    pub fn video_broken(&self) -> bool {
        self.video_available() && self.queue.current().is_some_and(|t| self.video_blocked == Some(t.id))
    }

    #[cfg(feature = "video")]
    fn video_player_missing(&self) -> bool {
        self.video.as_ref().is_none_or(|v| v.player.is_none())
    }

    #[cfg(not(feature = "video"))]
    fn video_player_missing(&self) -> bool {
        false
    }

    /// Il brano di cui qualcuno vuole il videoclip: la vista a tutto schermo, oppure il pannello
    /// «In riproduzione» aperto con il videoclip scelto al posto della copertina.
    pub(super) fn wanted_video(&self) -> Option<Uuid> {
        let track = self.queue.current().filter(|_| self.video_available())?;
        if self.video_blocked == Some(track.id) {
            return None;
        }
        let in_panel = self.panel_open && self.side_tab == SideTab::NowPlaying && self.panel_video;
        let on_stage = self.focus && self.focus_video;
        (in_panel || on_stage).then_some(track.id)
    }

    /// Porta il videoclip caricato a ciò che si vuole vedere: lo apre, o lo ferma e libera la memoria.
    pub(super) fn refresh_video(&mut self) -> Task<Message> {
        match self.wanted_video() {
            Some(_) => self.load_video(false),
            None => {
                self.close_video();
                Task::none()
            }
        }
    }

    /// Il server prepara i videoclip del brano corrente e dei prossimi in coda: quando si guardano
    /// sono già nello storage. Solo se il pannello mostra i videoclip; ogni brano una volta.
    pub(super) fn prepare_videos(&mut self) -> Task<Message> {
        if !cfg!(feature = "video") || !self.panel_video {
            return Task::none();
        }
        let ahead =
            self.queue.current().into_iter().chain(self.queue.upcoming().take(VIDEO_AHEAD).map(|(_, t)| t));
        let ids: Vec<Uuid> = ahead
            .filter(|track| track.has_video && !track.video_ready)
            .map(|track| track.id)
            .filter(|id| self.videos_prepared.insert(*id))
            .collect();
        Task::batch(ids.into_iter().map(|id| {
            let api = self.api();
            Task::perform(async move { api?.prepare_video(id).await }, |_| ()).discard()
        }))
    }

    #[cfg(feature = "video")]
    fn close_video(&mut self) {
        if let Some(player) = self.video.take().and_then(|view| view.player) {
            discard(player);
        }
    }

    #[cfg(not(feature = "video"))]
    fn close_video(&mut self) {}

    #[cfg(feature = "video")]
    fn clock(&self) -> Clock {
        Clock { position: self.audio_position(), at: Instant::now(), playing: self.playback.is_playing() }
    }

    #[cfg(feature = "video")]
    pub(super) fn load_video(&mut self, retried: bool) -> Task<Message> {
        let Some(track_id) = self.queue.current().filter(|t| t.has_video).map(|t| t.id) else {
            self.close_video();
            return Task::none();
        };
        // Lo stesso videoclip già aperto (o in apertura) resta com'è.
        if !retried && self.video.as_ref().is_some_and(|view| view.track_id == track_id) {
            return Task::none();
        }
        self.close_video();
        self.video = Some(VideoView::new(track_id, retried));
        let (api, clock) = (self.api(), self.clock());
        let dedicated = self.dedicated_video;
        Task::perform(open(api, track_id, clock, None, dedicated), move |result| {
            Message::VideoOpened(track_id, result)
        })
    }

    #[cfg(not(feature = "video"))]
    pub(super) fn load_video(&mut self, _retried: bool) -> Task<Message> {
        Task::none()
    }

    #[cfg(feature = "video")]
    pub(super) fn video_opened(
        &mut self,
        track_id: Uuid,
        result: Result<Option<OpenedVideo>, AppError>,
    ) -> Task<Message> {
        let opened = result.and_then(|opened| opened.ok_or(AppError::Video));
        if self.video.as_ref().is_none_or(|view| view.track_id != track_id || view.player.is_some()) {
            // Arrivato tardi: il brano è cambiato o il videoclip è stato chiuso.
            if let Some(player) = opened.ok().and_then(|opened| opened.take()) {
                discard(player);
            }
            return Task::none();
        }
        let player = opened.and_then(|opened| Ok((opened.take().ok_or(AppError::Video)?, opened.height)));
        match player {
            Ok((player, height)) => {
                if let Some(view) = self.video.as_mut() {
                    view.player = Some(player);
                    view.height = height;
                }
                self.align_video(true);
                self.schedule_upgrade(track_id)
            }
            Err(err) => {
                self.close_video();
                self.video_blocked = Some(track_id);
                self.notice = Some(Notice::Error(err.to_string()));
                Task::none()
            }
        }
    }

    /// Fra un po' si chiede se la versione completa è pronta.
    #[cfg(feature = "video")]
    fn schedule_upgrade(&self, track_id: Uuid) -> Task<Message> {
        if !self.video.as_ref().is_some_and(VideoView::wants_upgrade) {
            return Task::none();
        }
        Task::perform(tokio::time::sleep(UPGRADE_WAIT), move |()| Message::VideoUpgrade(track_id))
    }

    #[cfg(feature = "video")]
    pub(super) fn video_upgrade(&mut self, track_id: Uuid) -> Task<Message> {
        let clock = self.clock();
        let Some(view) = self.video.as_mut().filter(|view| view.track_id == track_id && view.wants_upgrade())
        else {
            return Task::none();
        };
        view.upgrades += 1;
        let better_than = view.height;
        let (api, dedicated) = (self.api(), self.dedicated_video);
        Task::perform(open(api, track_id, clock, better_than, dedicated), move |result| {
            Message::VideoUpgraded(track_id, result)
        })
    }

    /// La versione completa, se è arrivata, prende il posto di quella aperta senza fermarsi.
    #[cfg(feature = "video")]
    pub(super) fn video_upgraded(
        &mut self,
        track_id: Uuid,
        result: Result<Option<OpenedVideo>, AppError>,
    ) -> Task<Message> {
        let opened = result.ok().flatten();
        let Some(view) =
            self.video.as_mut().filter(|view| view.track_id == track_id && view.player.is_some())
        else {
            if let Some(player) = opened.and_then(|opened| opened.take()) {
                discard(player);
            }
            return Task::none();
        };
        if let Some((player, opened)) = opened.and_then(|opened| Some((opened.take()?, opened))) {
            if let Some(old) = view.player.replace(player) {
                discard(old);
            }
            view.height = opened.height;
            self.align_video(true);
        }
        self.schedule_upgrade(track_id)
    }

    #[cfg(feature = "video")]
    pub(super) fn video_failed(&mut self) -> Task<Message> {
        match &self.video {
            Some(view) if !view.retried => self.load_video(true),
            other => {
                self.video_blocked = other.as_ref().map(|view| view.track_id);
                self.close_video();
                self.notice = Some(Notice::Error(AppError::Video.to_string()));
                Task::none()
            }
        }
    }

    /// Riapre il videoclip in corso, per esempio con l'altro decoder.
    pub(super) fn reopen_video(&mut self) -> Task<Message> {
        self.close_video();
        self.refresh_video()
    }

    /// Il videoclip segue il player: pausa, ripresa e posizione.
    pub(super) fn sync_video(&mut self, seek_to: Option<Duration>) {
        #[cfg(feature = "video")]
        if let Some(view) = self.video.as_mut()
            && let Some(player) = view.player.as_mut()
        {
            player.set_paused(!self.playback.is_playing());
            if let Some(position) = seek_to {
                let _ = player.seek(position);
                view.synced_at = Instant::now();
            }
        }
        #[cfg(not(feature = "video"))]
        let _ = seek_to;
    }

    /// Al Tick: se il video si è allontanato dall'audio (rete lenta, decoder in ritardo) lo si
    /// riporta alla posizione dell'audio, al massimo ogni `RESYNC_PAUSE`.
    pub(super) fn resync_video(&mut self) {
        #[cfg(feature = "video")]
        self.align_video(false);
    }

    /// Allinea il video all'audio: subito con `now`, altrimenti solo dopo `RESYNC_PAUSE` dall'ultima
    /// volta. Pausa e ripresa seguono sempre il player.
    #[cfg(feature = "video")]
    fn align_video(&mut self, now: bool) {
        let playing = self.playback.is_playing();
        let audio = self.audio_position();
        let Some(view) = self.video.as_mut() else { return };
        if !now && (!playing || view.synced_at.elapsed() < RESYNC_PAUSE) {
            return;
        }
        let Some(player) = view.player.as_mut() else { return };
        if player.paused() == playing {
            player.set_paused(!playing);
        }
        let duration = player.duration();
        // Un video più corto dell'audio resta sull'ultimo fotogramma.
        if (!duration.is_zero() && audio >= duration) || player.eos() {
            return;
        }
        let video = player.position();
        if video.abs_diff(audio) > VIDEO_DRIFT {
            #[cfg(feature = "tour")]
            eprintln!("tour: resync video {video:?} audio {audio:?} lead {:?}", view.lead);
            view.lead = if video < audio {
                (view.lead + (audio - video)).min(MAX_LEAD)
            } else {
                view.lead.saturating_sub(video - audio)
            };
            let _ = player.seek(audio + view.lead);
            view.synced_at = Instant::now();
        }
    }
}

#[cfg(all(test, feature = "video"))]
mod tests {
    use super::*;

    #[test]
    fn the_gpu_model_is_read_from_the_driver() {
        let info = "Model: \t\t NVIDIA GeForce RTX 5060 Laptop GPU\nIRQ:   \t\t 52\n";
        assert_eq!(model_line(info).as_deref(), Some("NVIDIA GeForce RTX 5060 Laptop GPU"));
        assert_eq!(model_line("Model: \nIRQ: 52"), None);
    }
}
