//! Le «uscite»: i brani recenti del catalogo raggruppati come li vede chi ascolta (un album, o un
//! singolo), con la data di pubblicazione nel catalogo. Il backend non ha un elenco di novità né di
//! artisti seguiti, quindi il feed si ricava qui dai brani, che hanno `created_at`.

use uuid::Uuid;

use crate::api::models::ArtistRef;
use crate::api::{AlbumSummary, Track};
use crate::config::{FollowedArtist, PlayStat};

/// Un album o un singolo, con i brani che il feed ne conosce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// L'identificatore dell'album, o del brano se è un singolo.
    pub id: Uuid,
    pub is_album: bool,
    pub title: String,
    pub artist: ArtistRef,
    pub cover_url: Option<String>,
    /// La pubblicazione più recente tra i suoi brani.
    pub created_at: String,
    pub tracks: Vec<Track>,
}

impl Release {
    /// «Album» o «Singolo», per il testo sotto il titolo.
    pub fn kind_label(&self) -> &'static str {
        if self.is_album { "Album" } else { "Singolo" }
    }
}

/// Le uscite dei brani dati, dalla più recente. I brani di uno stesso album diventano un'uscita
/// sola; l'ordine è quello di `created_at` (testo ISO 8601, che si ordina come il tempo).
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

/// Le uscite degli artisti seguiti, dalla più recente.
pub fn from_followed<'a>(releases: &'a [Release], followed: &[FollowedArtist]) -> Vec<&'a Release> {
    releases.iter().filter(|r| followed.iter().any(|f| f.id == r.artist.id)).collect()
}

/// Quante uscite sono arrivate dopo l'ultima volta che si è guardato «Novità».
pub fn unread(news: &[&Release], seen: &str) -> usize {
    news.iter().filter(|r| r.created_at.as_str() > seen).count()
}

/// Una data ISO 8601 (`2026-05-01T10:00:00Z`) in parole brevi: «1 mag 2026».
pub fn date_label(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] =
        ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"];
    let year = iso.get(0..4)?;
    let month: usize = iso.get(5..7)?.parse().ok()?;
    let day: u32 = iso.get(8..10)?.parse().ok()?;
    let name = MONTHS.get(month.checked_sub(1)?)?;
    Some(format!("{day} {name} {year}"))
}

/// Un artista da mostrare in cerchio: chi è, con la sua immagine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistCard {
    pub artist: ArtistRef,
    pub cover_url: Option<String>,
    /// Brani del feed di questo artista: serve a ordinare chi ha più materiale.
    pub tracks: usize,
}

/// Gli artisti presenti nei brani dati, quelli con più brani prima. L'immagine di un artista è la
/// copertina di un suo brano.
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

/// I brani di un artista da mettere per primi: i più ascoltati da chi ascolta, poi i più recenti.
pub fn popular(tracks: &[Track], stats: &[PlayStat], count: usize) -> Vec<Track> {
    let plays = |t: &Track| stats.iter().find(|s| s.track.id == t.id).map_or(0, |s| s.plays);
    let mut ranked: Vec<&Track> = tracks.iter().collect();
    ranked.sort_by(|a, b| plays(b).cmp(&plays(a)).then_with(|| b.created_at.cmp(&a.created_at)));
    ranked.into_iter().take(count).cloned().collect()
}

/// Gli artisti degli album e dei brani dati, nell'ordine in cui compaiono (le copertine degli album
/// per prime: sono quelle più curate). Un artista compare una volta sola.
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
