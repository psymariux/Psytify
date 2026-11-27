//! Controlli multimediali del sistema (MPRIS su Linux, la sessione multimediale su Android): i tasti
//! multimediali della tastiera, il widget del desktop, la schermata di blocco del telefono e le
//! cuffie comandano il player, e il sistema mostra titolo, artista, copertina e posizione. Su
//! Android il lavoro con Java lo fa il guscio (`android/src/media.rs`), agganciato con
//! `set_phone_media`. Su Windows e macOS per ora non c'è: servirebbe la finestra nativa (HWND), e lì
//! `events` non produce nulla.
//!
//! I comandi arrivano come un flusso neutro ([`events`]), senza framework: Iced lo ascolta come
//! `Subscription`, il core lo potrà consumare direttamente.
#![cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    expect(dead_code, reason = "fuori da Linux i comandi del sistema non arrivano mai")
)]

use std::future::Future;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

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
    /// La copertina già scaricata (lato e RGBA), per chi la vuole in pixel e non per indirizzo
    /// (MPRIS la legge dall'indirizzo).
    pub artwork: Option<(u32, &'a [u8])>,
}

/// Il brano da mostrare al sistema, con i suoi testi: lo costruisce il riduttore (nessun prestito, può
/// viaggiare in un effetto) e l'host ne fa un [`NowPlaying`] con [`Now::borrowed`].
#[derive(Debug, Clone)]
pub struct Now {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration: Duration,
    pub playing: bool,
    pub position: Duration,
    /// La copertina già decodificata: lato e RGBA.
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
pub use phone::{
    command as phone_command, install as phone_install, notify_update as phone_notify_update,
    register as set_phone_media, register_install as set_phone_installer,
    register_update_notifier as set_phone_update_notifier,
};

/// Un flusso scritto da un compito: `run` riceve il capo che scrive e il flusso finisce quando `run`
/// termina e i comandi già scritti sono stati letti. Fa ciò che faceva `iced::stream::channel`, con
/// il canale di tokio al posto di quello di Iced.
/// Lo usa anche il movimento ridotto del sistema (`system_motion`).
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
    //! MPRIS sul bus della sessione, con zbus (in Rust puro: niente libdbus da installare). Il servizio è un
    //! compito asincrono dentro il flusso di [`events`]: aspetta un metodo chiamato dal sistema o un cambio di
    //! ciò che si mostra, e senza nessuno dei due non si sveglia (souvlaki, che c'era prima, controllava il suo
    //! canale ogni 10 ms: 100 risvegli al secondo anche a riposo, BACKLOG 13). Le interfacce e le proprietà
    //! sono quelle che souvlaki esponeva, con i nomi della specifica.

    use std::collections::HashMap;
    use std::time::Duration;

    use futures_util::Stream;
    use tokio::sync::mpsc::{self, UnboundedSender, unbounded_channel};
    use zbus::zvariant::{ObjectPath, Value};
    use zbus::{ConnectionBuilder, InterfaceRef, dbus_interface};

    use super::{Event, NowPlaying};

    /// Dove stanno le due interfacce, come vuole MPRIS.
    const PATH: &str = "/org/mpris/MediaPlayer2";

    /// Il collegamento con il sistema; si clona per viaggiare nei messaggi. Ciò che si mostra va al
    /// servizio su un canale: chi lo chiama non aspetta il bus.
    #[derive(Clone)]
    pub struct Handle(UnboundedSender<Change>);

    impl std::fmt::Debug for Handle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Handle")
        }
    }

    impl Handle {
        /// Mostra il brano (o niente, a player fermo). Un errore di D-Bus non ferma la musica: il
        /// widget del desktop resta solo indietro.
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

    /// Lo stato di riproduzione che il sistema legge.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Status {
        Playing,
        Paused,
        Stopped,
    }

    /// Il brano come lo legge il sistema.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct Metadata {
        title: String,
        artist: String,
        album: Option<String>,
        cover_url: Option<String>,
        /// In microsecondi, come vuole `mpris:length`.
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

    /// Un cambio di ciò che si mostra, dal core al servizio.
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

    /// `Seek` di MPRIS (microsecondi, avanti se positivo) in secondi interi, come prima con souvlaki.
    fn seek(offset: i64) -> Event {
        let secs = i64::try_from(offset.unsigned_abs() / 1_000_000).unwrap_or(i64::MAX);
        Event::SeekBy(if offset > 0 { secs } else { -secs })
    }

    /// `SetPosition` di MPRIS: niente se la posizione è negativa o oltre la fine del brano.
    fn set_position(position: i64, length: Option<i64>) -> Option<Event> {
        let micros = u64::try_from(position).ok()?;
        if length.is_some_and(|length| position > length) {
            return None;
        }
        Some(Event::SetPosition(Duration::from_micros(micros)))
    }

    /// `org.mpris.MediaPlayer2`: chi è il player.
    struct Root {
        commands: UnboundedSender<Event>,
    }

    #[dbus_interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {
            let _ = self.commands.send(Event::Raise);
        }

        /// Chiudere l'app dal widget non c'era nemmeno prima: la chiusura passa dal salvataggio della finestra.
        fn quit(&self) {}

        #[dbus_interface(property)]
        fn can_quit(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_raise(&self) -> bool {
            true
        }

        #[dbus_interface(property, name = "HasTrackList")]
        fn has_track_list(&self) -> bool {
            false
        }

        #[dbus_interface(property)]
        fn identity(&self) -> &'static str {
            "Psytify"
        }

        #[dbus_interface(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            Vec::new()
        }

        #[dbus_interface(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            Vec::new()
        }
    }

    /// `org.mpris.MediaPlayer2.Player`: i comandi e ciò che si mostra.
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

    #[dbus_interface(name = "org.mpris.MediaPlayer2.Player")]
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

        /// Psytify non apre indirizzi dal sistema.
        fn open_uri(&self, _uri: String) {}

        #[dbus_interface(property)]
        fn playback_status(&self) -> &'static str {
            match self.status {
                Status::Playing => "Playing",
                Status::Paused => "Paused",
                Status::Stopped => "Stopped",
            }
        }

        #[dbus_interface(property)]
        fn rate(&self) -> f64 {
            1.0
        }

        #[dbus_interface(property)]
        fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
            let mut dict = HashMap::new();
            // Un id del brano serve perché il sistema possa chiedere `SetPosition`.
            if let Ok(id) = ObjectPath::try_from("/") {
                dict.insert("mpris:trackid", Value::new(id));
            }
            if let Some(metadata) = &self.metadata {
                dict.insert("mpris:length", Value::new(metadata.length));
                dict.insert("xesam:title", Value::new(metadata.title.clone()));
                dict.insert("xesam:artist", Value::new(vec![metadata.artist.clone()]));
                if let Some(album) = &metadata.album {
                    dict.insert("xesam:album", Value::new(album.clone()));
                }
                if let Some(cover) = &metadata.cover_url {
                    dict.insert("mpris:artUrl", Value::new(cover.clone()));
                }
            }
            dict
        }

        #[dbus_interface(property)]
        fn volume(&self) -> f64 {
            self.volume
        }

        #[dbus_interface(property)]
        fn set_volume(&self, volume: f64) {
            self.send(Event::SetVolume(volume.clamp(0.0, 1.0) as f32));
        }

        #[dbus_interface(property)]
        fn position(&self) -> i64 {
            if self.status == Status::Stopped { 0 } else { micros(self.position) }
        }

        #[dbus_interface(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }

        #[dbus_interface(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }

        #[dbus_interface(property)]
        fn can_go_next(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_go_previous(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_pause(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_seek(&self) -> bool {
            true
        }

        #[dbus_interface(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    /// Registra Psytify sul bus della sessione e inoltra i comandi. Senza bus (per esempio fuori
    /// da una sessione grafica) il flusso finisce subito.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        super::channel(32, |output| async move {
            // Un nome per processo, come chiede MPRIS con più istanze: con un nome fisso la seconda
            // finestra non avrebbe il suo. `playerctl -p psytify` le trova tutte.
            let name = format!("org.mpris.MediaPlayer2.psytify.instance{}", std::process::id());
            if let Ok(builder) = ConnectionBuilder::session() {
                serve(builder, &name, output).await;
            }
        })
    }

    /// Il servizio su `builder` con il nome `name`, finché chi legge `output` c'è.
    async fn serve(builder: ConnectionBuilder<'_>, name: &str, output: mpsc::Sender<Event>) {
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
        let built = async { builder.serve_at(PATH, root)?.serve_at(PATH, player)?.name(name)?.build().await };
        let Ok(connection) = built.await else { return };
        let Ok(player) = connection.object_server().interface::<_, Player>(PATH).await else { return };
        // Il capo resta anche qui: il canale dei cambi non si chiude finché il servizio vive.
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

    /// Applica un cambio alle proprietà e lo dice al sistema (`PropertiesChanged`), come faceva souvlaki:
    /// la posizione non si annuncia (la specifica vuole che il sistema la legga quando serve).
    async fn apply(player: &InterfaceRef<Player>, change: Change) {
        let mut iface = player.get_mut().await;
        let context = player.signal_context();
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
        use zbus::zvariant::OwnedValue;

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

        /// Il servizio vero su un bus privato (`dbus-daemon` lanciato dal test, mai quello della sessione):
        /// un metodo chiamato dal sistema diventa un comando, ciò che il core mostra diventa proprietà, e
        /// fermo il flusso non dà niente. Senza `dbus-daemon` il test non fa niente.
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
            /// Il bus si ferma anche se il test fallisce.
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

            // Il flusso gira in un compito suo, come nell'host del core che lo aspetta sempre: i cambi di ciò
            // che si mostra arrivano al servizio solo mentre qualcuno lo fa avanzare.
            let builder = ConnectionBuilder::address(address.as_str()).unwrap();
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

            let client =
                tokio::time::timeout(within, ConnectionBuilder::address(address.as_str()).unwrap().build())
                    .await
                    .expect("il client")
                    .unwrap();
            // Senza la cache delle proprietà: `Position` non si annuncia, il sistema la legge quando serve.
            let proxy = async |interface: &'static str| -> zbus::Proxy<'static> {
                zbus::ProxyBuilder::new_bare(&client)
                    .destination(NAME)
                    .unwrap()
                    .path(PATH)
                    .unwrap()
                    .interface(interface)
                    .unwrap()
                    .cache_properties(zbus::CacheProperties::No)
                    .build()
                    .await
                    .unwrap()
            };
            let player = proxy("org.mpris.MediaPlayer2.Player").await;
            let root = proxy("org.mpris.MediaPlayer2").await;

            // I comandi del sistema, come li traduceva souvlaki: `Stop` è una pausa, `Quit` e `OpenUri` non
            // fanno niente, il salto è in secondi interi con il segno, il volume si tiene fra 0 e 1.
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

            // Chi è il player.
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

            // Ciò che il core mostra.
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
            // I cambi arrivano al servizio su un canale: si aspetta che il sistema li legga.
            let read = async || -> (String, i64, f64, HashMap<String, OwnedValue>) {
                let status: String = player.get_property("PlaybackStatus").await.unwrap();
                let position: i64 = player.get_property("Position").await.unwrap();
                let volume: f64 = player.get_property("Volume").await.unwrap();
                let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").await.unwrap();
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

            // `SetPosition` oltre la fine del brano non fa niente; dentro, va lì.
            let track = ObjectPath::try_from("/").unwrap();
            let _: () = player.call("SetPosition", &(&track, 250_000_000_i64)).await.unwrap();
            let _: () = player.call("SetPosition", &(&track, 30_000_000_i64)).await.unwrap();
            assert!(matches!(next().await, Event::SetPosition(at) if at == Duration::from_secs(30)));

            // Fermo: stato `Stopped`, posizione 0, il brano resta.
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

            // Fermo, il flusso non dà niente.
            assert!(
                tokio::time::timeout(Duration::from_millis(200), events.recv()).await.is_err(),
                "nessun evento senza comandi"
            );
            driver.abort();
            drop(daemon);
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

    use futures_util::Stream;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::{Event, NowPlaying};

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

    /// I comandi del guscio. Senza guscio registrato (desktop con `PSYTIFY_PHONE=1`) il flusso
    /// finisce subito.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        super::channel(32, |output| async move {
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
    use futures_util::Stream;

    use super::{Event, NowPlaying};

    #[derive(Debug, Clone)]
    pub struct Handle;

    impl Handle {
        pub fn show(&self, _now: Option<&NowPlaying<'_>>) {}

        pub fn set_volume(&self, _volume: f32) {}
    }

    /// Qui il sistema non manda comandi: un flusso vuoto.
    pub fn events() -> impl Stream<Item = Event> + Send + 'static {
        futures_util::stream::empty()
    }
}
