//! «Impostazioni», as the GPUI page (`desktop/src/app/settings_page.rs`) without what a phone does not
//! have (Discord, the server, the shortcuts): listening quality, autoplay, the sleep timer, «Riduci il
//! movimento», the equalizer with its presets and ten bands, the quality and the folder of the
//! downloads, the account, the version with the app's update, credits and licences. The full
//! player's «Copertina» / «Video» choice is a preference of this frontend too ([`CLIP_KEY`]).
//!
//! Every choice is a command the core already had: quality, autoplay, sleep timer and equalizer are
//! `PlaybackCmd`, the download quality `DownloadsCmd`, the activity sharing `SocialCmd::SaveProfile`,
//! the motion a preference of this frontend (`SettingsCmd::SaveUiPrefs`, the desktop's key, read
//! back from `SettingsView::ui`). The app draws a [`SettingsSnapshot`], told only when it changes.
//!
//! On Android the core reads nothing of the system's motion (`SystemMotion` has no source here): the
//! app passes what Android says (animations off, battery saver) to [`reduce_motion`].

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::update::BUILD_TAG;
use psytify_core::api::{DownloadQuality, Quality};
use psytify_core::command::{DownloadsCmd, PlaybackCmd, SettingsCmd, SleepChoice, SocialCmd, UpdateCheck};
use psytify_core::player::eq::{self, PRESETS};
use psytify_core::snapshot::Account;
use psytify_core::system_motion::{Reason, SystemMotion};
use serde_json::{Map, Value};

use crate::format;
use crate::update::UpdateLine;

/// The motion preference among the frontend keys of `config.json` (the desktop's key and values).
pub(crate) const MOTION_KEY: &str = "motion";
/// The older switch (`true` is «Ridotto»), read when the new key is missing.
const LEGACY_REDUCE_MOTION_KEY: &str = "reduce_motion";
/// «Video» on the full player's media slot: the clip shows whenever the track has one.
pub(crate) const CLIP_KEY: &str = "phone_clip";

/// Band labels, as an equalizer is read.
const BAND_LABELS: [&str; eq::BANDS] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];

/// Credits and rights: title and text.
const CREDITS: [(&str, &str); 4] = [
    ("Caratteri", "Inter e Departure Mono, con licenza SIL Open Font License 1.1."),
    ("Icone e Nastro", "Disegnati per Psytify sulla griglia dei pixel d'arte."),
    (
        "Marchi di accesso",
        "GitHub, Google e Discord da Simple Icons (CC0); i marchi restano dei loro titolari.",
    ),
    (
        "Diritti",
        "Ogni brano del catalogo ha fonte e licenza documentate dal server: si scarica solo se la licenza lo permette.",
    ),
];

/// «Riduci il movimento».
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum MotionChoice {
    /// Android's «Rimuovi animazioni» and the battery saver decide.
    #[default]
    System,
    Reduce,
    Full,
}

impl MotionChoice {
    const ALL: [Self; 3] = [Self::System, Self::Reduce, Self::Full];

    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Reduce => "reduce",
            Self::Full => "full",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "Segui il sistema",
            Self::Reduce => "Ridotto",
            Self::Full => "Completo",
        }
    }

    /// The saved choice: the new key if known, else the old switch, else «Segui il sistema».
    pub(crate) fn read(ui: &Map<String, Value>) -> Self {
        let chosen = ui
            .get(MOTION_KEY)
            .and_then(Value::as_str)
            .and_then(|key| Self::ALL.into_iter().find(|choice| choice.key() == key));
        chosen.unwrap_or(if ui.get(LEGACY_REDUCE_MOTION_KEY).and_then(Value::as_bool) == Some(true) {
            Self::Reduce
        } else {
            Self::System
        })
    }

    /// The keys to save for this choice: the new one, and the old one removed.
    fn prefs(self) -> Map<String, Value> {
        let mut prefs = Map::new();
        prefs.insert(MOTION_KEY.to_owned(), Value::String(self.key().to_owned()));
        prefs.insert(LEGACY_REDUCE_MOTION_KEY.to_owned(), Value::Null);
        prefs
    }
}

fn system(animations_off: bool, power_saver: bool) -> SystemMotion {
    SystemMotion { reduced_motion: Some(animations_off), power_saver: Some(power_saver) }
}

/// Whether the app stops its animations: «Ridotto» always, «Completo» never, «Segui il sistema» when
/// Android's animations are off or the battery saver is on.
#[uniffi::export]
pub fn reduce_motion(choice: MotionChoice, animations_off: bool, power_saver: bool) -> bool {
    match choice {
        MotionChoice::System => system(animations_off, power_saver).reduce(),
        MotionChoice::Reduce => true,
        MotionChoice::Full => false,
    }
}

/// The line under «Riduci il movimento»: following the system, what it asks right now.
#[uniffi::export]
pub fn motion_detail(choice: MotionChoice, animations_off: bool, power_saver: bool) -> String {
    if choice != MotionChoice::System {
        return "Gli sprite restano fermi e i pannelli compaiono senza scorrere.".to_owned();
    }
    match system(animations_off, power_saver).reason() {
        None => "Ora il sistema chiede movimento pieno.",
        Some(Reason::ReducedMotion) => "Ora il sistema chiede movimento ridotto (animazioni rimosse).",
        Some(Reason::PowerSaver) => "Ora il sistema chiede movimento ridotto (risparmio energetico).",
        Some(Reason::Both) => {
            "Ora il sistema chiede movimento ridotto (animazioni rimosse e risparmio energetico)."
        }
    }
    .to_owned()
}

/// A choice among a few, drawn as a select.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Select {
    pub options: Vec<String>,
    /// `None` when no option matches (the equalizer's bands changed by hand).
    pub chosen: Option<u32>,
}

/// One band of the equalizer.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Band {
    /// «31», «1k».
    pub label: String,
    pub gain_db: f32,
    /// «+3», «−6», «0».
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AccountLine {
    Unavailable,
    SignedOut,
    Working,
    /// «Nome · @utente».
    SignedIn {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Credit {
    pub title: String,
    pub text: String,
}

/// What the settings page shows.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingsSnapshot {
    pub quality: Select,
    pub quality_detail: String,
    pub autoplay: bool,
    pub sleep: Select,
    pub motion: MotionChoice,
    pub motion_options: Vec<String>,
    pub eq_enabled: bool,
    /// The presets, «Personalizzato» when none matches.
    pub eq_preset: Select,
    pub bands: Vec<Band>,
    pub max_gain_db: f32,
    pub download_quality: Select,
    pub download_detail: String,
    /// «I brani scaricati arrivano in …».
    pub folder: String,
    /// «3 brani · 42 MB, suonano anche senza rete. Liberi sul telefono: 12 GB».
    pub space: String,
    pub account: AccountLine,
    /// «Condividi cosa ascolti», with a profile.
    pub share_activity: Option<bool>,
    pub version: String,
    /// The app's update: the last check, the offer, the download or the package to install.
    pub update: UpdateLine,
    /// The full player shows the clip in place of the cover (saved across launches).
    pub clip: bool,
    pub credits: Vec<Credit>,
}

/// «+3», «−6» or «0» (the desktop's).
fn gain_label(gain: f32) -> String {
    if gain.abs() < 0.5 { "0".into() } else { format!("{gain:+.0}").replace('-', "−") }
}

fn quality_detail(quality: Quality) -> &'static str {
    match quality {
        Quality::Lossless => {
            "FLAC o WAV senza perdita da Internet Archive. Se un brano non ce l'ha, suona la versione standard."
        }
        Quality::Standard => {
            "Versione compressa (m4a) da YouTube, più leggera. Se un brano non ce l'ha, suona la lossless."
        }
    }
}

const QUALITIES: [Quality; 2] = [Quality::Lossless, Quality::Standard];

fn index(at: Option<usize>) -> Option<u32> {
    at.and_then(|at| u32::try_from(at).ok())
}

impl SettingsSnapshot {
    /// `free` is the space left in the downloads folder, read when the downloads changed.
    pub(crate) fn of(snapshot: &Snapshot, free: Option<u64>) -> Self {
        let (playback, downloads, social) = (&snapshot.playback, &snapshot.downloads, &snapshot.social);
        let sleep_at = playback
            .sleep
            .and_then(|timer| SleepChoice::ALL.iter().position(|known| *known == timer.choice))
            .map_or(0, |at| at + 1);
        let sleep =
            std::iter::once("Spento".to_owned()).chain(SleepChoice::ALL.iter().map(|c| c.label())).collect();
        let gains = playback.equalizer.gains;
        let preset = PRESETS.iter().position(|(_, preset)| *preset == gains);
        let mut presets: Vec<String> = PRESETS.iter().map(|(name, _)| (*name).to_owned()).collect();
        if preset.is_none() {
            presets.push("Personalizzato".to_owned());
        }
        let custom = presets.len() - 1;
        let mut space = if downloads.saved.is_empty() {
            "Nessun brano sul dispositivo.".to_owned()
        } else {
            format!(
                "{} · {}, suonano anche senza rete.",
                format::tracks(downloads.saved.len()),
                format::size(downloads.used_bytes)
            )
        };
        if let Some(free) = free {
            space.push_str(&format!(" Liberi sul telefono: {}.", format::size(free)));
        }
        let account = match &social.account {
            Account::Unavailable => AccountLine::Unavailable,
            Account::SignedOut => AccountLine::SignedOut,
            Account::Working => AccountLine::Working,
            Account::SignedIn(me) => AccountLine::SignedIn {
                text: match &me.profile {
                    Some(profile) => format!("{} · @{}", profile.display_name, profile.handle),
                    None => "Nessun nome utente ancora".to_owned(),
                },
            },
        };
        Self {
            quality: Select {
                options: QUALITIES.iter().map(|q| q.label().to_owned()).collect(),
                chosen: index(QUALITIES.iter().position(|q| *q == playback.quality)),
            },
            quality_detail: quality_detail(playback.quality).to_owned(),
            autoplay: playback.autoplay,
            sleep: Select { options: sleep, chosen: index(Some(sleep_at)) },
            motion: MotionChoice::read(&snapshot.settings.ui),
            motion_options: MotionChoice::ALL.iter().map(|c| c.label().to_owned()).collect(),
            eq_enabled: playback.equalizer.enabled,
            eq_preset: Select { options: presets, chosen: index(Some(preset.unwrap_or(custom))) },
            bands: BAND_LABELS
                .iter()
                .zip(gains)
                .map(|(label, gain)| Band {
                    label: (*label).to_owned(),
                    gain_db: gain,
                    text: gain_label(gain),
                })
                .collect(),
            max_gain_db: eq::MAX_GAIN_DB,
            download_quality: Select {
                options: DownloadQuality::ALL.iter().map(|q| q.label().to_owned()).collect(),
                chosen: index(DownloadQuality::ALL.iter().position(|q| *q == downloads.quality)),
            },
            download_detail: format!(
                "{}. Dal menu «Altro» di un brano («Scarica in…») si sceglie caso per caso; se il brano non ha la \
                 qualità chiesta arriva quella più vicina.",
                downloads.quality.description()
            ),
            folder: format!(
                "I brani scaricati arrivano in {}. Disinstallando l'app se ne vanno con lei.",
                downloads.folder.display()
            ),
            space,
            account,
            share_activity: social.account.profile().map(|profile| profile.share_activity),
            // The release tag the workflow builds from, else the crate's version (a local build).
            version: format!("Versione {}", BUILD_TAG.unwrap_or(env!("CARGO_PKG_VERSION"))),
            update: UpdateLine::of(&downloads.update),
            clip: snapshot.settings.ui.get(CLIP_KEY).and_then(Value::as_bool).unwrap_or(false),
            credits: CREDITS
                .iter()
                .map(|(title, text)| Credit { title: (*title).to_owned(), text: (*text).to_owned() })
                .collect(),
        }
    }
}

/// A change on the settings page.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SettingsAction {
    Quality {
        index: u32,
    },
    Autoplay {
        on: bool,
    },
    Sleep {
        index: u32,
    },
    Motion {
        choice: MotionChoice,
    },
    EqEnabled {
        on: bool,
    },
    EqPreset {
        index: u32,
    },
    /// A band moved (while dragging); the core applies it at once.
    EqBand {
        band: u32,
        gain_db: f32,
    },
    /// The drag ended: the equalizer is saved.
    EqCommit,
    DownloadQuality {
        index: u32,
    },
    ShareActivity {
        on: bool,
    },
    SignOut,
    /// «Copertina» (`false`) or «Video» on the full player.
    Clip {
        on: bool,
    },
    /// «Controlla ora».
    CheckUpdate,
    /// «Scarica» the offered package.
    DownloadUpdate,
    /// The offer or the ready package put aside until the next release.
    DismissUpdate,
}

/// The commands of `action` and the notice that goes with it, if any.
pub(crate) fn apply(action: &SettingsAction, snapshot: &Snapshot) -> (Vec<Command>, Option<String>) {
    let playback = |command| vec![Command::Playback(command)];
    let at = |index: u32| index as usize;
    match *action {
        SettingsAction::Quality { index } => match QUALITIES.get(at(index)) {
            Some(quality) => (playback(PlaybackCmd::Quality(*quality)), None),
            None => (Vec::new(), None),
        },
        SettingsAction::Autoplay { on } => (playback(PlaybackCmd::Autoplay(on)), None),
        SettingsAction::Sleep { index } => {
            let choice = at(index).checked_sub(1).and_then(|at| SleepChoice::ALL.get(at)).copied();
            if index > 0 && choice.is_none() {
                return (Vec::new(), None);
            }
            let notice = match choice {
                Some(choice) => format!("Timer di spegnimento: {}", choice.label().to_lowercase()),
                None => "Timer di spegnimento disattivato".to_owned(),
            };
            (playback(PlaybackCmd::Sleep(choice)), Some(notice))
        }
        SettingsAction::Motion { choice } => {
            (vec![Command::Settings(SettingsCmd::SaveUiPrefs(choice.prefs()))], None)
        }
        SettingsAction::EqEnabled { on } => (playback(PlaybackCmd::EqualizerEnabled(on)), None),
        SettingsAction::EqPreset { index } => match PRESETS.get(at(index)) {
            Some((_, gains)) => (
                vec![
                    Command::Playback(PlaybackCmd::EqualizerGains(*gains)),
                    Command::Playback(PlaybackCmd::Save),
                ],
                None,
            ),
            None => (Vec::new(), None),
        },
        SettingsAction::EqBand { band, gain_db } => {
            if at(band) >= eq::BANDS || !gain_db.is_finite() {
                return (Vec::new(), None);
            }
            // Half-decibel steps, as the desktop's sliders.
            let gain = ((gain_db * 2.0).round() / 2.0).clamp(-eq::MAX_GAIN_DB, eq::MAX_GAIN_DB);
            (playback(PlaybackCmd::EqualizerBand { band: at(band), gain }), None)
        }
        SettingsAction::EqCommit => (playback(PlaybackCmd::Save), None),
        SettingsAction::DownloadQuality { index } => match DownloadQuality::ALL.get(at(index)) {
            Some(quality) => (vec![Command::Downloads(DownloadsCmd::SetQuality(*quality))], None),
            None => (Vec::new(), None),
        },
        SettingsAction::ShareActivity { on } => match snapshot.social.account.profile() {
            Some(profile) => {
                let save = SocialCmd::SaveProfile {
                    handle: profile.handle.clone(),
                    display_name: profile.display_name.clone(),
                    share_activity: Some(on),
                };
                (vec![Command::Social(save)], None)
            }
            None => (Vec::new(), None),
        },
        SettingsAction::SignOut => (vec![Command::Social(SocialCmd::SignOut)], None),
        SettingsAction::Clip { on } => {
            let mut prefs = Map::new();
            prefs.insert(CLIP_KEY.to_owned(), Value::Bool(on));
            (vec![Command::Settings(SettingsCmd::SaveUiPrefs(prefs))], None)
        }
        SettingsAction::CheckUpdate => {
            (vec![Command::Downloads(DownloadsCmd::CheckUpdate(UpdateCheck::Manual))], None)
        }
        SettingsAction::DownloadUpdate => (vec![Command::Downloads(DownloadsCmd::DownloadUpdate)], None),
        SettingsAction::DismissUpdate => (vec![Command::Downloads(DownloadsCmd::DismissUpdate)], None),
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::config::Config;
    use psytify_core::testing::LocalCore;

    use super::*;

    fn send(core: &mut LocalCore, action: SettingsAction) -> Option<String> {
        let (commands, notice) = apply(&action, &core.snapshot());
        for command in commands {
            core.send(command);
        }
        notice
    }

    #[test]
    fn the_choices_reach_the_core_and_come_back_in_the_snapshot() {
        let mut core = LocalCore::new(Config::default(), 1);
        let shown = SettingsSnapshot::of(&core.snapshot(), None);
        assert_eq!(shown.quality.options, ["Lossless", "Standard"]);
        assert_eq!((shown.sleep.chosen, shown.motion), (Some(0), MotionChoice::System));
        assert_eq!(shown.bands.len(), eq::BANDS);

        send(&mut core, SettingsAction::Quality { index: 1 });
        send(&mut core, SettingsAction::Autoplay { on: !shown.autoplay });
        send(&mut core, SettingsAction::DownloadQuality { index: 3 });
        send(&mut core, SettingsAction::EqEnabled { on: true });
        send(&mut core, SettingsAction::EqPreset { index: 1 });
        let notice = send(&mut core, SettingsAction::Sleep { index: 3 });
        assert_eq!(notice.as_deref(), Some("Timer di spegnimento: 30 minuti"));
        send(&mut core, SettingsAction::Motion { choice: MotionChoice::Reduce });
        assert!(!shown.clip, "the cover until the person picks «Video»");
        send(&mut core, SettingsAction::Clip { on: true });

        let shown = SettingsSnapshot::of(&core.snapshot(), Some(1 << 30));
        assert_eq!(shown.quality.chosen, Some(1));
        assert_eq!(shown.download_quality.chosen, Some(3));
        assert!(shown.eq_enabled);
        assert_eq!((shown.eq_preset.chosen, shown.bands[0].text.as_str()), (Some(1), "+6"));
        assert_eq!(shown.sleep.chosen, Some(3));
        assert_eq!(shown.motion, MotionChoice::Reduce);
        assert!(shown.clip);
        assert!(shown.space.contains("Liberi sul telefono"), "{}", shown.space);

        send(&mut core, SettingsAction::EqBand { band: 0, gain_db: -3.3 });
        let shown = SettingsSnapshot::of(&core.snapshot(), None);
        assert_eq!(shown.bands[0].gain_db, -3.5, "half-decibel steps");
        assert_eq!(shown.eq_preset.options.last().map(String::as_str), Some("Personalizzato"));
        assert_eq!(shown.eq_preset.chosen, Some(u32::try_from(PRESETS.len()).unwrap_or(0)));
        assert!(apply(&SettingsAction::EqBand { band: 10, gain_db: 0.0 }, &core.snapshot()).0.is_empty());
        assert!(apply(&SettingsAction::Sleep { index: 99 }, &core.snapshot()).0.is_empty());
    }

    #[test]
    fn motion_follows_the_choice_and_the_system() {
        assert!(!reduce_motion(MotionChoice::System, false, false));
        assert!(reduce_motion(MotionChoice::System, true, false));
        assert!(reduce_motion(MotionChoice::System, false, true));
        assert!(reduce_motion(MotionChoice::Reduce, false, false));
        assert!(!reduce_motion(MotionChoice::Full, true, true));
        assert!(motion_detail(MotionChoice::System, false, true).contains("risparmio"));
        let mut ui = Map::new();
        ui.insert(LEGACY_REDUCE_MOTION_KEY.into(), Value::Bool(true));
        assert_eq!(MotionChoice::read(&ui), MotionChoice::Reduce);
        ui.extend(MotionChoice::Full.prefs());
        assert_eq!(MotionChoice::read(&ui), MotionChoice::Full);
    }
}
