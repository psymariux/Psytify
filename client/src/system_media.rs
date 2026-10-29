//! Controlli multimediali del sistema (MPRIS su Linux): i tasti multimediali della tastiera, il
//! widget del desktop e le cuffie comandano il player, e il sistema mostra titolo, artista,
//! copertina e posizione. Su Windows e macOS per ora non c'è: servirebbe la finestra nativa
//! (HWND), e lì `subscription` non produce nulla.
#![cfg_attr(
    not(target_os = "linux"),
    expect(dead_code, reason = "fuori da Linux i comandi del sistema non arrivano mai")
)]

use std::time::Duration;

use iced::Subscription;

/// Un comando arrivato dal sistema.
#[derive(Debug, Clone)]
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
}

/// Il secondi di un «avanti» o «indietro» senza durata.
const SEEK_STEP: i64 = 5;

#[cfg(target_os = "linux")]
pub use linux::{Handle, subscription};
#[cfg(not(target_os = "linux"))]
pub use other::{Handle, subscription};

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

#[cfg(not(target_os = "linux"))]
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
