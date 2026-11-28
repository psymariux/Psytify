//! System media controls (MPRIS on Linux, the media session on Android): the keyboard's media keys, the
//! desktop widget, the phone's lock screen and the headphones control the player, and the system shows the
//! title, artist, cover and position. On Android the Java work is done by the shell (`android/src/media.rs`),
//! attached with `set_phone_media`. On Windows and macOS there is none for now: it would need the native
//! window (HWND), and there `events` produces nothing.
//!
//! The commands arrive as a neutral stream ([`events`]), with no framework: the frontend listens to it as a
//! stream, and the core can consume it directly.
#![cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    expect(dead_code, reason = "fuori da Linux i comandi del sistema non arrivano mai")
)]

use std::future::Future;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

/// A command that came from the system.
#[derive(Debug, Clone)]
pub enum Event {
    /// The link is ready: from now on what the system shows is updated.
    Ready(Handle),
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    /// Forward (positive) or back, in seconds.
    SeekBy(i64),
    SetPosition(Duration),
    SetVolume(f32),
    /// Brings the window to the front.
    Raise,
}

/// What the system shows of the track playing.
pub struct NowPlaying<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: Option<&'a str>,
    pub cover_url: Option<&'a str>,
    pub duration: Duration,
    pub playing: bool,
    pub position: Duration,
    /// The cover already downloaded (side and RGBA), for whoever wants it in pixels and not by address
    /// (MPRIS reads it from the address).
    pub artwork: Option<(u32, &'a [u8])>,
}

/// The track to show to the system, with its texts: the reducer builds it (no borrow, it can
/// travel in an effect) and the host makes it a [`NowPlaying`] with [`Now::borrowed`].
#[derive(Debug, Clone)]
pub struct Now {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration: Duration,
    pub playing: bool,
    pub position: Duration,
    /// The cover already decoded: side and RGBA.
    pub artwork: Option<(u32, bytes::Bytes)>,
}

impl Now {
    pub fn borrowed(&self) -> NowPlaying<'_> {
        NowPlaying {
            title: &self.title,
            artist: &self.artist,
            album: self.album.as_deref(),
            cover_url: self.cover_url.as_deref(),
            duration: self.duration,
            playing: self.playing,
            position: self.position,
            artwork: self.artwork.as_ref().map(|(side, rgba)| (*side, &rgba[..])),
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{Handle, events};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub use other::{Handle, events};
#[cfg(target_os = "android")]
pub use phone::{Handle, events};
#[cfg(target_os = "android")]
pub use phone::{command as phone_command, register as set_phone_media};

/// A stream written by a task: `run` gets the sender that writes and the stream ends when `run`
/// ends and the commands already written have been read. It is a stream channel, built on
/// tokio's channel.
/// The system's reduced motion (`system_motion`) uses it too.
pub(crate) fn channel<T, F>(size: usize, run: impl FnOnce(mpsc::Sender<T>) -> F) -> impl Stream<Item = T>
where
    F: Future<Output = ()>,
{
    let (sender, receiver) = mpsc::channel(size);
    let received = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| (event, receiver))
    });
    let running = futures_util::stream::once(run(sender)).filter_map(|()| async { None });
    futures_util::stream::select(received, running)
}

#[cfg(target_os = "linux")]
mod linux {
    //! MPRIS on the session bus, with zbus (in pure Rust: no libdbus to install). The service is an
    //! async task inside the [`events`] stream: it waits for a method called by the system or for a change in
    //! what is shown, and with neither it does not wake up (souvlaki, which was there before, polled its
    //! channel every 10 ms: 100 wake-ups a second even at rest, BACKLOG 13). The interfaces and properties
    //! are those souvlaki exposed, with the names of the specification.

    use std::collections::HashMap;
    use std::time::Duration;

    use futures_util::Stream;
    use tokio::sync::mpsc::{self, UnboundedSender, unbounded_channel};
    use zbus::object_server::InterfaceRef;
    use zbus::{ObjectPath, Variant};
    use zbus::{connection::Builder, interface};

    use super::{Event, NowPlaying};

    /// Where the two interfaces are, as MPRIS wants.
    const PATH: &str = "/org/mpris/MediaPlayer2";

    /// The link with the system; it is cloned to travel in messages. What is shown goes to the
    /// service over a channel: whoever calls it does not wait for the bus.
    #[derive(Clone)]
    pub struct Handle(UnboundedSender<Change>);

    impl std::fmt::Debug for Handle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Handle")
        }
    }

    impl Handle {
        /// Shows the track (or nothing, with the player stopped). A D-Bus error does not stop the music: the
        /// desktop widget just stays behind.
        pub fn show(&self, now: Option<&NowPlaying<'_>>) {
            match now {
                None => drop(self.0.send(Change::Playback(Status::Stopped))),
                Some(now) => {
                    let _ = self.0.send(Change::Metadata(Metadata::of(now)));
                    let status = if now.playing { Status::Playing } else { Status::Paused };
                    let _ = self.0.send(Change::Position(now.position));
                    let _ = self.0.send(Change::Playback(status));
                }
            }
        }

        pub fn set_volume(&self, volume: f32) {
            let _ = self.0.send(Change::Volume(f64::from(volume)));
        }
    }

    /// The playback state that the system reads.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Status {
        Playing,
        Paused,
        Stopped,
    }

    /// The track as the system reads it.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct Metadata {
        title: String,
        artist: String,
        album: Option<String>,
        cover_url: Option<String>,
        /// In microseconds, as `mpris:length` wants.
        length: i64,
    }

    impl Metadata {
        fn of(now: &NowPlaying<'_>) -> Self {
            Self {
                title: now.title.to_owned(),
                artist: now.artist.to_owned(),
                album: now.album.map(str::to_owned),
                cover_url: now.cover_url.map(str::to_owned),
                length: micros(now.duration),
            }
        }
    }

    /// A change of what is shown, from the core to the service.
    #[derive(Debug)]
    enum Change {
        Metadata(Metadata),
        Position(Duration),
        Playback(Status),
        Volume(f64),
    }

    fn micros(duration: Duration) -> i64 {
        i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
    }

    /// MPRIS `Seek` (microseconds, forward if positive) in whole seconds, as before with souvlaki.
    fn seek(offset: i64) -> Event {
        let secs = i64::try_from(offset.unsigned_abs() / 1_000_000).unwrap_or(i64::MAX);
        Event::SeekBy(if offset > 0 { secs } else { -secs })
    }

    /// MPRIS `SetPosition`: nothing if the position is negative or beyond the end of the track.
    fn set_position(position: i64, length: Option<i64>) -> Option<Event> {
        let micros = u64::try_from(position).ok()?;
        if length.is_some_and(|length| position > length) {
            return None;
        }
        Some(Event::SetPosition(Duration::from_micros(micros)))
    }

    /// `org.mpris.MediaPlayer2`: who the player is.
    struct Root {
        commands: UnboundedSender<Event>,
    }

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {
            let _ = self.commands.send(Event::Raise);
        }

        /// Closing the app from the widget did not exist even before: closing goes through the window's save.
        fn quit(&self) {}

        #[zbus(property)]
        fn can_quit(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_raise(&self) -> bool {
            true
        }

        #[zbus(property, name = "HasTrackList")]
        fn has_track_list(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn identity(&self) -> &'static str {
            "Psytify"
        }

        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            Vec::new()
        }

        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            Vec::new()
        }
    }

    /// `org.mpris.MediaPlayer2.Player`: the commands and what is shown.
    struct Player {
        commands: UnboundedSender<Event>,
        status: Status,
        position: Duration,
        metadata: Option<Metadata>,
        volume: f64,
    }

    impl Player {
        fn send(&self, event: Event) {
            let _ = self.commands.send(event);
        }
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn next(&self) {
            self.send(Event::Next);
        }

        fn previous(&self) {
            self.send(Event::Previous);
        }

        fn pause(&self) {
            self.send(Event::Pause);
        }

        fn play_pause(&self) {
            self.send(Event::Toggle);
        }

        fn stop(&self) {
            self.send(Event::Pause);
        }

        fn play(&self) {
            self.send(Event::Play);
        }

        fn seek(&self, offset: i64) {
            self.send(seek(offset));
        }

        fn set_position(&self, _track_id: ObjectPath<'_>, position: i64) {
            if let Some(event) = set_position(position, self.metadata.as_ref().map(|m| m.length)) {
                self.send(event);
            }
        }

        /// Psytify does not open addresses from the system.
        fn open_uri(&self, _uri: String) {}

        #[zbus(property)]
        fn playback_status(&self) -> &'static str {
            match self.status {
                Status::Playing => "Playing",
                Status::Paused => "Paused",
                Status::Stopped => "Stopped",
            }
        }

        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<&'static str, Variant<'static>> {
            let mut dict = HashMap::new();
            // A track id is needed so the system can ask for `SetPosition`.
            if let Ok(id) = ObjectPath::try_from("/") {
                dict.insert("mpris:trackid", Variant::new(id));
            }
            if let Some(metadata) = &self.metadata {
                dict.insert("mpris:length", Variant::new(metadata.length));
                dict.insert("xesam:title", Variant::new(metadata.title.clone()));
                dict.insert("xesam:artist", Variant::new(vec![metadata.artist.clone()]));
                if let Some(album) = &metadata.album {
                    dict.insert("xesam:album", Variant::new(album.clone()));
                }
                if let Some(cover) = &metadata.cover_url {
                    dict.insert("mpris:artUrl", Variant::new(cover.clone()));
                }
            }
            dict
        }

        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.volume
        }

        #[zbus(property)]
        fn set_volume(&self, volume: f64) {
            self.send(Event::SetVolume(volume.clamp(0.0, 1.0) as f32));
        }

        #[zbus(property)]
        fn position(&self) -> i64 {
            if self.status == Status::Stopped { 0 } else { micros(self.position) }
        }

        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    /// Registers Psytify on the session bus and forwards the commands. Without a bus (for example outside
    /// a graphical session) the stream ends at once.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        super::channel(32, |output| async move {
            // One name per process, as MPRIS asks with several instances: with a fixed name the second
            // window would not have its own. `playerctl -p psytify` finds them all.
            let name = format!("org.mpris.MediaPlayer2.psytify.instance{}", std::process::id());
            serve(Builder::session(), &name, output).await;
        })
    }

    /// The service on `builder` with the name `name`, as long as whoever reads `output` is there.
    async fn serve(builder: Builder<'_>, name: &str, output: mpsc::Sender<Event>) {
        let (commands, mut called) = unbounded_channel();
        let (changes, mut changed) = unbounded_channel();
        let root = Root { commands: commands.clone() };
        let player = Player {
            commands,
            status: Status::Stopped,
            position: Duration::ZERO,
            metadata: None,
            volume: 1.0,
        };
        let Ok(connection) = builder.serve_at(PATH, root).serve_at(PATH, player).name(name).build().await
        else {
            return;
        };
        let Ok(player) = connection.object_server().interface::<_, Player>(PATH).await else { return };
        // The sender stays here too: the channel of changes does not close while the service lives.
        if output.send(Event::Ready(Handle(changes.clone()))).await.is_err() {
            return;
        }
        let _changes = changes;
        loop {
            tokio::select! {
                event = called.recv() => match event {
                    Some(event) => if output.send(event).await.is_err() { return },
                    None => return,
                },
                Some(change) = changed.recv() => apply(&player, change).await,
            }
        }
    }

    /// Applies a change to the properties and tells the system (`PropertiesChanged`), as souvlaki did:
    /// the position is not announced (the specification wants the system to read it when needed).
    async fn apply(player: &InterfaceRef<Player>, change: Change) {
        let mut iface = player.get_mut().await;
        let context = player.signal_emitter();
        let _ = match change {
            Change::Metadata(metadata) => {
                iface.metadata = Some(metadata);
                iface.metadata_changed(context).await
            }
            Change::Position(position) => {
                iface.position = position;
                Ok(())
            }
            Change::Playback(status) => {
                iface.status = status;
                iface.playback_status_changed(context).await
            }
            Change::Volume(volume) => {
                iface.volume = volume;
                iface.volume_changed(context).await
            }
        };
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use futures_util::StreamExt;
        use zbus::OwnedVariant;

        use super::*;

        #[test]
        fn system_seeks_become_signed_seconds() {
            assert!(matches!(seek(-10_000_000), Event::SeekBy(-10)));
            assert!(matches!(seek(5_400_000), Event::SeekBy(5)));
            assert!(matches!(seek(0), Event::SeekBy(0)));
        }

        #[test]
        fn a_position_before_the_start_or_past_the_end_is_ignored() {
            assert!(set_position(-1, None).is_none());
            assert!(set_position(6_000_000, Some(5_000_000)).is_none());
            let at = set_position(3_000_000, Some(5_000_000));
            assert!(matches!(at, Some(Event::SetPosition(at)) if at == Duration::from_secs(3)));
        }

        /// The real service on a private bus (`dbus-daemon` started by the test, never the session's one):
        /// a method called by the system becomes a command, what the core shows becomes a property, and
        /// when the stream is idle it gives nothing. Without `dbus-daemon` the test does nothing.
        #[tokio::test]
        async fn the_service_forwards_commands_and_shows_the_track() {
            let dir = std::env::temp_dir().join(format!("psytify-mpris-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("cartella del bus");
            let config = dir.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \
                     \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\
                     <busconfig><type>session</type><listen>unix:dir={}</listen><auth>EXTERNAL</auth>\
                     <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                     <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy>\
                     </busconfig>",
                    dir.display()
                ),
            )
            .expect("configurazione del bus");
            /// The bus stops even if the test fails.
            struct Daemon(std::process::Child, std::path::PathBuf);
            impl Drop for Daemon {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                    let _ = std::fs::remove_dir_all(&self.1);
                }
            }
            let Ok(daemon) = std::process::Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address"])
                .stdout(std::process::Stdio::piped())
                .spawn()
            else {
                eprintln!("dbus-daemon non c'è: test saltato");
                return;
            };
            let mut daemon = Daemon(daemon, dir);
            let mut address = String::new();
            {
                use std::io::BufRead;
                let stdout = daemon.0.stdout.take().expect("stdout");
                std::io::BufReader::new(stdout).read_line(&mut address).expect("address");
            }
            let address = address.trim().to_owned();
            let within = Duration::from_secs(10);
            const NAME: &str = "org.mpris.MediaPlayer2.psytify.test";

            // The stream runs in a task of its own, as in the core's host that always waits for it: the changes of
            // what is shown reach the service only while someone makes it advance.
            let builder = Builder::address(address.as_str());
            let mut service = Box::pin(crate::media::channel(32, |output| async move {
                serve(builder, NAME, output).await;
            }));
            let (forward, mut events) = tokio::sync::mpsc::unbounded_channel();
            let driver = tokio::spawn(async move {
                while let Some(event) = service.next().await {
                    if forward.send(event).is_err() {
                        break;
                    }
                }
            });
            let mut next = async || {
                tokio::time::timeout(within, events.recv()).await.expect("un evento").expect("il flusso")
            };
            let Event::Ready(handle) = next().await else { panic!("prima il collegamento") };

            let client = tokio::time::timeout(within, Builder::address(address.as_str()).build())
                .await
                .expect("il client")
                .unwrap();
            // Without the properties cache: `Position` is not announced, the system reads it when needed.
            let proxy = async |interface: &'static str| -> zbus::Proxy<'static> {
                zbus::proxy::Builder::<zbus::Proxy<'static>>::new(&client)
                    .destination(NAME)
                    .path(PATH)
                    .interface(interface)
                    .cache_properties(zbus::proxy::CacheProperties::No)
                    .build()
                    .await
                    .unwrap()
            };
            let player = proxy("org.mpris.MediaPlayer2.Player").await;
            let root = proxy("org.mpris.MediaPlayer2").await;

            // The system's commands, as souvlaki translated them: `Stop` is a pause, `Quit` and `OpenUri` do
            // nothing, the jump is in whole seconds with the sign, the volume is kept between 0 and 1.
            type Expected = fn(&Event) -> bool;
            let calls: [(&str, Expected); 6] = [
                ("Next", |event| matches!(event, Event::Next)),
                ("Previous", |event| matches!(event, Event::Previous)),
                ("Play", |event| matches!(event, Event::Play)),
                ("Pause", |event| matches!(event, Event::Pause)),
                ("Stop", |event| matches!(event, Event::Pause)),
                ("PlayPause", |event| matches!(event, Event::Toggle)),
            ];
            for (method, expected) in calls {
                let _: () = player.call(method, &()).await.unwrap();
                let event = next().await;
                assert!(expected(&event), "{method}: {event:?}");
            }
            let _: () = root.call("Quit", &()).await.unwrap();
            let _: () = player.call("OpenUri", &("https://example.invalid/a.flac",)).await.unwrap();
            let _: () = player.call("Seek", &(-10_000_000_i64)).await.unwrap();
            assert!(matches!(next().await, Event::SeekBy(-10)), "Quit e OpenUri non danno niente");
            let _: () = player.call("Seek", &(5_400_000_i64)).await.unwrap();
            assert!(matches!(next().await, Event::SeekBy(5)));
            let _: () = root.call("Raise", &()).await.unwrap();
            assert!(matches!(next().await, Event::Raise));
            player.set_property("Volume", 0.25_f64).await.unwrap();
            assert!(matches!(next().await, Event::SetVolume(volume) if (volume - 0.25).abs() < 1e-6));
            player.set_property("Volume", 1.5_f64).await.unwrap();
            assert!(matches!(next().await, Event::SetVolume(volume) if (volume - 1.0).abs() < 1e-6));

            // Who the player is.
            assert_eq!(root.get_property::<String>("Identity").await.unwrap(), "Psytify");
            for (property, value) in [("CanQuit", true), ("CanRaise", true), ("HasTrackList", false)] {
                assert_eq!(root.get_property::<bool>(property).await.unwrap(), value, "{property}");
            }
            assert!(root.get_property::<Vec<String>>("SupportedUriSchemes").await.unwrap().is_empty());
            assert!(root.get_property::<Vec<String>>("SupportedMimeTypes").await.unwrap().is_empty());
            for property in ["CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek", "CanControl"] {
                assert!(player.get_property::<bool>(property).await.unwrap(), "{property}");
            }
            for property in ["Rate", "MinimumRate", "MaximumRate"] {
                assert_eq!(player.get_property::<f64>(property).await.unwrap(), 1.0, "{property}");
            }
            assert_eq!(player.get_property::<String>("PlaybackStatus").await.unwrap(), "Stopped");
            assert_eq!(player.get_property::<f64>("Volume").await.unwrap(), 1.0);

            // What the core shows.
            let now = NowPlaying {
                title: "Titolo",
                artist: "Artista",
                album: Some("Album"),
                cover_url: Some("https://example.invalid/cover.jpg"),
                duration: Duration::from_secs(200),
                playing: true,
                position: Duration::from_secs(12),
                artwork: None,
            };
            handle.show(Some(&now));
            handle.set_volume(0.5);
            // The changes reach the service over a channel: it waits for the system to read them.
            let read = async || -> (String, i64, f64, HashMap<String, OwnedVariant>) {
                let status: String = player.get_property("PlaybackStatus").await.unwrap();
                let position: i64 = player.get_property("Position").await.unwrap();
                let volume: f64 = player.get_property("Volume").await.unwrap();
                let metadata: HashMap<String, OwnedVariant> = player.get_property("Metadata").await.unwrap();
                (status, position, volume, metadata)
            };
            let mut shown = read().await;
            for _ in 0..50 {
                if shown.0 == "Playing" && (shown.2 - 0.5).abs() < 1e-6 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                shown = read().await;
            }
            assert_eq!((shown.0.as_str(), shown.1, shown.2), ("Playing", 12_000_000, 0.5));
            let text = |key: &str| shown.3.get(key).and_then(|value| String::try_from(value.clone()).ok());
            assert_eq!(text("xesam:title").as_deref(), Some("Titolo"));
            assert_eq!(text("xesam:album").as_deref(), Some("Album"));
            assert_eq!(text("mpris:artUrl").as_deref(), Some("https://example.invalid/cover.jpg"));
            let artist =
                shown.3.get("xesam:artist").and_then(|value| Vec::<String>::try_from(value.clone()).ok());
            assert_eq!(artist, Some(vec!["Artista".to_owned()]));
            let length = shown.3.get("mpris:length").and_then(|value| i64::try_from(value.clone()).ok());
            assert_eq!(length, Some(200_000_000));
            let track =
                shown.3.get("mpris:trackid").and_then(|value| ObjectPath::try_from(value.clone()).ok());
            assert_eq!(track.as_ref().map(ObjectPath::as_str), Some("/"));

            // `SetPosition` beyond the end of the track does nothing; inside, it goes there.
            let track = ObjectPath::try_from("/").unwrap();
            let _: () = player.call("SetPosition", &(&track, 250_000_000_i64)).await.unwrap();
            let _: () = player.call("SetPosition", &(&track, 30_000_000_i64)).await.unwrap();
            assert!(matches!(next().await, Event::SetPosition(at) if at == Duration::from_secs(30)));

            // Stopped: state `Stopped`, position 0, the track stays.
            handle.show(None);
            for _ in 0..50 {
                shown = read().await;
                if shown.0 == "Stopped" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert_eq!((shown.0.as_str(), shown.1), ("Stopped", 0));
            assert!(shown.3.contains_key("xesam:title"));

            // Stopped, the stream gives nothing.
            assert!(
                tokio::time::timeout(Duration::from_millis(200), events.recv()).await.is_err(),
                "nessun evento senza comandi"
            );
            driver.abort();
            drop(daemon);
        }
    }
}

/// Android: the shell registers who shows the track to the system and sends back the commands (Play,
/// Pause, Next, Previous, jump) with `command`.
#[cfg(target_os = "android")]
mod phone {
    use std::sync::{Mutex, OnceLock, PoisonError};
    use std::time::Duration;

    use futures_util::Stream;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::{Event, NowPlaying};

    type Show = Box<dyn Fn(Option<&NowPlaying<'_>>) + Send + Sync>;
    type Commands = (UnboundedSender<Event>, Mutex<Option<UnboundedReceiver<Event>>>);

    static SHOW: OnceLock<Show> = OnceLock::new();
    static COMMANDS: OnceLock<Commands> = OnceLock::new();

    fn commands() -> &'static Commands {
        COMMANDS.get_or_init(|| {
            let (sender, receiver) = unbounded_channel();
            (sender, Mutex::new(Some(receiver)))
        })
    }

    /// Registers who shows the track to the system. The first call counts.
    pub fn register(show: impl Fn(Option<&NowPlaying<'_>>) + Send + Sync + 'static) {
        let _ = SHOW.set(Box::new(show));
    }

    /// A command from the system: 0 Play, 1 Pause, 2 Next, 3 Previous, 4 jump to `value` ms,
    /// 5 Play or Pause. The other codes do nothing.
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

    /// The command receiver, held by one core for its life. It goes back to its place when the stream
    /// is dropped (the core stops), so a core started later in the same process still gets the
    /// buttons, including those pressed while no core ran.
    struct Lease(Option<UnboundedReceiver<Event>>);

    impl Drop for Lease {
        fn drop(&mut self) {
            if let Some(receiver) = self.0.take() {
                *commands().1.lock().unwrap_or_else(PoisonError::into_inner) = Some(receiver);
            }
        }
    }

    /// The shell's commands. Without a registered shell (desktop with `PSYTIFY_PHONE=1`) the stream
    /// ends at once.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        super::channel(32, |output| async move {
            if SHOW.get().is_none() {
                return;
            }
            let Some(receiver) = commands().1.lock().unwrap_or_else(PoisonError::into_inner).take() else {
                return;
            };
            let mut lease = Lease(Some(receiver));
            let Some(receiver) = lease.0.as_mut() else { return };
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
    use futures_util::Stream;

    use super::{Event, NowPlaying};

    #[derive(Debug, Clone)]
    pub struct Handle;

    impl Handle {
        pub fn show(&self, _now: Option<&NowPlaying<'_>>) {}

        pub fn set_volume(&self, _volume: f32) {}
    }

    /// Here the system sends no commands: an empty stream.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        futures_util::stream::empty()
    }
}
