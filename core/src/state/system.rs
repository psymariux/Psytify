//! What the system and Discord show of the track playing (step 4.3b): the desktop's media widget
//! (MPRIS), the phone's lock screen and "Sta ascoltando" on Discord.
//!
//! The reducer decides **what** to show and **when**, looking at the playback view:
//! only at a transition (another track, pause and resume, a jump in the track), never at each tick;
//! the host, which holds the system links on the core's thread, runs the effects
//! ([`MediaOp`], [`DiscordOp`]). The position to show is `clock.at(now)` with the injected `now`,
//! so the reducer stays without a clock; Discord wants the system's time and the host takes it.

use std::time::{Duration, Instant};

use bytes::Bytes;
use uuid::Uuid;

use crate::config::DEFAULT_DISCORD_APP_ID;
use crate::effect::Effect;
use crate::media::Now;
use crate::state::playback::PlaybackView;

/// The cover already decoded, which the frontend hands to the core with
/// [`PlaybackCmd::Artwork`](crate::command::PlaybackCmd::Artwork): the media session of
/// Android wants it in pixels, while MPRIS reads the address. The core keeps it as long as the track in
/// progress is the one of its `url`.
#[derive(Clone)]
pub struct Artwork {
    pub url: String,
    pub side: u32,
    pub rgba: Bytes,
}

impl std::fmt::Debug for Artwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Artwork").field("url", &self.url).field("side", &self.side).finish_non_exhaustive()
    }
}

/// An operation on the system's media link. The host runs it if the link is
/// ready (`Event::Ready`); before that, it is lost, and at `Ready` the reducer sends the state again.
#[derive(Debug)]
pub enum MediaOp {
    /// The track shown, or nothing with the player stopped.
    Show(Option<Box<Now>>),
    Volume(f32),
}

/// A Discord Rich Presence operation.
#[derive(Debug)]
// `testing` re-exports the type, so its fields count as read there.
#[cfg_attr(
    all(target_os = "android", not(any(test, feature = "test-support"))),
    expect(dead_code, reason = "su Android non c'è Discord: l'host non legge i campi")
)]
pub enum DiscordOp {
    /// Starts the thread for the Discord app with this identifier.
    Start(String),
    /// Stops the thread: the activity disappears from Discord.
    Stop,
    /// The activity to show, or none (when paused it disappears, as in other music apps).
    Set(Option<Box<DiscordNow>>),
}

/// The track Discord shows, with the position and the duration: the host makes the activity with the time of the
/// sistema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordNow {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub position: Duration,
    pub duration: Duration,
}

/// What was shown last time: if it does not change, nothing is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    track: Option<Uuid>,
    playing: bool,
    /// A jump in the track or a track starting again moves the position the system shows.
    seeks: u64,
    started: u64,
    /// The cover in pixels has arrived.
    art: bool,
}

#[derive(Debug, Default)]
pub(crate) struct System {
    /// The media link has given its sender: from now on the effects arrive.
    ready: bool,
    shown: Option<Shown>,
    volume: Option<f32>,
    art: Option<Artwork>,
    /// The identifier of the Discord app with the thread on, if it is on.
    discord: Option<String>,
    discord_shown: Option<Shown>,
}

impl System {
    /// The link is ready: everything the system has not seen yet is sent again.
    pub(crate) fn ready(&mut self) {
        self.ready = true;
        self.shown = None;
        self.volume = None;
    }

    pub(crate) fn set_art(&mut self, art: Artwork) {
        self.art = Some(art);
    }

    /// The Discord setting has changed, or not: it turns the thread on or off. Without Discord (Android)
    /// or with the switch off nothing starts.
    pub(crate) fn set_discord(&mut self, presence: bool, app_id: &str, effects: &mut Vec<Effect>) -> bool {
        let chosen = app_id.trim();
        let chosen = if chosen.is_empty() { DEFAULT_DISCORD_APP_ID } else { chosen };
        let wanted = (cfg!(not(target_os = "android")) && presence && !chosen.is_empty()).then_some(chosen);
        if self.discord.as_deref() == wanted {
            return false;
        }
        if self.discord.take().is_some() {
            effects.push(Effect::Discord(DiscordOp::Stop));
        }
        if let Some(id) = wanted {
            self.discord = Some(id.to_owned());
            effects.push(Effect::Discord(DiscordOp::Start(id.to_owned())));
        }
        self.discord_shown = None;
        true
    }

    /// After each input: if the track, the pause or the position have changed for the system or for
    /// Discord, it is said.
    pub(crate) fn sync(&mut self, view: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        let wants_media = self.ready;
        if !wants_media && self.discord.is_none() {
            return;
        }
        let track = view.queue.current().filter(|_| view.is_active());
        let position = view.clock.at(now);
        let art =
            track.and_then(|t| self.art.as_ref().filter(|a| t.cover_url.as_deref() == Some(a.url.as_str())));
        let shown = Shown {
            track: track.map(|t| t.id),
            playing: view.is_playing(),
            seeks: view.seeks,
            started: view.started,
            art: art.is_some(),
        };
        if wants_media {
            if self.shown != Some(shown) {
                self.shown = Some(shown);
                let now = track.map(|track| {
                    Box::new(Now {
                        title: track.title.clone(),
                        artist: track.artist.name.clone(),
                        album: track.album.as_ref().map(|album| album.title.clone()),
                        cover_url: track.cover_url.clone(),
                        duration: track.duration(),
                        playing: shown.playing,
                        position,
                        artwork: art.map(|a| (a.side, a.rgba.clone())),
                    })
                });
                effects.push(Effect::Media(MediaOp::Show(now)));
            }
            if self.volume != Some(view.volume) {
                self.volume = Some(view.volume);
                effects.push(Effect::Media(MediaOp::Volume(view.volume)));
            }
        }
        if self.discord.is_some() {
            // Paused or stopped the activity is the same, whatever changes: none.
            let on_air = track.filter(|_| shown.playing);
            let key = on_air.map(|_| Shown { art: false, ..shown });
            if self.discord_shown != Some(key.unwrap_or(IDLE)) {
                self.discord_shown = Some(key.unwrap_or(IDLE));
                let activity = on_air.map(|track| {
                    Box::new(DiscordNow {
                        title: track.title.clone(),
                        artist: track.artist.name.clone(),
                        album: track.album.as_ref().map(|album| album.title.clone()),
                        cover_url: track.cover_url.clone(),
                        position,
                        duration: track.duration(),
                    })
                });
                effects.push(Effect::Discord(DiscordOp::Set(activity)));
            }
        }
    }
}

/// "Niente su Discord": the key for when nothing plays.
const IDLE: Shown = Shown { track: None, playing: false, seeks: 0, started: 0, art: false };
