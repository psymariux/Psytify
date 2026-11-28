//! The reduced motion requested by the system (step 6.5b): when the desktop wants fewer animations
//! (accessibility) or when the system is in power saving. A frontend that "follows the system" reads it
//! from the settings view (`SettingsView::system_motion`) and reduces the motion.
//!
//! The signal arrives as a stream ([`events`]) that the host holds on the core's thread, like the
//! media controls: one reading at the start and then one at each real change, with no timer at all.
//! On Linux it comes from D-Bus:
//!
//! - the `org.freedesktop.portal.Settings` portal: `org.freedesktop.appearance` `reduced-motion`
//!   (1 = reduced), and as a fallback `org.gnome.desktop.interface` `enable-animations` (false =
//!   reduced) and `org.kde.kdeglobals.KDE` `AnimationDurationFactor` (0 = reduced), listening to the
//!   `SettingChanged` signal;
//! - the power profile of power-profiles-daemon, on the system bus
//!   (`org.freedesktop.UPower.PowerProfiles`, or the old name `net.hadess.PowerProfiles`):
//!   `ActiveProfile` = `power-saver`, listening to `PropertiesChanged`.
//!
//! A missing service is not an error: that source stays "no preference". On Windows and
//! Android for now the stream ends at once (no preference: BACKLOG 22 and 35).

/// What the system asks about motion. `None`: no answer (the service is not there).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemMotion {
    /// The desktop asks for reduced animations (portal, GNOME or KDE).
    pub reduced_motion: Option<bool>,
    /// The power profile is power saving.
    pub power_saver: Option<bool>,
}

/// Why the system asks for reduced motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The desktop settings (accessibility, animations off).
    ReducedMotion,
    /// Power saving.
    PowerSaver,
    Both,
}

impl SystemMotion {
    /// The system asks for reduced motion: for accessibility or for power saving.
    /// Without an answer, full motion applies.
    #[must_use]
    pub fn reduce(self) -> bool {
        self.reason().is_some()
    }

    /// Why, if it says so.
    #[must_use]
    pub fn reason(self) -> Option<Reason> {
        match (self.reduced_motion == Some(true), self.power_saver == Some(true)) {
            (true, true) => Some(Reason::Both),
            (true, false) => Some(Reason::ReducedMotion),
            (false, true) => Some(Reason::PowerSaver),
            (false, false) => None,
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::events;
#[cfg(all(target_os = "linux", test))]
use linux::watch;

/// Outside Linux the system says nothing, for now: the stream ends at once and the frontend
/// stays on full motion (Windows: BACKLOG 22; Android: BACKLOG 35).
#[cfg(not(target_os = "linux"))]
pub fn events() -> impl futures_util::Stream<Item = SystemMotion> + Send + 'static {
    futures_util::stream::empty()
}

#[cfg(target_os = "linux")]
mod linux {
    use futures_util::{Stream, StreamExt};
    use tokio::sync::mpsc;
    use zbus::fdo::PropertiesProxy;
    use zbus::names::InterfaceName;
    use zbus::zvariant::{OwnedValue, Value};
    use zbus::{CacheProperties, Connection, Proxy, ProxyBuilder};

    use super::SystemMotion;

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
    pub(super) const APPEARANCE: (&str, &str) = ("org.freedesktop.appearance", "reduced-motion");
    const GNOME: (&str, &str) = ("org.gnome.desktop.interface", "enable-animations");
    const KDE: (&str, &str) = ("org.kde.kdeglobals.KDE", "AnimationDurationFactor");
    /// power-profiles-daemon: today's name and the one from before 0.20 (name, path).
    pub(super) const POWER: [(&str, &str); 2] = [
        ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
        ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
    ];
    pub(super) const ACTIVE_PROFILE: &str = "ActiveProfile";
    const POWER_SAVER: &str = "power-saver";

    /// The readings of each source. The desktop preference is reduced if any of the three says so
    /// (a GNOME portal may not translate `enable-animations` into `reduced-motion`).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub(super) struct Sources {
        appearance: Option<bool>,
        gnome: Option<bool>,
        kde: Option<bool>,
        power_saver: Option<bool>,
    }

    impl Sources {
        /// A portal setting; those of other keys do not count. A value of an unexpected type is
        /// "no preference".
        pub(super) fn set(&mut self, namespace: &str, key: &str, value: &Value<'_>) {
            let value = unwrap_variant(value);
            match (namespace, key) {
                APPEARANCE => self.appearance = reduced_from_appearance(value),
                GNOME => self.gnome = reduced_from_gnome(value),
                KDE => self.kde = reduced_from_kde(value),
                _ => {}
            }
        }

        pub(super) fn set_profile(&mut self, value: Option<&Value<'_>>) {
            self.power_saver = value.and_then(|value| power_saver(unwrap_variant(value)));
        }

        pub(super) fn motion(self) -> SystemMotion {
            let known = [self.appearance, self.gnome, self.kde];
            let reduced_motion =
                if known.contains(&Some(true)) { Some(true) } else { known.iter().find_map(|known| *known) };
            SystemMotion { reduced_motion, power_saver: self.power_saver }
        }
    }

    /// The portal's old `Read` method puts the answer in one variant too many.
    fn unwrap_variant<'v>(value: &'v Value<'v>) -> &'v Value<'v> {
        match value {
            Value::Value(inner) => unwrap_variant(inner),
            value => value,
        }
    }

    /// `reduced-motion`: 0 no preference, 1 reduced motion.
    fn reduced_from_appearance(value: &Value<'_>) -> Option<bool> {
        match value {
            Value::U32(value) => Some(*value == 1),
            _ => None,
        }
    }

    /// `enable-animations`: animations off means reduced motion.
    fn reduced_from_gnome(value: &Value<'_>) -> Option<bool> {
        match value {
            Value::Bool(enabled) => Some(!enabled),
            _ => None,
        }
    }

    /// `AnimationDurationFactor`: 0 means instant animations. KDE's portal gives it as a
    /// number or as text, depending on the version.
    fn reduced_from_kde(value: &Value<'_>) -> Option<bool> {
        let factor = match value {
            Value::F64(factor) => *factor,
            Value::Str(text) => text.as_str().trim().parse().ok()?,
            _ => return None,
        };
        Some(factor <= 0.0)
    }

    fn power_saver(value: &Value<'_>) -> Option<bool> {
        match value {
            Value::Str(profile) => Some(profile.as_str() == POWER_SAVER),
            _ => None,
        }
    }

    /// Connects to the buses and sends the readings: the portal is on the session bus, power-profiles-daemon on
    /// the system one. Without either of them the stream ends at once.
    pub fn events() -> impl Stream<Item = SystemMotion> + Send + 'static {
        crate::media::channel(4, |output| async move {
            let session = Connection::session().await.ok();
            let system = Connection::system().await.ok();
            if session.is_some() || system.is_some() {
                run(session, system, output).await;
            }
        })
    }

    /// The readings from an already open link, for both sources (the tests use a private bus with
    /// fake services).
    #[cfg(test)]
    pub(super) fn watch(connection: Connection) -> impl Stream<Item = SystemMotion> + Send + 'static {
        crate::media::channel(4, |output| run(Some(connection.clone()), Some(connection), output))
    }

    /// Reads the sources, sends the first reading and then one at each real change. It only waits for the signals:
    /// idle, it never wakes up. It ends when nobody reads any more or all the signals have ended.
    async fn run(
        session: Option<Connection>,
        system: Option<Connection>,
        output: mpsc::Sender<SystemMotion>,
    ) {
        let mut sources = Sources::default();
        let portal = match &session {
            Some(session) => proxy(session, PORTAL, PORTAL_PATH, PORTAL_SETTINGS).await,
            None => None,
        };
        // Subscribe before reading: a change in between is not lost.
        let mut settings = match &portal {
            Some(portal) => portal.receive_signal("SettingChanged").await.ok(),
            None => None,
        };
        if let Some(portal) = &portal {
            for (namespace, key) in [APPEARANCE, GNOME, KDE] {
                if let Some(value) = read_setting(portal, namespace, key).await {
                    sources.set(namespace, key, &value);
                }
            }
        }
        let power = match &system {
            Some(system) => power_profiles(system, &mut sources).await,
            None => None,
        };
        let (mut profiles, interface) = match power {
            Some((proxy, interface, stream)) => (Some((proxy, stream)), Some(interface)),
            None => (None, None),
        };
        let mut sent = sources.motion();
        if output.send(sent).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                Some(message) = next(&mut settings) => {
                    if let Ok((namespace, key, value)) = message.body::<(String, String, OwnedValue)>() {
                        sources.set(&namespace, &key, &value);
                    }
                }
                Some(changed) = next_profile(&mut profiles) => {
                    let Some(interface) = &interface else { continue };
                    let Ok(args) = changed.args() else { continue };
                    if args.interface_name().as_str() != interface.as_str() {
                        continue;
                    }
                    if let Some(value) = args.changed_properties().get(ACTIVE_PROFILE) {
                        sources.set_profile(Some(value));
                    } else if args.invalidated_properties().contains(&ACTIVE_PROFILE)
                        && let Some((proxy, _)) = &profiles
                    {
                        let value = proxy.get(interface.as_ref(), ACTIVE_PROFILE).await.ok();
                        sources.set_profile(value.as_deref());
                    }
                }
                else => return,
            }
            let now = sources.motion();
            if now != sent {
                sent = now;
                if output.send(now).await.is_err() {
                    return;
                }
            }
        }
    }

    type ProfileStream = zbus::fdo::PropertiesChangedStream<'static>;

    /// The next signal of a stream; a finished stream is removed and no longer waited for.
    async fn next(stream: &mut Option<zbus::SignalStream<'static>>) -> Option<std::sync::Arc<zbus::Message>> {
        let Some(inner) = stream else { return std::future::pending().await };
        let item = inner.next().await;
        if item.is_none() {
            *stream = None;
        }
        item
    }

    async fn next_profile(
        stream: &mut Option<(PropertiesProxy<'static>, ProfileStream)>,
    ) -> Option<zbus::fdo::PropertiesChanged> {
        let Some((_, inner)) = stream else { return std::future::pending().await };
        let item = inner.next().await;
        if item.is_none() {
            *stream = None;
        }
        item
    }

    /// A proxy without a property cache: no background work.
    async fn proxy(
        connection: &Connection,
        name: &'static str,
        path: &'static str,
        interface: &'static str,
    ) -> Option<Proxy<'static>> {
        ProxyBuilder::new_bare(connection)
            .destination(name)
            .ok()?
            .path(path)
            .ok()?
            .interface(interface)
            .ok()?
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .ok()
    }

    /// A portal setting: `ReadOne` (portal 2) or, if it is missing, the old `Read`.
    async fn read_setting(portal: &Proxy<'_>, namespace: &str, key: &str) -> Option<OwnedValue> {
        match portal.call::<_, _, OwnedValue>("ReadOne", &(namespace, key)).await {
            Ok(value) => Some(value),
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                portal.call::<_, _, OwnedValue>("Read", &(namespace, key)).await.ok()
            }
            Err(_) => None,
        }
    }

    /// The active power profile, from the first of the two names that answers, and the stream of its
    /// changes. Neither of them: nothing (no preference).
    async fn power_profiles(
        connection: &Connection,
        sources: &mut Sources,
    ) -> Option<(PropertiesProxy<'static>, InterfaceName<'static>, ProfileStream)> {
        for (name, path) in POWER {
            let Ok(builder) = PropertiesProxy::builder(connection).destination(name) else { continue };
            let Ok(builder) = builder.path(path) else { continue };
            let Ok(proxy) = builder.cache_properties(CacheProperties::No).build().await else { continue };
            let Ok(interface) = InterfaceName::try_from(name) else { continue };
            let Ok(stream) = proxy.receive_properties_changed().await else { continue };
            let Ok(value) = proxy.get(interface.as_ref(), ACTIVE_PROFILE).await else { continue };
            sources.set_profile(Some(&value));
            return Some((proxy, interface, stream));
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn set(sources: &mut Sources, (namespace, key): (&str, &str), value: Value<'_>) {
            sources.set(namespace, key, &value);
        }

        #[test]
        fn each_desktop_source_is_read_with_its_own_meaning() {
            let mut sources = Sources::default();
            assert_eq!(sources.motion(), SystemMotion::default(), "nessuna fonte, nessuna preferenza");
            set(&mut sources, APPEARANCE, Value::U32(0));
            assert_eq!(sources.motion().reduced_motion, Some(false));
            set(&mut sources, APPEARANCE, Value::Value(Box::new(Value::U32(1))));
            assert_eq!(sources.motion().reduced_motion, Some(true), "la variante doppia di `Read`");
            set(&mut sources, APPEARANCE, Value::Str("1".into()));
            assert_eq!(sources.motion().reduced_motion, None, "un tipo inatteso non è una risposta");

            set(&mut sources, GNOME, Value::Bool(false));
            assert_eq!(sources.motion().reduced_motion, Some(true), "animazioni spente su GNOME");
            set(&mut sources, GNOME, Value::Bool(true));
            set(&mut sources, KDE, Value::F64(0.0));
            assert_eq!(sources.motion().reduced_motion, Some(true), "fattore 0 su KDE");
            set(&mut sources, KDE, Value::Str("1.0".into()));
            assert_eq!(sources.motion().reduced_motion, Some(false));
            set(&mut sources, ("org.example", "reduced-motion"), Value::U32(1));
            assert_eq!(sources.motion().reduced_motion, Some(false), "un'altra chiave non conta");
        }

        #[test]
        fn only_the_power_saver_profile_asks_for_less_motion() {
            let mut sources = Sources::default();
            for (profile, saver) in [("power-saver", true), ("balanced", false), ("performance", false)] {
                sources.set_profile(Some(&Value::Str(profile.into())));
                assert_eq!(sources.motion().power_saver, Some(saver), "{profile}");
            }
            sources.set_profile(None);
            assert_eq!(sources.motion().power_saver, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_asks_for_less_motion_for_accessibility_or_power_saving() {
        let motion = |reduced_motion, power_saver| SystemMotion { reduced_motion, power_saver };
        assert!(!SystemMotion::default().reduce(), "nessuna risposta: movimento pieno");
        assert_eq!(motion(Some(false), Some(false)).reason(), None);
        assert_eq!(motion(Some(true), None).reason(), Some(Reason::ReducedMotion));
        assert_eq!(motion(None, Some(true)).reason(), Some(Reason::PowerSaver));
        assert_eq!(motion(Some(true), Some(true)).reason(), Some(Reason::Both));
        assert!(motion(Some(false), Some(true)).reduce());
        assert!(!motion(Some(false), None).reduce());
    }

    /// The real link against fake services on a private bus (`dbus-daemon` started by the
    /// test, never the session's one): the first reading and then the live switch to power saving
    /// and to reduced motion, and back. Without `dbus-daemon` the test does nothing.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_watcher_follows_the_fake_services_live() {
        use std::time::Duration;

        use futures_util::StreamExt;
        use zbus::zvariant::{OwnedValue, Value};
        use zbus::{ConnectionBuilder, SignalContext, dbus_interface};

        struct FakeSettings {
            reduced: u32,
        }

        #[dbus_interface(name = "org.freedesktop.portal.Settings")]
        impl FakeSettings {
            #[dbus_interface(name = "ReadOne")]
            fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
                if (namespace, key) == linux::APPEARANCE {
                    Ok(Value::U32(self.reduced).into())
                } else {
                    Err(zbus::fdo::Error::Failed("not found".into()))
                }
            }

            #[dbus_interface(signal, name = "SettingChanged")]
            async fn setting_changed(
                ctxt: &SignalContext<'_>,
                namespace: &str,
                key: &str,
                value: Value<'_>,
            ) -> zbus::Result<()>;
        }

        struct FakeProfiles {
            active: String,
        }

        #[dbus_interface(name = "org.freedesktop.UPower.PowerProfiles")]
        impl FakeProfiles {
            #[dbus_interface(property, name = "ActiveProfile")]
            fn active_profile(&self) -> String {
                self.active.clone()
            }
        }

        // A bus of its own: no services folder, so nothing starts by itself.
        let dir = std::env::temp_dir().join(format!("psytify-motion-test-{}", std::process::id()));
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
        let (power_name, power_path) = linux::POWER[0];
        // A bus that does not answer does not block the test: each step has its own deadline.
        let within = Duration::from_secs(10);
        let service = ConnectionBuilder::address(address.as_str())
            .unwrap()
            .name(PORTAL_NAME)
            .unwrap()
            .name(power_name)
            .unwrap()
            .serve_at(PORTAL_PATH_FOR_TEST, FakeSettings { reduced: 0 })
            .unwrap()
            .serve_at(power_path, FakeProfiles { active: "performance".into() })
            .unwrap()
            .build();
        let service = tokio::time::timeout(within, service).await.expect("il servizio finto").unwrap();
        let client = ConnectionBuilder::address(address.as_str()).unwrap().build();
        let client = tokio::time::timeout(within, client).await.expect("il client").unwrap();
        let mut readings = Box::pin(watch(client));
        let mut next = async || {
            tokio::time::timeout(Duration::from_secs(5), readings.next())
                .await
                .expect("una lettura")
                .expect("il flusso")
        };

        let full = SystemMotion { reduced_motion: Some(false), power_saver: Some(false) };
        assert_eq!(next().await, full, "la prima lettura");

        // Power saving, live.
        let profiles = service.object_server().interface::<_, FakeProfiles>(power_path).await.unwrap();
        profiles.get_mut().await.active = "power-saver".into();
        profiles.get().await.active_profile_changed(profiles.signal_context()).await.unwrap();
        assert_eq!(next().await, SystemMotion { power_saver: Some(true), ..full });

        // The desktop's reduced motion, live; a signal of another key sends nothing.
        let settings =
            service.object_server().interface::<_, FakeSettings>(PORTAL_PATH_FOR_TEST).await.unwrap();
        let ctxt = settings.signal_context();
        FakeSettings::setting_changed(ctxt, "org.example", "other", Value::U32(1)).await.unwrap();
        FakeSettings::setting_changed(ctxt, linux::APPEARANCE.0, linux::APPEARANCE.1, Value::U32(1))
            .await
            .unwrap();
        assert_eq!(next().await, SystemMotion { reduced_motion: Some(true), power_saver: Some(true) });

        // And back to full motion.
        profiles.get_mut().await.active = "balanced".into();
        profiles.get().await.active_profile_changed(profiles.signal_context()).await.unwrap();
        FakeSettings::setting_changed(ctxt, linux::APPEARANCE.0, linux::APPEARANCE.1, Value::U32(0))
            .await
            .unwrap();
        assert_eq!(next().await, SystemMotion { reduced_motion: Some(true), power_saver: Some(false) });
        assert_eq!(next().await, full);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), readings.next()).await.is_err(),
            "fermo, nessuna lettura in più"
        );

        drop(readings);
        drop(service);
        drop(daemon);
    }

    /// The real buses of whoever runs the test, read only: it prints the first reading. By hand
    /// (`cargo test -p psytify-core -- --ignored reads_the_real_buses --nocapture`), never in CI.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "legge i bus veri della sessione e del sistema"]
    async fn reads_the_real_buses() {
        use futures_util::StreamExt;
        let mut readings = Box::pin(events());
        let first = tokio::time::timeout(std::time::Duration::from_secs(10), readings.next()).await;
        eprintln!("prima lettura: {first:?}");
        assert!(first.is_ok(), "una risposta entro 10 s");
    }

    #[cfg(target_os = "linux")]
    const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
    #[cfg(target_os = "linux")]
    const PORTAL_PATH_FOR_TEST: &str = "/org/freedesktop/portal/desktop";
}
