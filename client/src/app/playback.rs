//! I messaggi della riproduzione, smistati da `App::handle`, e la vista del core che li riflette.
//! Dal passo 4.3 suono, coda, brani preparati in anticipo, timer di spegnimento e riproduzione
//! automatica stanno nel core (`core/src/state/playback.rs`): qui ogni messaggio diventa un
//! `PlaybackCmd` e ciò che il core pubblica (`PlaybackView`) arriva in `App::playback_published`.
//! Restano nell'app il testo (con la riga in corso), la radio, i mix e gli album da riprodurre
//! (passo 4.3b), e i controlli del sistema.

use std::sync::Arc;
use std::time::Instant;

use iced::{Task, window};
use psytify_core::command::Artwork;
use psytify_core::command::{Command, PlaybackCmd};
use psytify_core::snapshot::{LyricsState, PlaybackEvent, PlaybackView};

use super::{App, HOVER_INTENT, Hovered, Load, Message, Notice, VOLUME_LINGER};
use crate::error::AppError;
use crate::player::PlaybackState;
use crate::player::eq::PRESETS;
use crate::ui::motion::Tween;
use crate::ui::theme as tokens;

/// Brani della coda di cui si scaricano le copertine quando la riproduzione automatica ne aggiunge.
const AUTOPLAY_COVERS: usize = 12;

impl App {
    /// Un comando alla riproduzione del core.
    pub(super) fn send_playback(&mut self, command: PlaybackCmd) -> Task<Message> {
        self.send_core(Command::Playback(command))
    }

    /// Copia nell'app la vista della riproduzione dell'ultima istantanea: gli `Arc` della vista, non
    /// gli elementi, e i pochi valori che l'interfaccia legge direttamente.
    pub(super) fn copy_playback(&mut self) {
        let view = Arc::clone(&self.core_snapshot.playback);
        if !Arc::ptr_eq(&self.queue, &view.queue) {
            self.queue = Arc::clone(&view.queue);
        }
        self.autoplay = view.autoplay;
        self.sleep = view.sleep;
        self.equalizer = view.equalizer;
        self.quality = view.quality;
        self.playback = view;
    }

    /// La riproduzione appena pubblicata dal core, al posto di `previous`: la copia dell'app si
    /// rinnova e si fa ciò che aspettava la vista nuova (testo, videoclip, controlli del sistema,
    /// avvisi).
    pub(super) fn playback_published(&mut self, previous: &PlaybackView) -> Task<Message> {
        self.copy_playback();
        let view = Arc::clone(&self.playback);
        let mut tasks = Vec::new();
        let current = |view: &PlaybackView| view.queue.current().map(|track| track.id);

        // Il testo che il core ha caricato (o sta caricando): la riga in corso riparte da capo.
        if previous.lyrics != view.lyrics {
            self.lyrics = match &view.lyrics.state {
                LyricsState::Idle => Load::Idle,
                LyricsState::Loading => Load::Loading,
                LyricsState::Ready(text) => Load::Ready(Arc::clone(text)),
                LyricsState::Failed(err) => Load::Failed(err.clone()),
            };
            self.lyrics_line = None;
            self.lyrics_scroll = Tween::rest(-1.0);
            tasks.push(self.follow_lyrics());
        }
        // Un altro brano in corso: la sua copertina, se già c'è, va al sistema (Android la vuole in
        // pixel); se arriva dopo, ci pensa `CoverLoaded`.
        if current(previous) != current(&view) {
            tasks.push(self.send_artwork());
        }
        // Fermo o un altro brano: lo slider trascinato non vale più.
        if previous.generation != view.generation
            || (view.state == PlaybackState::Stopped && previous.state != PlaybackState::Stopped)
        {
            self.scrubbing = None;
        }
        // Il videoclip segue il player (i controlli del sistema li pilota il core; i videoclip dei
        // brani in arrivo li prepara il core).
        self.follow_video(previous);
        if let Some(raised) = &view.event
            && raised.seq > self.playback_event
        {
            self.playback_event = raised.seq;
            tasks.push(self.playback_event_raised(raised.event.clone()));
        }
        Task::batch(tasks)
    }

    /// La copertina già decodificata del brano in corso, al core, che la dà alla sessione multimediale
    /// di Android (MPRIS legge l'indirizzo).
    pub(super) fn send_artwork(&mut self) -> Task<Message> {
        let Some(url) = self.queue.current().and_then(|track| track.cover_url.clone()) else {
            return Task::none();
        };
        match self.covers.artwork(&url) {
            Some((side, rgba)) => {
                self.send_playback(PlaybackCmd::Artwork(Box::new(Artwork { url, side, rgba })))
            }
            None => Task::none(),
        }
    }

    /// Le copertine dei brani in coda, da preparare: una radio o un album hanno brani mai visti.
    fn fetch_queue_covers(&mut self) -> Task<Message> {
        let urls = self.queue.entries().filter_map(|(_, track)| track.cover_url.clone()).collect();
        self.fetch_covers(urls)
    }

    /// Ciò che il core ha voluto far sapere all'ascoltatore.
    fn playback_event_raised(&mut self, event: PlaybackEvent) -> Task<Message> {
        match event {
            PlaybackEvent::SleepStopped => {
                self.notice = Some(Notice::Info("Buonanotte: la musica si è fermata".into()));
                Task::none()
            }
            PlaybackEvent::AutoplayAdded => {
                self.notice = Some(Notice::Info("Riproduzione automatica: ancora musica come questa".into()));
                let urls = self
                    .queue
                    .upcoming()
                    .take(AUTOPLAY_COVERS)
                    .filter_map(|(_, track)| track.cover_url.clone())
                    .collect();
                self.fetch_covers(urls)
            }
            PlaybackEvent::SeekFailed => {
                self.notice = Some(Notice::Error(AppError::Seek.to_string()));
                Task::none()
            }
            PlaybackEvent::RadioStarted(track) => {
                self.notice = Some(Notice::Info(format!("Radio di «{}»", track.title)));
                self.fetch_queue_covers()
            }
            PlaybackEvent::AlbumStarted => self.fetch_queue_covers(),
            PlaybackEvent::PlayFailed(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                Task::none()
            }
            // Il widget del desktop chiede di portare la finestra in primo piano.
            PlaybackEvent::Raise => self.window.map_or_else(Task::none, window::gain_focus),
        }
    }

    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_playback(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SeekFraction(fraction) => self.send_playback(PlaybackCmd::SeekFraction(fraction)),
            Message::PlayTracks(tracks, start) => self.send_playback(PlaybackCmd::Play { tracks, start }),
            Message::ShufflePlay(tracks) => self.send_playback(PlaybackCmd::ShufflePlay(tracks)),
            Message::PlayQueueAt(position) => self.send_playback(PlaybackCmd::PlayAt(position)),
            Message::Hover(hovered) => {
                self.hovered = hovered.map(|hovered| (hovered, Instant::now()));
                self.hover_generation += 1;
                let (generation, Some(Hovered { id, track: true })) = (self.hover_generation, hovered) else {
                    return Task::none();
                };
                Task::perform(tokio::time::sleep(HOVER_INTENT), move |()| {
                    Message::HoverSettled(generation, id)
                })
            }
            Message::HoverEnd(id) => {
                if self.hovered.is_some_and(|(hovered, _)| hovered.id == id) {
                    self.hovered = None;
                    self.hover_generation += 1;
                }
                Task::none()
            }
            Message::HoverSettled(generation, id) => {
                let still_there = self.hovered.is_some_and(|(hovered, _)| hovered.id == id);
                if generation == self.hover_generation && still_there {
                    self.send_playback(PlaybackCmd::Warm(id))
                } else {
                    Task::none()
                }
            }
            Message::TogglePlay => self.send_playback(PlaybackCmd::Toggle),
            Message::Next => self.send_playback(PlaybackCmd::Next),
            Message::Previous => self.send_playback(PlaybackCmd::Previous),
            Message::SeekBy(delta) => self.send_playback(PlaybackCmd::SeekBy(delta)),
            Message::Scrub(fraction) => {
                if self.playback.is_active() {
                    self.scrubbing = Some(fraction);
                }
                Task::none()
            }
            Message::ScrubReleased => match self.scrubbing.take() {
                Some(fraction) => self.send_playback(PlaybackCmd::SeekFraction(fraction)),
                None => Task::none(),
            },
            Message::Volume(volume) => self.send_playback(PlaybackCmd::Volume(volume)),
            Message::VolumeReleased => self.send_playback(PlaybackCmd::Save),
            Message::VolumeSide(volume) => {
                self.volume_touch += 1;
                self.volume_motion.retarget(1.0, tokens::MOTION_PANEL);
                self.send_playback(PlaybackCmd::Volume(volume))
            }
            Message::VolumeSideReleased => {
                let touch = self.volume_touch;
                Task::batch([
                    self.send_playback(PlaybackCmd::Save),
                    Task::perform(tokio::time::sleep(VOLUME_LINGER), move |()| {
                        Message::VolumeCollapse(touch)
                    }),
                ])
            }
            Message::VolumeCollapse(touch) => {
                if touch == self.volume_touch {
                    self.volume_motion.retarget(0.0, tokens::MOTION_PANEL);
                }
                Task::none()
            }
            Message::ToggleMute => self.send_playback(PlaybackCmd::ToggleMute),
            Message::ToggleShuffle => self.send_playback(PlaybackCmd::ToggleShuffle),
            Message::CycleRepeat => self.send_playback(PlaybackCmd::CycleRepeat),
            Message::Tick => {
                // La posizione la dà l'orologio del core: il Tick ridisegna la barra, allinea il
                // videoclip e segue il testo. Il suono lo legge il core, con il suo battito, e la
                // jam non ne ha bisogno (gli eventi della stanza entrano nel core da soli).
                self.resync_video();
                self.follow_lyrics()
            }
            Message::LyricSeek(at) => {
                if self.playback.is_active() {
                    self.send_playback(PlaybackCmd::SeekTo(at))
                } else {
                    Task::none()
                }
            }
            Message::EqualizerToggle(enabled) => self.send_playback(PlaybackCmd::EqualizerEnabled(enabled)),
            Message::EqualizerBand(band, gain) => {
                self.send_playback(PlaybackCmd::EqualizerBand { band, gain })
            }
            Message::EqualizerPreset(index) => match PRESETS.get(index) {
                Some((_, gains)) => self.send_playback(PlaybackCmd::EqualizerGains(*gains)),
                None => Task::none(),
            },
            Message::EqualizerReleased => self.send_playback(PlaybackCmd::Save),
            Message::Enqueue(track) => {
                self.notice = Some(Notice::Info(format!("«{}» aggiunto alla coda", track.title)));
                self.send_playback(PlaybackCmd::Enqueue(Box::new(track)))
            }
            Message::RemoveFromQueue(position) => self.send_playback(PlaybackCmd::Remove(position)),
            Message::ClearQueue => self.send_playback(PlaybackCmd::Clear),
            Message::PlayAlbum(id) => self.send_playback(PlaybackCmd::PlayAlbum(id)),
            Message::StartRadio(track) => self.send_playback(PlaybackCmd::StartRadio(track)),
            Message::PlayMix(index) => self.send_playback(PlaybackCmd::PlayMix(index)),
            Message::SetSleep(choice) => {
                self.notice = Some(Notice::Info(match choice {
                    Some(choice) => format!("Timer di spegnimento: {}", choice.label().to_lowercase()),
                    None => "Timer di spegnimento disattivato".into(),
                }));
                self.send_playback(PlaybackCmd::Sleep(choice))
            }
            Message::SetAutoplay(on) => self.send_playback(PlaybackCmd::Autoplay(on)),
            Message::LikeCurrent => match self.queue.current().cloned() {
                Some(track) => self.toggle_like(track),
                None => Task::none(),
            },
            Message::VolumeStep(delta) => {
                let volume = (self.playback.volume + delta).clamp(0.0, 1.0);
                self.handle(Message::Volume(volume))
            }
            Message::EnqueueNext(track) => self.enqueue_next(track),
            Message::SetQuality(quality) => {
                if quality != self.quality {
                    self.notice =
                        Some(Notice::Info(format!("Qualità {}: vale dal prossimo brano", quality.label())));
                }
                self.send_playback(PlaybackCmd::Quality(quality))
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}
