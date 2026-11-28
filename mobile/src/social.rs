//! What the app shows of the friends and the jam outside their pages: the jam marker of the player
//! bar (`LIVE` while in a room) and the counts on the way into «Amici» and «Jam». Told to the app only
//! when they change (`CoreListener::marks_changed`).

use psytify_core::Snapshot;
use psytify_core::snapshot::JamPhase;

use crate::playback::count;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct SocialMarks {
    /// In a jam: connecting, or in the room.
    pub jam: bool,
    /// In the room: the `LIVE` badge.
    pub live: bool,
    /// People in the room.
    pub members: u32,
    /// Jam invites waiting.
    pub invites: u32,
    /// Friend requests waiting.
    pub requests: u32,
}

impl SocialMarks {
    pub(crate) fn of(snapshot: &Snapshot) -> Self {
        let (jam, social) = (&snapshot.jam, &snapshot.social);
        Self {
            jam: jam.is_active(),
            live: jam.phase == JamPhase::Open,
            members: count(jam.members.len()),
            invites: count(social.jam_invites.len()),
            requests: social.friends.ready().map_or(0, |friends| count(friends.incoming.len())),
        }
    }
}
