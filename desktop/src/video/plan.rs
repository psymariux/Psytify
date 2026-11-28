//! The video's decisions, pure: when a pipeline must exist, what to do when the core
//! publishes a new video, how to apply to the player what the core's sync rules decide
//! (`psytify_core::video::Alignment`) and when to check the drift. `stage.rs` applies them.
//!
//! **When a pipeline exists.** Only for the track playing, only while an interface spot
//! shows the video (the panel on the video or the immersive view on the video), only while the track
//! plays or is loading, and only for the video that the core has ready for that track. Paused, stopped,
//! errore, cambio di brano, videoclip nascosto: nessuna pipeline, nessun thread, nessun fotogramma
//! (`05-video-spike.md` §5).

use std::time::{Duration, Instant};

use psytify_core::video::Adjust;
use psytify_design::tokens::motion::PROGRESS_TICK_MS;
use uuid::Uuid;

/// The video that must be open: the track and the sequence number of the core's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub track: Uuid,
    pub seq: u64,
}

/// What is known to decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// An interface spot shows the video.
    pub shown: bool,
    /// Il brano in corso, se suona o si sta caricando (in pausa, fermo o in errore: `None`).
    pub running: Option<Uuid>,
    /// The track the core's video view talks about, and the `seq` of the ready video (`None` if it
    /// is not ready: stopped, preparing or failed).
    pub ready: Option<Want>,
}

/// The video that must be open now, if there is one.
pub fn wanted(inputs: Inputs) -> Option<Want> {
    let ready = inputs.ready?;
    (inputs.shown && inputs.running == Some(ready.track)).then_some(ready)
}

/// The pipeline there is: for which video, if it has already opened and which better version is opening
/// di fianco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Current {
    pub track: Uuid,
    pub seq: u64,
    /// The pipeline has opened (otherwise it is still opening on another thread).
    pub open: bool,
    /// The `seq` of the better version that opens alongside.
    pub upgrading: Option<u64>,
}

/// Che cosa fare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing: what there is is fine (or there is nothing and nothing is needed).
    Keep,
    /// Close what there is (if any) and open this.
    Open(Want),
    /// Open a better version of the same track alongside and replace it when it is ready, without
    /// fermarsi.
    Upgrade(Want),
    /// Close: nobody wants a video any more.
    Close,
}

/// From the pipeline there is to the one needed (the same rules as `open_clip`).
pub fn plan(current: Option<Current>, want: Option<Want>) -> Action {
    match (current, want) {
        (None, None) => Action::Keep,
        (Some(_), None) => Action::Close,
        (Some(now), Some(want))
            if now.track == want.track && (now.seq == want.seq || now.upgrading == Some(want.seq)) =>
        {
            Action::Keep
        }
        (Some(now), Some(want)) if now.track == want.track && now.open => Action::Upgrade(want),
        (_, Some(want)) => Action::Open(want),
    }
}

/// What can be done to the player so that it follows the audio.
pub trait Controls {
    fn set_paused(&mut self, paused: bool);
    fn seek(&mut self, position: Duration);
}

/// Does to the player what the core's rules have decided: first the pause or the resume, then the seek.
pub fn apply(player: &mut impl Controls, adjust: Adjust) {
    if let Some(paused) = adjust.pause {
        player.set_paused(paused);
    }
    if let Some(position) = adjust.seek {
        player.seek(position);
    }
}

/// How often the drift between video and audio is checked: the beat of the progress bar. The check
/// rides on the frame wake-up that already exists (no extra timer): it is done only if at least
/// this has passed since the last one.
pub const CHECK_EVERY: Duration = Duration::from_millis(PROGRESS_TICK_MS as u64);

/// È ora di controllare lo scarto.
pub fn check_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= CHECK_EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(0xA);
    const B: Uuid = Uuid::from_u128(0xB);

    fn ready(track: Uuid, seq: u64) -> Option<Want> {
        Some(Want { track, seq })
    }

    #[test]
    fn a_pipeline_exists_only_for_the_running_track_with_a_ready_clip_on_screen() {
        let on = Inputs { shown: true, running: Some(A), ready: ready(A, 1) };
        assert_eq!(wanted(on), ready(A, 1));
        // Nascosto, in pausa (o fermo, in errore), un altro brano, nessun videoclip pronto: niente.
        assert_eq!(wanted(Inputs { shown: false, ..on }), None);
        assert_eq!(wanted(Inputs { running: None, ..on }), None);
        assert_eq!(wanted(Inputs { running: Some(B), ..on }), None);
        assert_eq!(wanted(Inputs { ready: None, ..on }), None);
        // The core's view still talks about the previous track: its next publication is awaited.
        assert_eq!(wanted(Inputs { ready: ready(B, 4), ..on }), None);
    }

    #[test]
    fn a_new_clip_opens_a_better_one_opens_beside_and_nothing_closes_what_is_wanted() {
        let open = Current { track: A, seq: 1, open: true, upgrading: None };
        // Niente e niente: fermi. Qualcosa e niente: si chiude.
        assert_eq!(plan(None, None), Action::Keep);
        assert_eq!(plan(Some(open), None), Action::Close);
        // Il primo videoclip si apre.
        assert_eq!(plan(None, ready(A, 1)), Action::Open(Want { track: A, seq: 1 }));
        // The same: it stays. A new seq of the same track already open: the better version alongside.
        assert_eq!(plan(Some(open), ready(A, 1)), Action::Keep);
        assert_eq!(plan(Some(open), ready(A, 2)), Action::Upgrade(Want { track: A, seq: 2 }));
        // While the better version is opening, no other copy is opened.
        let upgrading = Current { upgrading: Some(2), ..open };
        assert_eq!(plan(Some(upgrading), ready(A, 2)), Action::Keep);
        // A new seq while the first pipeline is still opening: it restarts with that one.
        let opening = Current { open: false, ..open };
        assert_eq!(plan(Some(opening), ready(A, 2)), Action::Open(Want { track: A, seq: 2 }));
        // Another track: the old one is closed and the new one opened.
        assert_eq!(plan(Some(open), ready(B, 3)), Action::Open(Want { track: B, seq: 3 }));
    }

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
    }

    impl Controls for Fake {
        fn set_paused(&mut self, paused: bool) {
            self.calls.push(format!("paused {paused}"));
        }
        fn seek(&mut self, position: Duration) {
            self.calls.push(format!("seek {}", position.as_millis()));
        }
    }

    #[test]
    fn the_core_decision_is_applied_pause_first_then_the_seek() {
        let mut player = Fake::default();
        apply(&mut player, Adjust::default());
        assert!(player.calls.is_empty(), "niente da fare, niente al player");
        apply(&mut player, Adjust { pause: Some(false), seek: Some(Duration::from_millis(1500)) });
        apply(&mut player, Adjust { pause: Some(true), seek: None });
        apply(&mut player, Adjust { pause: None, seek: Some(Duration::from_millis(40)) });
        assert_eq!(player.calls, ["paused false", "seek 1500", "paused true", "seek 40"]);
    }

    #[test]
    fn the_drift_check_rides_the_frames_at_most_every_tick() {
        let start = Instant::now();
        assert!(check_due(None, start), "il primo fotogramma controlla subito");
        assert!(!check_due(Some(start), start + Duration::from_millis(33)));
        assert!(!check_due(Some(start), start + CHECK_EVERY - Duration::from_millis(1)));
        assert!(check_due(Some(start), start + CHECK_EVERY));
        // A clock that goes back breaks nothing.
        assert!(!check_due(Some(start + CHECK_EVERY), start));
    }
}
