//! Il videoclip del brano in riproduzione: quando aprirlo, come tenerlo allineato all'audio e come
//! passare alla versione migliore. Il server dà subito una versione a 360p e poco dopo quella a
//! 720p; il client apre la prima, poi chiede di nuovo l'URL finché non arriva la seconda e la
//! sostituisce senza fermarsi. GStreamer apre il video su un thread a parte (può bloccare fino a
//! 5 secondi): la UI non si ferma mai.

#[cfg(feature = "video")]
use std::time::{Duration, Instant};

use iced::Task;
use uuid::Uuid;

#[cfg(feature = "video")]
use super::Notice;
use super::{App, Message, SideTab};
#[cfg(feature = "video")]
use crate::error::AppError;
#[cfg(feature = "video")]
use psytify_core::command::{Command, VideoCmd};
#[cfg(feature = "video")]
use psytify_core::player::Clock;
#[cfg(feature = "video")]
use psytify_core::snapshot::{Clip, ClipState, PlaybackView};
#[cfg(feature = "video")]
use psytify_core::video::{Adjust, Alignment, DRIFT, Measure, SETTLE};

/// Il player del videoclip del brano in corso. Che videoclip serve, quando aprirlo, che cosa fare se
/// non si apre e quando c'è una versione migliore lo decide il core (`VideoView`, passo 4.9); qui
/// sta solo GStreamer: aprire l'indirizzo che il core pubblica su un thread a parte (può bloccare
/// fino a 5 secondi: la UI non si ferma mai), tenerlo allineato all'audio con le regole del core
/// (`Alignment`) e chiuderlo.
#[cfg(feature = "video")]
pub struct VideoStage {
    pub track_id: Uuid,
    pub player: Option<iced_video_player::Video>,
    /// Il `seq` del videoclip del core che `player` mostra (o che si sta aprendo).
    pub(super) seq: u64,
    /// Una versione migliore, di questo `seq`, si sta aprendo di fianco: la si sostituisce quando è
    /// pronta, senza fermarsi.
    upgrading: Option<u64>,
    alignment: Alignment,
}

#[cfg(feature = "video")]
impl VideoStage {
    fn new(track_id: Uuid, seq: u64) -> Self {
        Self { track_id, player: None, seq, upgrading: None, alignment: Alignment::new(Instant::now()) }
    }
}

/// Un player aperto su un altro thread. Viaggia in un messaggio, che deve essere clonabile: chi lo
/// riceve lo prende una volta sola.
#[cfg(feature = "video")]
#[derive(Clone)]
pub struct OpenedVideo {
    player: std::sync::Arc<std::sync::Mutex<Option<iced_video_player::Video>>>,
}

#[cfg(feature = "video")]
impl std::fmt::Debug for OpenedVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedVideo").finish_non_exhaustive()
    }
}

#[cfg(feature = "video")]
impl OpenedVideo {
    fn take(&self) -> Option<iced_video_player::Video> {
        self.player.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

/// Ciò che il player misura di sé, per le regole di allineamento del core.
#[cfg(feature = "video")]
fn measure(player: &iced_video_player::Video) -> Measure {
    Measure {
        position: player.position(),
        duration: player.duration(),
        paused: player.paused(),
        ended: player.eos(),
    }
}

/// Fa al player ciò che le regole del core hanno deciso.
#[cfg(feature = "video")]
fn apply(player: &mut iced_video_player::Video, adjust: Adjust) {
    if let Some(paused) = adjust.pause {
        player.set_paused(paused);
    }
    if let Some(position) = adjust.seek {
        let _ = player.seek(position);
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

/// Apre l'indirizzo che il core ha pubblicato già alla posizione dell'audio: `clock` è l'orologio
/// della riproduzione, che il thread proietta in avanti mentre il video si prepara.
#[cfg(feature = "video")]
async fn open(url: String, clock: Clock, dedicated: bool) -> Result<OpenedVideo, AppError> {
    #[cfg(feature = "tour")]
    let asked = Instant::now();
    let url = url::Url::parse(&url).map_err(|_| AppError::Video)?;
    let player = tokio::task::spawn_blocking(move || {
        // L'errore di GStreamer può contenere l'URL firmato: non lo si conserva.
        choose_decoders(dedicated);
        let mut player = iced_video_player::Video::new(&url).map_err(|_| AppError::Video)?;
        player.set_paused(!clock.playing);
        let target = clock.at(Instant::now());
        if player.seek(target).is_ok() {
            let deadline = Instant::now() + SETTLE;
            while Instant::now() < deadline && player.position().abs_diff(clock.at(Instant::now())) > DRIFT {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(feature = "tour")]
        eprintln!("tour: video open+seek {:?} at {:?}", asked.elapsed(), player.position());
        Ok::<_, AppError>(player)
    })
    .await
    .map_err(|_| AppError::Video)??;
    Ok(OpenedVideo { player: std::sync::Arc::new(std::sync::Mutex::new(Some(player))) })
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
        self.video_available() && self.queue.current().is_some_and(|t| self.video_failed_for(t.id))
    }

    #[cfg(feature = "video")]
    fn video_player_missing(&self) -> bool {
        self.video.as_ref().is_none_or(|v| v.player.is_none())
    }

    #[cfg(not(feature = "video"))]
    fn video_player_missing(&self) -> bool {
        false
    }

    /// Il core ha fermato il videoclip di quel brano (non si apriva, nemmeno al secondo tentativo):
    /// non lo riprova finché il brano non cambia o l'ascoltatore non preme «Riprova».
    #[cfg(feature = "video")]
    fn video_failed_for(&self, track: Uuid) -> bool {
        let view = &self.core_snapshot.video;
        view.track == Some(track) && matches!(view.state, ClipState::Failed(_))
    }

    #[cfg(not(feature = "video"))]
    fn video_failed_for(&self, _track: Uuid) -> bool {
        false
    }

    /// Un posto dell'interfaccia mostra il videoclip: la vista a tutto schermo, oppure il pannello
    /// «In riproduzione» aperto con il videoclip scelto al posto della copertina. Il core lo sa
    /// (`VideoCmd::Show`) e chiede il videoclip al server solo allora.
    pub(super) fn video_surface(&self) -> bool {
        let in_panel = self.panel_open && self.side_tab == SideTab::NowPlaying && self.panel_video;
        let on_stage = self.focus && self.focus_video;
        cfg!(feature = "video") && (in_panel || on_stage)
    }

    /// Il brano di cui qualcuno vuole il videoclip, se può averlo.
    pub(super) fn wanted_video(&self) -> Option<Uuid> {
        let track = self.queue.current().filter(|_| self.video_available())?;
        if self.video_failed_for(track.id) {
            return None;
        }
        self.video_surface().then_some(track.id)
    }

    /// Il videoclip che il core pubblica è cambiato: si apre quello nuovo, si sostituisce con la
    /// versione migliore, o si chiude il player se non c'è più niente da mostrare. Un videoclip che
    /// si è fermato lo dice con un avviso, una volta.
    #[cfg(feature = "video")]
    pub(super) fn video_published(&mut self) -> Task<Message> {
        let view = std::sync::Arc::clone(&self.core_snapshot.video);
        if view.failures > self.video_failures {
            self.video_failures = view.failures;
            if let ClipState::Failed(error) = &view.state {
                self.notice = Some(Notice::Error(error.to_string()));
            }
        }
        match (view.track, &view.state) {
            (Some(track), ClipState::Ready(clip)) => self.open_clip(track, clip),
            // Fermo, in preparazione, o non riuscito: niente player.
            _ => {
                self.close_video();
                Task::none()
            }
        }
    }

    /// Apre il videoclip che il core ha pubblicato, se non è già quello che c'è.
    #[cfg(feature = "video")]
    fn open_clip(&mut self, track: Uuid, clip: &Clip) -> Task<Message> {
        let (url, seq) = (clip.url().to_owned(), clip.seq);
        let (clock, dedicated) = (self.playback.clock, self.dedicated_video);
        match self.video.as_mut() {
            // Lo stesso videoclip già aperto (o in apertura) resta com'è.
            Some(stage) if stage.track_id == track && (stage.seq == seq || stage.upgrading == Some(seq)) => {
                Task::none()
            }
            // Un indirizzo nuovo per lo stesso brano, con il player già aperto: una versione
            // migliore, che si apre di fianco e prende il posto di quella che suona.
            Some(stage) if stage.track_id == track && stage.player.is_some() => {
                stage.upgrading = Some(seq);
                Task::perform(open(url, clock, dedicated), move |result| {
                    Message::VideoUpgraded(track, seq, result)
                })
            }
            _ => {
                self.close_video();
                self.video = Some(VideoStage::new(track, seq));
                Task::perform(open(url, clock, dedicated), move |result| {
                    Message::VideoOpened(track, seq, result)
                })
            }
        }
    }

    #[cfg(feature = "video")]
    fn close_video(&mut self) {
        if let Some(player) = self.video.take().and_then(|stage| stage.player) {
            discard(player);
        }
    }

    /// Il videoclip si è aperto su un altro thread.
    #[cfg(feature = "video")]
    pub(super) fn video_opened(
        &mut self,
        track: Uuid,
        seq: u64,
        result: Result<OpenedVideo, AppError>,
    ) -> Task<Message> {
        let player = result.ok().and_then(|opened| opened.take());
        let waiting = self
            .video
            .as_ref()
            .is_some_and(|stage| stage.track_id == track && stage.seq == seq && stage.player.is_none());
        if !waiting {
            // Arrivato tardi: il brano è cambiato o il videoclip è stato chiuso.
            if let Some(player) = player {
                discard(player);
            }
            return Task::none();
        }
        match (player, self.video.as_mut()) {
            (Some(player), Some(stage)) => {
                stage.player = Some(player);
                stage.alignment = Alignment::new(Instant::now());
                self.align_video(true);
                Task::none()
            }
            // Non si è aperto: il core ne chiede un altro indirizzo una volta, poi lo ferma.
            _ => self.send_video(VideoCmd::Failed { seq }),
        }
    }

    /// La versione migliore si è aperta di fianco: prende il posto di quella che suonava, senza
    /// fermarsi. Se non si apre il core lo sa e riprova più tardi, e intanto resta la vecchia.
    #[cfg(feature = "video")]
    pub(super) fn video_upgraded(
        &mut self,
        track: Uuid,
        seq: u64,
        result: Result<OpenedVideo, AppError>,
    ) -> Task<Message> {
        let player = result.ok().and_then(|opened| opened.take());
        let view = &self.core_snapshot.video;
        let current =
            view.track == Some(track) && matches!(&view.state, ClipState::Ready(clip) if clip.seq == seq);
        let Some(stage) = self.video.as_mut().filter(|stage| {
            stage.track_id == track && stage.player.is_some() && stage.upgrading == Some(seq)
        }) else {
            if let Some(player) = player {
                discard(player);
            }
            return Task::none();
        };
        stage.upgrading = None;
        match player {
            Some(player) if current => {
                if let Some(old) = stage.player.replace(player) {
                    discard(old);
                }
                stage.seq = seq;
                self.align_video(true);
                Task::none()
            }
            Some(player) => {
                // Un altro indirizzo ha già preso il suo posto: questo non serve più.
                discard(player);
                Task::none()
            }
            None => self.send_video(VideoCmd::Failed { seq }),
        }
    }

    /// Lo stream del videoclip è caduto: lo dice al core, che ne chiede un altro indirizzo una volta.
    #[cfg(feature = "video")]
    pub(super) fn video_failed(&mut self) -> Task<Message> {
        match self.video.as_ref().filter(|stage| stage.player.is_some()).map(|stage| stage.seq) {
            Some(seq) => self.send_video(VideoCmd::Failed { seq }),
            None => Task::none(),
        }
    }

    /// Il videoclip segue il player: se la riproduzione è cambiata da `previous` (un brano che parte,
    /// un salto, pausa o ripresa) lo porta dove sta l'audio.
    pub(super) fn follow_video(&mut self, previous: &psytify_core::snapshot::PlaybackView) {
        #[cfg(feature = "video")]
        if let Some(stage) = self.video.as_mut()
            && let Some(player) = stage.player.as_mut()
            && let Some(adjust) = stage.alignment.follow(previous, &self.playback, Instant::now())
        {
            apply(player, adjust);
        }
        #[cfg(not(feature = "video"))]
        let _ = previous;
    }

    /// Al Tick: se il video si è allontanato dall'audio (rete lenta, decoder in ritardo) lo si
    /// riporta alla posizione dell'audio, secondo le regole del core.
    pub(super) fn resync_video(&mut self) {
        #[cfg(feature = "video")]
        self.align_video(false);
    }

    /// Allinea il video all'audio: subito con `force` (appena aperto), altrimenti solo se è passato
    /// abbastanza dall'ultima volta. Pausa e ripresa seguono sempre il player.
    #[cfg(feature = "video")]
    fn align_video(&mut self, force: bool) {
        let playback: &PlaybackView = &self.playback;
        let Some(stage) = self.video.as_mut() else { return };
        let Some(player) = stage.player.as_mut() else { return };
        let now = Instant::now();
        let adjust = stage.alignment.align(playback, measure(player), force, now);
        #[cfg(feature = "tour")]
        if adjust.seek.is_some() {
            eprintln!(
                "tour: resync video {:?} audio {:?} lead {:?}",
                player.position(),
                playback.position(now),
                stage.alignment.lead()
            );
        }
        apply(player, adjust);
    }
}

/// I comandi al core per il videoclip: in una build con `video` il core ha il dominio.
#[cfg(feature = "video")]
impl App {
    fn send_video(&mut self, command: VideoCmd) -> Task<Message> {
        self.send_core(Command::Video(command))
    }

    /// Un posto dell'interfaccia mostra (o non mostra più) il videoclip.
    pub(super) fn show_video(&mut self, on: bool) -> Task<Message> {
        self.send_video(VideoCmd::Show(on))
    }

    /// Chi ascolta vuole (o non vuole più) i videoclip dei brani in arrivo preparati dal server.
    fn prepare_videos(&mut self, on: bool) -> Task<Message> {
        self.send_video(VideoCmd::PrepareAhead(on))
    }

    /// Toglie il blocco di un videoclip che non si apriva.
    fn retry_video(&mut self) -> Task<Message> {
        self.send_video(VideoCmd::Retry)
    }

    /// Riapre il videoclip in corso, per esempio con l'altro decoder.
    fn reopen_video(&mut self) -> Task<Message> {
        self.send_video(VideoCmd::Reopen)
    }

    /// Dice al core che cosa si vuole del video: all'avvio, con il pannello riaperto sul videoclip del
    /// brano ripreso (altrimenti resta su «Preparo il videoclip…» finché qualcosa non cambia) e con la
    /// scelta di averli in anticipo.
    pub(super) fn video_wants(&mut self) -> Task<Message> {
        let (show, ahead) = (self.video_surface(), self.panel_video);
        Task::batch([self.prepare_videos(ahead), self.show_video(show)])
    }
}

/// Senza `video` (Android, build senza GStreamer) il core non ha il dominio: niente da dire.
#[cfg(not(feature = "video"))]
impl App {
    pub(super) fn show_video(&mut self, _on: bool) -> Task<Message> {
        Task::none()
    }

    fn prepare_videos(&mut self, _on: bool) -> Task<Message> {
        Task::none()
    }

    fn retry_video(&mut self) -> Task<Message> {
        Task::none()
    }

    fn reopen_video(&mut self) -> Task<Message> {
        Task::none()
    }

    pub(super) fn video_wants(&mut self) -> Task<Message> {
        Task::none()
    }
}

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_video(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetDedicatedVideo(on) => {
                self.dedicated_video = on;
                // Il videoclip aperto riparte con il decoder scelto; il prossimo lo usa da sé.
                Task::batch([self.persist(), self.reopen_video()])
            }
            Message::GpuDetected(gpu) => {
                self.dedicated_gpu = gpu;
                Task::none()
            }
            Message::SetPanelVideo(on) => {
                self.panel_video = on;
                Task::batch([self.retry_video(), self.prepare_videos(on), self.persist()])
            }
            Message::RetryVideo => self.retry_video(),
            Message::ToggleVideoFocus => {
                if self.focus && self.focus_video {
                    self.focus = false;
                    self.focus_video = false;
                    return Task::none();
                }
                let Some(id) = self.queue.current().filter(|t| t.has_video).map(|t| t.id) else {
                    return Task::none();
                };
                self.focus = true;
                self.focus_video = true;
                if self.video_failed_for(id) { self.retry_video() } else { Task::none() }
            }
            Message::ToggleVideoLyrics => {
                self.video_lyrics = !self.video_lyrics;
                Task::none()
            }
            #[cfg(feature = "video")]
            Message::VideoOpened(track_id, seq, result) => self.video_opened(track_id, seq, result),
            #[cfg(feature = "video")]
            Message::VideoUpgraded(track_id, seq, result) => self.video_upgraded(track_id, seq, result),
            #[cfg(feature = "video")]
            Message::VideoFailed => self.video_failed(),
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
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
