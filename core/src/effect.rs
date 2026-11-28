//! What the reducer asks of the host. These are data, not closures: the tests check
//! what was asked without running it. The host arms and cancels the timers, runs the domains'
//! async jobs ([`Job`]) on the runtime and reports their outcome to the reducer ([`Done`]).

use std::time::Duration;

use crate::error::AppError;
use crate::state::jam::JamOp;
use crate::state::playback::AudioOp;
use crate::state::system::{DiscordOp, MediaOp};
#[cfg(feature = "video")]
use crate::state::video;
use crate::state::{catalog, downloads, external, jam, library, playback, settings, social};

/// An effect to run.
#[derive(Debug)]
pub(crate) enum Effect {
    /// Arms a timer that fires after `after`. Re-arming the same key cancels the previous timer:
    /// a debounce is a re-arm.
    Timer { key: TimerKey, after: Duration },
    /// Cancels an armed timer; if there is none, nothing.
    CancelTimer(TimerKey),
    /// An operation on the audio output. The host runs it at once, on the core's thread, and what
    /// the audio answers comes back in the same round as `Input::Audio`.
    Audio(AudioOp),
    /// The track or volume to show to the system: the host sends it to the media link it holds,
    /// if it is ready.
    Media(MediaOp),
    /// Turns Discord Rich Presence on, off or updates it.
    Discord(DiscordOp),
    /// An operation on the jam room link, which the host holds on its own thread: it starts it,
    /// sends it the local state, closes it. The room's events come back as `Input::Jam`.
    Jam(JamOp),
    /// Opens an address in the system browser (sign-in with GitHub, Google or Discord): the host
    /// runs it at once with `platform::open_url`, and if it does not open it answers with
    /// `Done::Social(BrowserFailed)`. The address is a `Secret`: it does not appear in logs.
    OpenUrl(crate::auth::Secret),
    /// Opens (`true`) or closes the link that listens to the motion requested by the system
    /// (`system_motion::events`), which the host holds on the core's thread: the readings come back as
    /// `Input::SystemMotion`.
    WatchMotion(bool),
    /// Runs a job on the runtime; its outcome returns to the domain as `Input::Done`.
    Run(Job),
    /// The shutdown requested with `Input::Shutdown` is ready: `config.json` has the last state, or
    /// the last write failed for this reason. After this, the core stops.
    Flushed(Result<(), AppError>),
}

/// The key of a timer: the domain and the timer, declared in the domain's module. A domain
/// removes its `expect` when it arms the first timer. The jam has none: it runs on the events of its
/// link. The music video has only one, armed while a shown video waits for the
/// full version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerKey {
    Settings(settings::Timer),
    #[expect(dead_code, reason = "nessun timer finché il dominio non passa nel core (passo 4.2)")]
    Library(library::Timer),
    Playback(playback::Timer),
    Catalog(catalog::Timer),
    External(external::Timer),
    Downloads(downloads::Timer),
    Social(social::Timer),
    #[cfg(feature = "video")]
    Video(video::Timer),
}

/// An async job (network, files): the domain and the job, declared in the domain's module.
/// A domain removes its `expect` when it asks for the first job.
#[derive(Debug)]
pub(crate) enum Job {
    Settings(settings::Job),
    #[expect(dead_code, reason = "nessun lavoro finché il dominio non passa nel core (passo 4.2)")]
    Library(library::Job),
    Playback(playback::Job),
    Catalog(catalog::Job),
    External(external::Job),
    Downloads(downloads::Job),
    Social(social::Job),
    Jam(jam::Job),
    #[cfg(feature = "video")]
    Video(video::Job),
}

/// The outcome of a [`Job`], for the domain that asked for it.
#[derive(Debug)]
pub(crate) enum Done {
    Settings(settings::Done),
    Library(library::Done),
    Playback(playback::Done),
    Catalog(catalog::Done),
    External(external::Done),
    Downloads(downloads::Done),
    Social(social::Done),
    Jam(jam::Done),
    #[cfg(feature = "video")]
    Video(video::Done),
}

impl Job {
    /// Runs the job. It runs on the runtime, outside the reducer: it is the only place where a domain
    /// touches the network or files.
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Settings(job) => Done::Settings(job.run().await),
            Self::Library(job) => Done::Library(job.run().await),
            Self::Playback(job) => Done::Playback(job.run().await),
            Self::Catalog(job) => Done::Catalog(job.run().await),
            Self::External(job) => Done::External(job.run().await),
            Self::Downloads(job) => Done::Downloads(job.run().await),
            Self::Social(job) => Done::Social(job.run().await),
            Self::Jam(job) => Done::Jam(job.run().await),
            #[cfg(feature = "video")]
            Self::Video(job) => Done::Video(job.run().await),
        }
    }

    /// The outcome to deliver if the job ends in a panic: the domain still gets a
    /// reply and does not keep waiting (at shutdown, the core would never stop).
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Settings(job) => Done::Settings(job.lost()),
            Self::Library(job) => Done::Library(job.lost()),
            Self::Playback(job) => Done::Playback(job.lost()),
            Self::Catalog(job) => Done::Catalog(job.lost()),
            Self::External(job) => Done::External(job.lost()),
            Self::Downloads(job) => Done::Downloads(job.lost()),
            Self::Social(job) => Done::Social(job.lost()),
            Self::Jam(job) => Done::Jam(job.lost()),
            #[cfg(feature = "video")]
            Self::Video(job) => Done::Video(job.lost()),
        }
    }
}
