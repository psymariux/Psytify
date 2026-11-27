//! The lyrics of the current track for the full player's «Testo». What to show comes from the
//! core's `LyricsView` (loaded only while a frontend asks, `PlaybackCmd::Lyrics`), with the desktop's
//! rules (`desktop/src/app/lyrics_view.rs`, `Shown::of`); the line timing (the current line, the dots
//! of an instrumental pause, when the next change comes) is the core's `player::lyrics`. The app only
//! scrolls and draws: it asks [`LyricsModel::cursor`] at the times the cursor itself names.

use std::sync::Arc;
use std::time::Duration;

use psytify_core::player::lyrics::{Line, Text, current, dots_lit, is_pause, next_change};
use psytify_core::snapshot::{LyricsState, PlaybackView};

use crate::playback::{count, millis};

/// A verse with the time it starts; `pause` for an instrumental gap (drawn as three dots).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LyricLine {
    pub at_ms: u64,
    pub text: String,
    pub pause: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LyricsStatus {
    /// Nothing in the queue.
    NoTrack,
    Loading,
    Failed {
        message: String,
    },
    /// LRCLIB has no text for this track.
    Missing,
    /// Text without timings: scrolled by hand.
    Plain {
        text: String,
    },
    Synced {
        lines: Vec<LyricLine>,
    },
}

/// What «Testo» shows. `version` grows with every change; cursors and taps carry it back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LyricsSnapshot {
    pub version: u64,
    pub status: LyricsStatus,
}

/// Where the synced text is at one position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct LyricsCursor {
    /// The current verse; `None` before the first one.
    pub line: Option<u32>,
    /// Dots lit in an instrumental pause (1 to 3), 0 on a verse with words.
    pub dots: u32,
    /// How long until the line or the dots change, from that position; `None` after the last verse
    /// (nothing to wait for).
    pub next_change_ms: Option<u64>,
}

/// What the core's lyrics view means for the current track.
#[derive(Debug, Clone, Default)]
enum Shown {
    #[default]
    NoTrack,
    Loading,
    Failed(String),
    Missing,
    Plain(String),
    Synced(Arc<Text>),
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NoTrack, Self::NoTrack)
            | (Self::Loading, Self::Loading)
            | (Self::Missing, Self::Missing) => true,
            (Self::Failed(a), Self::Failed(b)) | (Self::Plain(a), Self::Plain(b)) => a == b,
            (Self::Synced(a), Self::Synced(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
    }
}

impl Shown {
    fn of(view: &PlaybackView) -> Self {
        let Some(track) = view.queue.current() else { return Self::NoTrack };
        if view.lyrics.track != Some(track.id) {
            return Self::Loading;
        }
        match &view.lyrics.state {
            LyricsState::Idle | LyricsState::Loading => Self::Loading,
            LyricsState::Failed(error) => Self::Failed(error.to_string()),
            LyricsState::Ready(text) if text.is_synced() => Self::Synced(Arc::clone(text)),
            LyricsState::Ready(text) => text.plain.clone().map_or(Self::Missing, Self::Plain),
        }
    }

    fn status(&self) -> LyricsStatus {
        match self {
            Self::NoTrack => LyricsStatus::NoTrack,
            Self::Loading => LyricsStatus::Loading,
            Self::Failed(message) => LyricsStatus::Failed { message: message.clone() },
            Self::Missing => LyricsStatus::Missing,
            Self::Plain(text) => LyricsStatus::Plain { text: text.clone() },
            Self::Synced(text) => LyricsStatus::Synced {
                lines: text
                    .lines
                    .iter()
                    .map(|line| LyricLine {
                        at_ms: millis(line.at),
                        text: line.text.clone(),
                        pause: is_pause(line),
                    })
                    .collect(),
            },
        }
    }
}

/// The lyrics last told to the app, kept for its cursors and taps.
#[derive(Debug, Default)]
pub(crate) struct LyricsModel {
    shown: Shown,
    version: u64,
}

impl LyricsModel {
    /// The snapshot to tell the app if what it shows changed (or always, with `first`: a core that
    /// just started).
    pub(crate) fn observe(&mut self, view: &PlaybackView, first: bool) -> Option<LyricsSnapshot> {
        let shown = Shown::of(view);
        if shown == self.shown && !first {
            return None;
        }
        if shown != self.shown {
            self.version += 1;
            self.shown = shown;
        }
        Some(LyricsSnapshot { version: self.version, status: self.shown.status() })
    }

    /// The cursor of the synced text of `version` at `position`; an empty one for anything else.
    pub(crate) fn cursor(&self, version: u64, position: Duration) -> LyricsCursor {
        match &self.shown {
            Shown::Synced(text) if version == self.version => cursor(&text.lines, position),
            _ => LyricsCursor { line: None, dots: 0, next_change_ms: None },
        }
    }

    /// Where verse `line` of `version` starts, if that text is still the one shown.
    pub(crate) fn line_at(&self, version: u64, line: u32) -> Option<Duration> {
        match &self.shown {
            Shown::Synced(text) if version == self.version => text.lines.get(line as usize).map(|l| l.at),
            _ => None,
        }
    }
}

/// The current verse, its dots and the next change at `position`, with the core's rules.
pub(crate) fn cursor(lines: &[Line], position: Duration) -> LyricsCursor {
    let line = current(lines, position);
    let dots = match line.and_then(|ix| lines.get(ix).map(|verse| (ix, verse))) {
        Some((ix, verse)) if is_pause(verse) => {
            dots_lit(verse, lines.get(ix + 1).map(|next| next.at), position)
        }
        _ => 0,
    };
    LyricsCursor { line: line.map(count), dots, next_change_ms: next_change(lines, position).map(millis) }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::Track;
    use psytify_core::error::AppError;
    use psytify_core::player::Queue;
    use psytify_core::snapshot::LyricsView;

    use super::*;
    use crate::test_support::track;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Invented placeholder verses, never real lyrics.
    fn text() -> Arc<Text> {
        Arc::new(Text {
            lines: vec![
                Line { at: ms(1_000), text: "prima riga di prova".into() },
                Line { at: ms(4_000), text: String::new() },
                Line { at: ms(10_000), text: "seconda riga di prova".into() },
            ],
            plain: None,
        })
    }

    fn view(tracks: Vec<Track>, lyrics: LyricsView) -> PlaybackView {
        let mut queue = Queue::default();
        if !tracks.is_empty() {
            queue.replace(tracks, 0, 1);
        }
        PlaybackView { queue: Arc::new(queue), lyrics, ..PlaybackView::default() }
    }

    fn ready(state: LyricsState) -> PlaybackView {
        view(vec![track(1)], LyricsView { track: Some(track(1).id), state })
    }

    #[test]
    fn the_states_follow_the_core_for_the_current_track_only() {
        let mut model = LyricsModel::default();
        let first = model.observe(&view(vec![], LyricsView::default()), true);
        assert_eq!(first.map(|s| s.status), Some(LyricsStatus::NoTrack));
        assert_eq!(model.observe(&view(vec![], LyricsView::default()), false), None, "nothing new");

        // Lyrics of another track are still loading for this one.
        let other =
            view(vec![track(1)], LyricsView { track: Some(track(2).id), state: LyricsState::Ready(text()) });
        assert_eq!(model.observe(&other, false).map(|s| s.status), Some(LyricsStatus::Loading));
        let failed = ready(LyricsState::Failed(AppError::Network));
        assert!(matches!(model.observe(&failed, false).map(|s| s.status), Some(LyricsStatus::Failed { .. })));
        let plain = Text { lines: vec![], plain: Some("testo semplice di prova".into()) };
        let shown = model.observe(&ready(LyricsState::Ready(Arc::new(plain))), false).map(|s| s.status);
        assert_eq!(shown, Some(LyricsStatus::Plain { text: "testo semplice di prova".into() }));
        let missing = ready(LyricsState::Ready(Arc::new(Text::default())));
        assert_eq!(model.observe(&missing, false).map(|s| s.status), Some(LyricsStatus::Missing));
    }

    #[test]
    fn synced_lines_cross_with_their_times_and_pauses_and_a_beat_publishes_nothing() {
        let mut model = LyricsModel::default();
        let text = text();
        let snapshot =
            model.observe(&ready(LyricsState::Ready(Arc::clone(&text))), false).unwrap_or_else(|| {
                unreachable!("a new text is told");
            });
        let LyricsStatus::Synced { lines } = snapshot.status else { unreachable!("synced") };
        assert_eq!(
            lines.iter().map(|l| (l.at_ms, l.pause)).collect::<Vec<_>>(),
            [(1_000, false), (4_000, true), (10_000, false)]
        );
        // The same text again (another playback publication) is not told twice.
        assert_eq!(model.observe(&ready(LyricsState::Ready(text)), false), None);
        assert_eq!(model.line_at(snapshot.version, 2), Some(ms(10_000)));
        assert_eq!(model.line_at(snapshot.version + 1, 2), None, "a stale tap does nothing");
        assert_eq!(model.line_at(snapshot.version, 9), None);
    }

    #[test]
    fn the_cursor_names_the_line_the_dots_and_when_to_ask_again() {
        let mut model = LyricsModel::default();
        let version = model.observe(&ready(LyricsState::Ready(text())), false).map_or(0, |s| s.version);
        assert_eq!(
            model.cursor(version, ms(0)),
            LyricsCursor { line: None, dots: 0, next_change_ms: Some(1_000) }
        );
        assert_eq!(
            model.cursor(version, ms(2_000)),
            LyricsCursor { line: Some(0), dots: 0, next_change_ms: Some(2_000) }
        );
        // The pause from 4 s to 10 s lights a dot every 2 s.
        assert_eq!(
            model.cursor(version, ms(4_000)),
            LyricsCursor { line: Some(1), dots: 1, next_change_ms: Some(2_000) }
        );
        assert_eq!(model.cursor(version, ms(9_000)).dots, 3);
        assert_eq!(
            model.cursor(version, ms(12_000)),
            LyricsCursor { line: Some(2), dots: 0, next_change_ms: None }
        );
        assert_eq!(model.cursor(version + 1, ms(12_000)).line, None, "another text's cursor is empty");
    }
}
