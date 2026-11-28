//! Data shared by the unit tests.

use psytify_core::api::Track;
use psytify_core::api::models::{ArtistRef, Licence};
use uuid::Uuid;

pub(crate) fn track(n: u128) -> Track {
    Track {
        id: Uuid::from_u128(n),
        title: format!("Track {n}"),
        track_no: None,
        duration_ms: 180_000,
        audio_format: "flac".into(),
        downloadable: false,
        has_video: false,
        video_ready: false,
        artist: ArtistRef { id: Uuid::from_u128(1000 + n), name: format!("Artist {n}") },
        album: None,
        licence: Licence { kind: "cc0".into(), attribution: String::new() },
        cover_url: Some(format!("https://archive.org/services/img/item-{n}")),
        created_at: format!("2026-11-{:02}T00:00:00Z", 1 + n % 28),
    }
}
