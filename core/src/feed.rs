//! The «uscite» (releases): the catalog's recent tracks grouped as the listener sees them (an album, or a
//! single), with the release date in the catalog. The backend has no list of new releases nor of
//! followed artists, so the feed is derived here from the tracks, which have `created_at`.

use uuid::Uuid;

use crate::api::models::ArtistRef;
use crate::api::{AlbumSummary, Track};
use crate::config::taste::{self, ArtistTaste, Mix};
use crate::config::{FollowedArtist, PlayStat};

/// Releases in the carousel at the top of the Home.
pub const HERO_COUNT: usize = 5;
/// Mixes shown on the Home.
const MIX_COUNT: usize = 4;
/// Artists in the row «I tuoi artisti».
const ARTIST_COUNT: usize = 20;

/// The Home feed, computed when the data changes and not on every frame: the library domain
/// rebuilds it when the recent tracks, the followed artists or the plays change.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Feed {
    /// All known releases, from the most recent.
    pub releases: Vec<Release>,
    /// The most recent ones with a cover to show large: the carousel.
    pub hero: Vec<Release>,
    /// The releases of the followed artists.
    pub news: Vec<Release>,
    pub mixes: Vec<Mix>,
    /// The artists to show in a circle, and whether they are "yours" (followed or listened to).
    pub artists: Vec<ArtistCard>,
    pub artists_are_yours: bool,
    /// The most played by the listener, from the first.
    pub top: Vec<PlayStat>,
}

impl Feed {
    /// Builds the feed from the catalog's recent tracks, the followed artists and the plays.
    /// `seed` keeps the mixes fixed for the whole session.
    pub fn build(tracks: &[Track], followed: &[FollowedArtist], stats: &[PlayStat], seed: u64) -> Self {
        let releases = releases(tracks);
        let mut hero: Vec<Release> = releases.iter().filter(|r| r.cover_url.is_some()).cloned().collect();
        hero.truncate(HERO_COUNT);
        let news: Vec<Release> = from_followed(&releases, followed).into_iter().cloned().collect();

        let mut cards: Vec<ArtistCard> = followed
            .iter()
            .map(|a| ArtistCard {
                artist: ArtistRef { id: a.id, name: a.name.clone() },
                cover_url: a.cover_url.clone(),
                tracks: 0,
            })
            .collect();
        for ArtistTaste { artist, cover_url, .. } in taste::top_artists(stats) {
            if !cards.iter().any(|a| a.artist.id == artist.id) {
                cards.push(ArtistCard { artist, cover_url, tracks: 0 });
            }
        }
        let artists_are_yours = !cards.is_empty();
        for card in artists(tracks) {
            if !cards.iter().any(|a| a.artist.id == card.artist.id) {
                cards.push(card);
            }
        }
        cards.truncate(ARTIST_COUNT);

        Self {
            mixes: taste::mixes(stats, tracks, MIX_COUNT, seed),
            top: taste::top_tracks(stats, 10).into_iter().cloned().collect(),
            releases,
            hero,
            news,
            artists: cards,
            artists_are_yours,
        }
    }
}

/// An album or a single, with the tracks the feed knows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The album's identifier, or the track's if it is a single.
    pub id: Uuid,
    pub is_album: bool,
    pub title: String,
    pub artist: ArtistRef,
    pub cover_url: Option<String>,
    /// The most recent release among its tracks.
    pub created_at: String,
    pub tracks: Vec<Track>,
}

impl Release {
    /// «Album» or «Singolo», for the text under the title.
    pub fn kind_label(&self) -> &'static str {
        if self.is_album { "Album" } else { "Singolo" }
    }
}

/// The releases of the given tracks, from the most recent. The tracks of one album become a single
/// release; the order is that of `created_at` (ISO 8601 text, which sorts like time).
pub fn releases(tracks: &[Track]) -> Vec<Release> {
    let mut found: Vec<Release> = Vec::new();
    for track in tracks {
        let (id, is_album, title) = match &track.album {
            Some(album) => (album.id, true, album.title.clone()),
            None => (track.id, false, track.title.clone()),
        };
        match found.iter_mut().find(|r| r.id == id) {
            Some(release) => {
                if track.created_at > release.created_at {
                    release.created_at.clone_from(&track.created_at);
                }
                if release.cover_url.is_none() {
                    release.cover_url.clone_from(&track.cover_url);
                }
                release.tracks.push(track.clone());
            }
            None => found.push(Release {
                id,
                is_album,
                title,
                artist: track.artist.clone(),
                cover_url: track.cover_url.clone(),
                created_at: track.created_at.clone(),
                tracks: vec![track.clone()],
            }),
        }
    }
    for release in &mut found {
        release.tracks.sort_by_key(|t| (t.track_no.unwrap_or(u32::MAX), t.created_at.clone()));
    }
    found.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.title.cmp(&b.title)));
    found
}

/// The releases of the followed artists, from the most recent.
pub fn from_followed<'a>(releases: &'a [Release], followed: &[FollowedArtist]) -> Vec<&'a Release> {
    releases.iter().filter(|r| followed.iter().any(|f| f.id == r.artist.id)).collect()
}

/// How many releases arrived since the last time "Novità" was looked at.
pub fn unread(news: &[&Release], seen: &str) -> usize {
    news.iter().filter(|r| r.created_at.as_str() > seen).count()
}

/// An ISO 8601 date (`2026-05-01T10:00:00Z`) in short words: "1 mag 2026".
pub fn date_label(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] =
        ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"];
    let year = iso.get(0..4)?;
    let month: usize = iso.get(5..7)?.parse().ok()?;
    let day: u32 = iso.get(8..10)?.parse().ok()?;
    let name = MONTHS.get(month.checked_sub(1)?)?;
    Some(format!("{day} {name} {year}"))
}

/// An artist to show in a circle: who they are, with their image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistCard {
    pub artist: ArtistRef,
    pub cover_url: Option<String>,
    /// Tracks of this artist in the feed: used to order who has the most material.
    pub tracks: usize,
}

/// The artists present in the given tracks, those with the most tracks first. An artist's image is the
/// cover of one of their tracks.
pub fn artists(tracks: &[Track]) -> Vec<ArtistCard> {
    let mut found: Vec<ArtistCard> = Vec::new();
    for track in tracks {
        match found.iter_mut().find(|a| a.artist.id == track.artist.id) {
            Some(known) => {
                known.tracks += 1;
                if known.cover_url.is_none() {
                    known.cover_url.clone_from(&track.cover_url);
                }
            }
            None => found.push(ArtistCard {
                artist: track.artist.clone(),
                cover_url: track.cover_url.clone(),
                tracks: 1,
            }),
        }
    }
    found.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.artist.name.cmp(&b.artist.name)));
    found
}

/// An artist's tracks to put first: the most played by the listener, then the most recent.
pub fn popular(tracks: &[Track], stats: &[PlayStat], count: usize) -> Vec<Track> {
    let plays = |t: &Track| stats.iter().find(|s| s.track.id == t.id).map_or(0, |s| s.plays);
    let mut ranked: Vec<&Track> = tracks.iter().collect();
    ranked.sort_by(|a, b| plays(b).cmp(&plays(a)).then_with(|| b.created_at.cmp(&a.created_at)));
    ranked.into_iter().take(count).cloned().collect()
}

/// The artists of the given albums and tracks, in the order they appear (the album covers
/// first: they are the most carefully made). An artist appears only once.
pub fn artists_from(albums: &[AlbumSummary], tracks: &[Track]) -> Vec<ArtistCard> {
    let mut found: Vec<ArtistCard> = Vec::new();
    let mut add = |artist: &ArtistRef, cover_url: Option<&String>| match found
        .iter_mut()
        .find(|a| a.artist.id == artist.id)
    {
        Some(known) => {
            known.tracks += 1;
            if known.cover_url.is_none() {
                known.cover_url = cover_url.cloned();
            }
        }
        None => found.push(ArtistCard { artist: artist.clone(), cover_url: cover_url.cloned(), tracks: 1 }),
    };
    for album in albums {
        add(&album.artist, album.cover_url.as_ref());
    }
    for track in tracks {
        add(&track.artist, track.cover_url.as_ref());
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::{AlbumRef, fixtures::track};

    fn on_album(n: u128, album: u128, created: &str) -> Track {
        let mut t = track(n);
        t.album = Some(AlbumRef { id: Uuid::from_u128(album), title: format!("Album {album}") });
        t.track_no = Some(n as u32);
        t.created_at = created.into();
        t
    }

    fn single(n: u128, created: &str) -> Track {
        let mut t = track(n);
        t.album = None;
        t.created_at = created.into();
        t
    }

    #[test]
    fn the_tracks_of_an_album_become_one_release() {
        let tracks = [on_album(1, 10, "2026-03-01"), on_album(2, 10, "2026-03-02"), single(3, "2026-01-01")];
        let found = releases(&tracks);
        assert_eq!(found.len(), 2);
        assert!(found[0].is_album && found[0].tracks.len() == 2);
        assert_eq!(found[0].created_at, "2026-03-02");
        assert!(!found[1].is_album);
    }

    #[test]
    fn the_newest_release_comes_first() {
        let tracks = [single(1, "2026-01-01"), single(2, "2026-05-01"), single(3, "2026-03-01")];
        let order: Vec<Uuid> = releases(&tracks).iter().map(|r| r.id).collect();
        assert_eq!(order, [track(2).id, track(3).id, track(1).id]);
    }

    #[test]
    fn tracks_of_an_album_are_in_album_order() {
        let tracks =
            [on_album(3, 10, "2026-03-01"), on_album(1, 10, "2026-03-01"), on_album(2, 10, "2026-03-01")];
        let numbers: Vec<_> = releases(&tracks)[0].tracks.iter().map(|t| t.track_no).collect();
        assert_eq!(numbers, [Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn only_followed_artists_make_the_news() {
        let mut other = single(2, "2026-02-01");
        other.artist = ArtistRef { id: Uuid::from_u128(900), name: "Sale".into() };
        let tracks = [single(1, "2026-01-01"), other];
        let all = releases(&tracks);
        let followed = [FollowedArtist { id: tracks[1].artist.id, name: "Sale".into(), cover_url: None }];
        let news = from_followed(&all, &followed);
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].artist.name, "Sale");
    }

    #[test]
    fn unread_counts_what_came_after_the_last_visit() {
        let tracks = [single(1, "2026-01-01"), single(2, "2026-03-01"), single(3, "2026-05-01")];
        let all = releases(&tracks);
        let news: Vec<&Release> = all.iter().collect();
        assert_eq!(unread(&news, ""), 3);
        assert_eq!(unread(&news, "2026-03-01"), 1);
        assert_eq!(unread(&news, "2026-06-01"), 0);
    }

    #[test]
    fn albums_and_tracks_give_each_artist_once() {
        let mut a = single(1, "2026-01-01");
        let mut b = single(2, "2026-01-01");
        a.artist = ArtistRef { id: Uuid::from_u128(1), name: "Uno".into() };
        b.artist = ArtistRef { id: Uuid::from_u128(2), name: "Due".into() };
        let album = AlbumSummary {
            id: Uuid::from_u128(9),
            title: "Banchi".into(),
            released_on: None,
            artist: a.artist.clone(),
            track_count: 3,
            cover_url: Some("https://archive.org/img/9".into()),
        };
        let found = artists_from(&[album], &[a, b]);
        let names: Vec<_> = found.iter().map(|c| c.artist.name.as_str()).collect();
        assert_eq!(names, ["Uno", "Due"]);
        assert_eq!(found[0].cover_url.as_deref(), Some("https://archive.org/img/9"));
    }

    #[test]
    fn popular_tracks_are_the_most_played_then_the_newest() {
        let tracks = [single(1, "2026-01-01"), single(2, "2026-03-01"), single(3, "2026-02-01")];
        let stats = [PlayStat { track: tracks[0].clone(), plays: 4, last_played: 10 }];
        let order: Vec<Uuid> = popular(&tracks, &stats, 3).iter().map(|t| t.id).collect();
        assert_eq!(order, [track(1).id, track(2).id, track(3).id]);
        assert_eq!(popular(&tracks, &stats, 2).len(), 2);
    }

    fn dated(n: u128, created: &str, cover: bool) -> Track {
        let mut t = track(n);
        t.album = None;
        t.created_at = created.into();
        t.cover_url = cover.then(|| format!("https://archive.org/img/{n}"));
        t
    }

    #[test]
    fn the_hero_shows_the_newest_releases_that_have_a_cover() {
        let tracks: Vec<Track> = (1..=8).map(|n| dated(n, &format!("2026-0{n}-01"), n != 8)).collect();
        let feed = Feed::build(&tracks, &[], &[], 1);
        assert_eq!(feed.hero.len(), HERO_COUNT);
        assert!(feed.hero.iter().all(|r| r.cover_url.is_some()));
        assert_eq!(feed.hero[0].id, track(7).id);
    }

    #[test]
    fn the_news_are_the_releases_of_followed_artists() {
        let mut a = dated(1, "2026-01-01", true);
        let mut b = dated(2, "2026-02-01", true);
        a.artist = ArtistRef { id: Uuid::from_u128(11), name: "Uno".into() };
        b.artist = ArtistRef { id: Uuid::from_u128(22), name: "Due".into() };
        let followed = [FollowedArtist { id: b.artist.id, name: "Due".into(), cover_url: None }];
        let feed = Feed::build(&[a, b], &followed, &[], 1);
        assert_eq!(feed.news.len(), 1);
        assert_eq!(feed.news[0].artist.name, "Due");
    }

    #[test]
    fn followed_artists_come_first_in_the_artist_row() {
        let mut a = dated(1, "2026-01-01", true);
        let mut b = dated(2, "2026-02-01", true);
        a.artist = ArtistRef { id: Uuid::from_u128(11), name: "Uno".into() };
        b.artist = ArtistRef { id: Uuid::from_u128(22), name: "Due".into() };
        let followed = [FollowedArtist { id: a.artist.id, name: "Uno".into(), cover_url: None }];
        let feed = Feed::build(&[b, a], &followed, &[], 1);
        assert!(feed.artists_are_yours);
        assert_eq!(feed.artists[0].artist.name, "Uno");
        assert_eq!(feed.artists.len(), 2);
    }

    #[test]
    fn without_taste_the_artists_come_from_the_catalogue() {
        let feed = Feed::build(&[dated(1, "2026-01-01", true)], &[], &[], 1);
        assert!(!feed.artists_are_yours);
        assert_eq!(feed.artists.len(), 1);
    }

    #[test]
    fn dates_read_in_italian() {
        assert_eq!(date_label("2026-05-01T10:00:00Z").as_deref(), Some("1 mag 2026"));
        assert_eq!(date_label("2025-12-24").as_deref(), Some("24 dic 2025"));
    }

    #[test]
    fn a_broken_date_gives_nothing() {
        assert_eq!(date_label("ieri"), None);
        assert_eq!(date_label("2026-13-01"), None);
    }

    #[test]
    fn artists_with_more_tracks_come_first() {
        let mut a = single(1, "2026-01-01");
        let mut b = single(2, "2026-01-01");
        let mut c = single(3, "2026-01-01");
        a.artist = ArtistRef { id: Uuid::from_u128(1), name: "Uno".into() };
        b.artist = ArtistRef { id: Uuid::from_u128(2), name: "Due".into() };
        c.artist = b.artist.clone();
        let found = artists(&[a, b, c]);
        assert_eq!(found[0].artist.name, "Due");
        assert_eq!(found[0].tracks, 2);
    }
}
