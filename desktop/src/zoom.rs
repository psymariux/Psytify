//! The UI zoom preference (desktop v2 §1): `Auto` from the window width, or a pinned preset. A frontend
//! preference in `config.json` (`SettingsCmd::SaveUiPrefs`, key [`ZOOM_KEY`]): `"auto"` or a number.

use psytify_design::tokens::zoom::{self, PRESETS};
use serde_json::{Map, Value};

pub const ZOOM_KEY: &str = "zoom";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ZoomPref {
    #[default]
    Auto,
    Fixed(f32),
}

impl ZoomPref {
    /// The choices in menu order: `Auto`, then the presets.
    pub fn options() -> Vec<Self> {
        std::iter::once(Self::Auto).chain(PRESETS.iter().map(|z| Self::Fixed(*z))).collect()
    }

    /// The saved preference; an unknown value is `Auto`.
    pub fn read(ui: &Map<String, Value>) -> Self {
        match ui.get(ZOOM_KEY).and_then(Value::as_f64) {
            // Stored values are presets, a handful of short decimals: `f32` is exact enough.
            #[allow(clippy::cast_possible_truncation)]
            Some(value) => {
                let value = value as f32;
                PRESETS
                    .iter()
                    .find(|preset| (**preset - value).abs() < 1e-3)
                    .map_or(Self::Auto, |z| Self::Fixed(*z))
            }
            None => Self::Auto,
        }
    }

    /// The keys to save for this choice.
    pub fn prefs(self) -> Map<String, Value> {
        let value = match self {
            Self::Auto => Value::String("auto".to_owned()),
            Self::Fixed(z) => serde_json::Number::from_f64(f64::from(z)).map_or(Value::Null, Value::Number),
        };
        let mut prefs = Map::new();
        prefs.insert(ZOOM_KEY.to_owned(), value);
        prefs
    }

    pub fn label(self) -> String {
        match self {
            Self::Auto => "Automatica".to_owned(),
            // A percentage of a preset: small and positive.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Self::Fixed(z) => format!("{} %", (z * 100.0 + 1e-3).floor() as u32),
        }
    }

    /// The zoom in effect for a window `width` logical px wide, with `current` in effect now.
    pub fn effective(self, width: f32, current: f32) -> f32 {
        match self {
            Self::Auto => zoom::auto_zoom(width, current),
            Self::Fixed(z) => z,
        }
    }

    /// Ctrl+= and Ctrl+-: the next preset from the zoom in effect.
    pub fn stepped(current: f32, up: bool) -> Self {
        Self::Fixed(if up { zoom::zoom_in(current) } else { zoom::zoom_out(current) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preference_round_trips_and_unknown_values_are_auto() {
        for pref in ZoomPref::options() {
            let mut ui = Map::new();
            ui.extend(pref.prefs());
            assert_eq!(ZoomPref::read(&ui), pref);
        }
        let mut ui = Map::new();
        assert_eq!(ZoomPref::read(&ui), ZoomPref::Auto);
        ui.insert(ZOOM_KEY.into(), Value::from(1.33));
        assert_eq!(ZoomPref::read(&ui), ZoomPref::Auto, "not a preset");
        assert_eq!(ZoomPref::Fixed(1.125).label(), "112 %");
        assert_eq!(ZoomPref::Fixed(0.9).label(), "90 %");
    }

    #[test]
    fn auto_follows_the_window_and_a_pinned_zoom_does_not() {
        assert_eq!(ZoomPref::Auto.effective(2560.0, 1.0), 1.5);
        assert_eq!(ZoomPref::Fixed(1.25).effective(1280.0, 1.0), 1.25);
        assert_eq!(ZoomPref::stepped(1.0, true), ZoomPref::Fixed(1.125));
        assert_eq!(ZoomPref::stepped(1.0, false), ZoomPref::Fixed(0.9));
    }
}
