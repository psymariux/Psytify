//! Controlli multimediali del sistema (MPRIS su Linux, la sessione multimediale su Android): i tasti
//! multimediali della tastiera, il widget del desktop, la schermata di blocco del telefono e le
//! cuffie comandano il player, e il sistema mostra titolo, artista, copertina e posizione. Su
//! Android il lavoro con Java lo fa il guscio (`android/src/media.rs`), agganciato con
//! `set_phone_media`. Su Windows e macOS per ora non c'è: servirebbe la finestra nativa (HWND), e lì
//! `subscription` non produce nulla.
#![cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    expect(dead_code, reason = "fuori da Linux i comandi del sistema non arrivano mai")
)]

use std::time::Duration;

use iced::Subscription;

/// Un comando arrivato dal sistema.
#[derive(Debug, Clone)]
#[cfg_attr(
    target_os = "android",
    expect(dead_code, reason = "il telefono non manda salti relativi, volume né «porta in primo piano»")
)]
pub enum Event {
    /// Il collegamento è pronto: da qui in poi si aggiorna ciò che il sistema mostra.
    Ready(Handle),
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    /// Avanti (positivo) o indietro, in secondi.
    SeekBy(i64),
    SetPosition(Duration),
    SetVolume(f32),
    /// Porta la finestra in primo piano.
    Raise,
}

/// Ciò che il sistema mostra del brano in riproduzione.
pub struct NowPlaying<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: Option<&'a str>,
    pub cover_url: Option<&'a str>,
    pub duration: Duration,
    pub playing: bool,
    pub position: Duration,
    /// La copertina già scaricata (lato e RGBA), per chi la vuole in pixel e non per indirizzo.
    #[cfg_attr(not(target_os = "android"), expect(dead_code, reason = "MPRIS la legge dall'indirizzo"))]
    pub artwork: Option<(u32, &'a [u8])>,
}

/// Il secondi di un «avanti» o «indietro» senza durata.
#[cfg_attr(target_os = "android", expect(dead_code, reason = "lo usa solo MPRIS"))]
const SEEK_STEP: i64 = 5;

#[cfg(target_os = "linux")]
pub use linux::{Handle, subscription};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub use other::{Handle, subscription};
#[cfg(target_os = "android")]
pub use phone::{Handle, subscription};
#[cfg(target_os = "android")]
pub use phone::{
    command as phone_command, install as phone_install, notify_update as phone_notify_update,
    register as set_phone_media, register_install as set_phone_installer,
    register_update_notifier as set_phone_update_notifier,
};

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::{Arc, Mutex, PoisonError};

    use futures_util::SinkExt;
    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
        SeekDirection,
    };

    use super::{Event, NowPlaying, SEEK_STEP, Subscription};

    /// Il collegamento con il sistema; si clona per viaggiare nei messaggi.
    #[derive(Clone)]
    pub struct Handle(Arc<Mutex<MediaControls>>);

    impl std::fmt::Debug for Handle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Handle")
        }
    }

    impl Handle {
        /// Mostra il brano (o niente, a player fermo). Un errore di D-Bus non ferma la musica: il
        /// widget del desktop resta solo indietro.
        pub fn show(&self, now: Option<&NowPlaying<'_>>) {
            let mut controls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = match now {
                None => controls.set_playback(MediaPlayback::Stopped),
                Some(now) => controls
                    .set_metadata(MediaMetadata {
                        title: Some(now.title),
                        artist: Some(now.artist),
                        album: now.album,
                        cover_url: now.cover_url,
                        duration: Some(now.duration),
                    })
                    .and_then(|()| {
                        let progress = Some(MediaPosition(now.position));
                        controls.set_playback(if now.playing {
                            MediaPlayback::Playing { progress }
                        } else {
                            MediaPlayback::Paused { progress }
                        })
                    }),
            };
        }

        pub fn set_volume(&self, volume: f32) {
            let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).set_volume(f64::from(volume));
        }
    }

    pub fn subscription() -> Subscription<Event> {
        Subscription::run(events)
    }

    /// Registra Psytify sul bus della sessione e inoltra i comandi. Senza bus (per esempio fuori
    /// da una sessione grafica) non succede nulla.
    fn events() -> impl futures_util::Stream<Item = Event> {
        iced::stream::channel(32, |mut output| async move {
            // Un nome per processo, come chiede MPRIS con più istanze: con un nome fisso la seconda
            // finestra farebbe cadere il thread di D-Bus (`NameTaken`). `playerctl -p psytify` le trova tutte.
            let name = format!("psytify.instance{}", std::process::id());
            let config = PlatformConfig { display_name: "Psytify", dbus_name: &name, hwnd: None };
            let Ok(mut controls) = MediaControls::new(config) else { return };
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            if controls.attach(move |event| drop(sender.send(event))).is_err() {
                return;
            }
            if output.send(Event::Ready(Handle(Arc::new(Mutex::new(controls))))).await.is_err() {
                return;
            }
            while let Some(event) = receiver.recv().await {
                if let Some(event) = translate(event)
                    && output.send(event).await.is_err()
                {
                    return;
                }
            }
        })
    }

    fn translate(event: MediaControlEvent) -> Option<Event> {
        let signed = |direction, secs: i64| if direction == SeekDirection::Forward { secs } else { -secs };
        Some(match event {
            MediaControlEvent::Play => Event::Play,
            MediaControlEvent::Pause | MediaControlEvent::Stop => Event::Pause,
            MediaControlEvent::Toggle => Event::Toggle,
            MediaControlEvent::Next => Event::Next,
            MediaControlEvent::Previous => Event::Previous,
            MediaControlEvent::Seek(direction) => Event::SeekBy(signed(direction, SEEK_STEP)),
            MediaControlEvent::SeekBy(direction, by) => {
                Event::SeekBy(signed(direction, i64::try_from(by.as_secs()).unwrap_or(i64::MAX)))
            }
            MediaControlEvent::SetPosition(MediaPosition(at)) => Event::SetPosition(at),
            MediaControlEvent::SetVolume(volume) => Event::SetVolume(volume.clamp(0.0, 1.0) as f32),
            MediaControlEvent::Raise => Event::Raise,
            MediaControlEvent::OpenUri(_) | MediaControlEvent::Quit => return None,
        })
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::*;

        #[test]
        fn system_seeks_become_signed_seconds() {
            let back = translate(MediaControlEvent::SeekBy(SeekDirection::Backward, Duration::from_secs(10)));
            assert!(matches!(back, Some(Event::SeekBy(-10))));
            assert!(matches!(
                translate(MediaControlEvent::Seek(SeekDirection::Forward)),
                Some(Event::SeekBy(5))
            ));
            assert!(matches!(translate(MediaControlEvent::Stop), Some(Event::Pause)));
            assert!(translate(MediaControlEvent::Quit).is_none());
        }
    }
}

/// Android: il guscio registra chi mostra il brano al sistema e manda indietro i comandi (Play,
/// Pausa, Successivo, Precedente, salto) con `command`.
#[cfg(target_os = "android")]
mod phone {
    use std::path::Path;
    use std::sync::{Mutex, OnceLock, PoisonError};
    use std::time::Duration;

    use futures_util::SinkExt;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::{Event, NowPlaying, Subscription};

    type Show = Box<dyn Fn(Option<&NowPlaying<'_>>) + Send + Sync>;
    type Commands = (UnboundedSender<Event>, Mutex<Option<UnboundedReceiver<Event>>>);

    static SHOW: OnceLock<Show> = OnceLock::new();
    static INSTALL: OnceLock<Box<dyn Fn(&Path) + Send + Sync>> = OnceLock::new();
    static NOTIFY_UPDATE: OnceLock<Box<dyn Fn(&str) + Send + Sync>> = OnceLock::new();
    static COMMANDS: OnceLock<Commands> = OnceLock::new();

    fn commands() -> &'static Commands {
        COMMANDS.get_or_init(|| {
            let (sender, receiver) = unbounded_channel();
            (sender, Mutex::new(Some(receiver)))
        })
    }

    /// Registra chi mostra il brano al sistema. Vale la prima chiamata.
    pub fn register(show: impl Fn(Option<&NowPlaying<'_>>) + Send + Sync + 'static) {
        let _ = SHOW.set(Box::new(show));
    }

    /// Il guscio Android registra come si installa un pacchetto scaricato dall'app.
    pub fn register_install(install: impl Fn(&Path) + Send + Sync + 'static) {
        let _ = INSTALL.set(Box::new(install));
    }

    /// Il guscio Android registra come si mostra la notifica di una versione nuova.
    pub fn register_update_notifier(notify: impl Fn(&str) + Send + Sync + 'static) {
        let _ = NOTIFY_UPDATE.set(Box::new(notify));
    }

    /// Avvisa con una notifica di sistema che c'è la versione `tag`.
    pub fn notify_update(tag: &str) {
        if let Some(notify) = NOTIFY_UPDATE.get() {
            notify(tag);
        }
    }

    /// Passa il pacchetto al sistema, che chiede la conferma all'utente e installa.
    pub fn install(path: &Path) {
        if let Some(install) = INSTALL.get() {
            install(path);
        }
    }

    /// Un comando dal sistema: 0 Play, 1 Pausa, 2 Successivo, 3 Precedente, 4 salto a `value` ms,
    /// 5 Play o Pausa. Gli altri codici non fanno nulla.
    pub fn command(code: i32, value: i64) {
        let event = match code {
            0 => Event::Play,
            1 => Event::Pause,
            2 => Event::Next,
            3 => Event::Previous,
            4 => Event::SetPosition(Duration::from_millis(u64::try_from(value).unwrap_or(0))),
            5 => Event::Toggle,
            _ => return,
        };
        let _ = commands().0.send(event);
    }

    #[derive(Debug, Clone)]
    pub struct Handle;

    impl Handle {
        pub fn show(&self, now: Option<&NowPlaying<'_>>) {
            if let Some(show) = SHOW.get() {
                show(now);
            }
        }

        pub fn set_volume(&self, _volume: f32) {}
    }

    pub fn subscription() -> Subscription<Event> {
        Subscription::run(events)
    }

    /// Senza guscio registrato (desktop con `PSYTIFY_PHONE=1`) non succede nulla.
    fn events() -> impl futures_util::Stream<Item = Event> {
        iced::stream::channel(32, |mut output| async move {
            if SHOW.get().is_none() {
                return;
            }
            let Some(mut receiver) = commands().1.lock().unwrap_or_else(PoisonError::into_inner).take()
            else {
                return;
            };
            if output.send(Event::Ready(Handle)).await.is_err() {
                return;
            }
            while let Some(event) = receiver.recv().await {
                if output.send(event).await.is_err() {
                    return;
                }
            }
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod other {
    use super::{Event, NowPlaying, Subscription};

    #[derive(Debug, Clone)]
    pub struct Handle;

    impl Handle {
        pub fn show(&self, _now: Option<&NowPlaying<'_>>) {}

        pub fn set_volume(&self, _volume: f32) {}
    }

    pub fn subscription() -> Subscription<Event> {
        Subscription::none()
    }
}
