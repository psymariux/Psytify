//! Ciò che l'app impara da chi ascolta, solo su questo computer: gli artisti seguiti, gli album
//! salvati, quanti ascolti ha ogni brano e i «mix» che ne vengono. Il backend non ha utenti per
//! questo, quindi tutto vive nel file di configurazione; ogni lista ha un tetto.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::Track;
use crate::api::models::ArtistRef;

/// Artisti seguiti al massimo.
pub const MAX_FOLLOWED: usize = 200;
/// Album salvati al massimo.
pub const MAX_SAVED_ALBUMS: usize = 200;
/// Brani di cui si ricordano gli ascolti.
pub const MAX_STATS: usize = 150;
/// Brani di un mix.
pub const MIX_SIZE: usize = 24;

/// Un artista seguito, con ciò che serve a mostrarlo senza interrogare il backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowedArtist {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// Un album salvato nella libreria.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedAlbum {
    pub id: Uuid,
    pub title: String,
    pub artist: ArtistRef,
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// Quante volte è partito un brano, e quando l'ultima.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayStat {
    pub track: Track,
    pub plays: u32,
    /// Secondi dal 1970.
    pub last_played: u64,
}

/// Un artista con i suoi ascolti, per «i tuoi artisti» e i mix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistTaste {
    pub artist: ArtistRef,
    pub plays: u32,
    pub cover_url: Option<String>,
}

/// Un mix: un nome, i brani e i nomi degli artisti da mostrare sotto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mix {
    pub title: String,
    pub subtitle: String,
    pub tracks: Vec<Track>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Segue o smette di seguire. Restituisce `true` se ora lo segue.
pub fn toggle_follow(followed: &mut Vec<FollowedArtist>, artist: FollowedArtist) -> bool {
    match followed.iter().position(|a| a.id == artist.id) {
        Some(at) => {
            followed.remove(at);
            false
        }
        None => {
            followed.insert(0, artist);
            followed.truncate(MAX_FOLLOWED);
            true
        }
    }
}

/// Salva o toglie un album. Restituisce `true` se ora è salvato.
pub fn toggle_saved_album(saved: &mut Vec<SavedAlbum>, album: SavedAlbum) -> bool {
    match saved.iter().position(|a| a.id == album.id) {
        Some(at) => {
            saved.remove(at);
            false
        }
        None => {
            saved.insert(0, album);
            saved.truncate(MAX_SAVED_ALBUMS);
            true
        }
    }
}

/// Conta un ascolto. La lista resta ordinata (più ascolti prima, a parità il più recente) e
/// limitata: chi ha pochi ascolti e vecchi esce per primo.
pub fn count_play(stats: &mut Vec<PlayStat>, track: &Track, now: u64) {
    match stats.iter_mut().find(|s| s.track.id == track.id) {
        Some(stat) => {
            stat.plays = stat.plays.saturating_add(1);
            stat.last_played = now;
            stat.track = track.clone();
        }
        None => stats.push(PlayStat { track: track.clone(), plays: 1, last_played: now }),
    }
    stats.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.last_played.cmp(&a.last_played)));
    stats.truncate(MAX_STATS);
}

/// I brani più ascoltati, dal primo.
pub fn top_tracks(stats: &[PlayStat], count: usize) -> Vec<&PlayStat> {
    stats.iter().take(count).collect()
}

/// Gli artisti più ascoltati: gli ascolti dei loro brani sommati.
pub fn top_artists(stats: &[PlayStat]) -> Vec<ArtistTaste> {
    let mut artists: Vec<ArtistTaste> = Vec::new();
    for stat in stats {
        match artists.iter_mut().find(|a| a.artist.id == stat.track.artist.id) {
            Some(known) => {
                known.plays = known.plays.saturating_add(stat.plays);
                if known.cover_url.is_none() {
                    known.cover_url.clone_from(&stat.track.cover_url);
                }
            }
            None => artists.push(ArtistTaste {
                artist: stat.track.artist.clone(),
                plays: stat.plays,
                cover_url: stat.track.cover_url.clone(),
            }),
        }
    }
    artists.sort_by(|a, b| b.plays.cmp(&a.plays).then_with(|| a.artist.name.cmp(&b.artist.name)));
    artists
}

/// Ascolti totali, per la riga «hai ascoltato N brani».
pub fn total_plays(stats: &[PlayStat]) -> u64 {
    stats.iter().map(|s| u64::from(s.plays)).sum()
}

/// Un generatore di numeri casuali minimo (xorshift): basta a mescolare, e il seme lo rende
/// ripetibile nei test.
struct Shuffler(u64);

impl Shuffler {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for at in (1..items.len()).rev() {
            let other = (self.next() % (at as u64 + 1)) as usize;
            items.swap(at, other);
        }
    }
}

/// Mescola `items` in modo ripetibile: lo stesso seme dà lo stesso ordine.
pub fn shuffle<T>(items: &mut [T], seed: u64) {
    Shuffler(seed | 1).shuffle(items);
}

/// La coda di una «radio» partita da `start`: il brano, poi altri brani dello stesso artista e
/// qualche brano di altri artisti da scoprire, mescolati. `own` sono i brani dell'artista, `pool` il
/// catalogo recente.
pub fn radio(start: &Track, own: &[Track], pool: &[Track], size: usize, seed: u64) -> Vec<Track> {
    let mut rng = Shuffler(seed | 1);
    let mut same: Vec<Track> = own.iter().filter(|t| t.id != start.id).cloned().collect();
    rng.shuffle(&mut same);
    let mut other: Vec<Track> =
        pool.iter().filter(|t| t.id != start.id && t.artist.id != start.artist.id).cloned().collect();
    rng.shuffle(&mut other);
    // Due terzi dello stesso artista, il resto da altri: la radio resta riconoscibile ma varia.
    let from_others = (size.saturating_sub(1) / 3).min(other.len());
    let from_same = size.saturating_sub(1).saturating_sub(from_others).min(same.len());
    let mut queue = vec![start.clone()];
    queue.extend(same.into_iter().take(from_same));
    queue.extend(other.into_iter().take(from_others));
    queue
}

/// Fino a `count` mix, uno per ognuno dei tuoi artisti più ascoltati: i brani di quell'artista
/// (quelli che hai ascoltato per primi, poi il resto del catalogo), mescolati, con in coda qualche
/// brano di altri artisti da scoprire. `pool` è il catalogo recente.
pub fn mixes(stats: &[PlayStat], pool: &[Track], count: usize, seed: u64) -> Vec<Mix> {
    let mut rng = Shuffler(seed | 1);
    let mut result = Vec::new();
    for taste in top_artists(stats).into_iter().take(count) {
        let mut own: Vec<Track> = Vec::new();
        let heard = stats.iter().map(|s| &s.track);
        for track in heard.chain(pool.iter()) {
            if track.artist.id == taste.artist.id && !own.iter().any(|t| t.id == track.id) {
                own.push(track.clone());
            }
        }
        rng.shuffle(&mut own);
        let mut others: Vec<Track> =
            pool.iter().filter(|t| t.artist.id != taste.artist.id).cloned().collect();
        rng.shuffle(&mut others);
        let room = MIX_SIZE.saturating_sub(own.len()).min(others.len());
        let mut tracks = own;
        tracks.extend(others.into_iter().take(room.min(MIX_SIZE / 3)));
        tracks.truncate(MIX_SIZE);
        if tracks.len() < 2 {
            continue;
        }
        let mut names: Vec<&str> = Vec::new();
        for track in &tracks {
            if !names.contains(&track.artist.name.as_str()) {
                names.push(&track.artist.name);
            }
        }
        let subtitle = match names.len() {
            0 | 1 => taste.artist.name.clone(),
            2 => format!("{} e {}", names[0], names[1]),
            n => format!("{}, {} e altri {}", names[0], names[1], n - 2),
        };
        result.push(Mix { title: format!("Mix di {}", taste.artist.name), subtitle, tracks });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

    fn artist(n: u128) -> FollowedArtist {
        FollowedArtist { id: Uuid::from_u128(n), name: format!("Artista {n}"), cover_url: None }
    }

    #[test]
    fn following_twice_unfollows_and_the_newest_comes_first() {
        let mut followed = Vec::new();
        assert!(toggle_follow(&mut followed, artist(1)));
        assert!(toggle_follow(&mut followed, artist(2)));
        assert_eq!(followed[0].id, artist(2).id);
        assert!(!toggle_follow(&mut followed, artist(2)));
        assert_eq!(followed.len(), 1);
    }

    #[test]
    fn the_followed_list_is_capped() {
        let mut followed = Vec::new();
        for n in 0..(MAX_FOLLOWED as u128 + 10) {
            toggle_follow(&mut followed, artist(n));
        }
        assert_eq!(followed.len(), MAX_FOLLOWED);
    }

    #[test]
    fn saving_an_album_twice_removes_it() {
        let mut saved = Vec::new();
        let album = SavedAlbum {
            id: Uuid::from_u128(7),
            title: "Banchi".into(),
            artist: ArtistRef { id: Uuid::from_u128(1), name: "Marea".into() },
            cover_url: None,
        };
        assert!(toggle_saved_album(&mut saved, album.clone()));
        assert!(!toggle_saved_album(&mut saved, album));
        assert!(saved.is_empty());
    }

    #[test]
    fn plays_are_counted_and_the_most_played_comes_first() {
        let mut stats = Vec::new();
        count_play(&mut stats, &track(1), 100);
        count_play(&mut stats, &track(2), 200);
        count_play(&mut stats, &track(1), 300);
        assert_eq!((stats[0].track.id, stats[0].plays), (track(1).id, 2));
        assert_eq!(total_plays(&stats), 3);
    }

    #[test]
    fn equal_plays_put_the_latest_first() {
        let mut stats = Vec::new();
        count_play(&mut stats, &track(1), 100);
        count_play(&mut stats, &track(2), 200);
        assert_eq!(stats[0].track.id, track(2).id);
    }

    #[test]
    fn the_stats_are_capped_and_keep_the_most_played() {
        let mut stats = Vec::new();
        for _ in 0..5 {
            count_play(&mut stats, &track(1), 1);
        }
        for n in 2..(MAX_STATS as u128 + 20) {
            count_play(&mut stats, &track(n), n as u64);
        }
        assert_eq!(stats.len(), MAX_STATS);
        assert_eq!(stats[0].track.id, track(1).id);
    }

    #[test]
    fn artists_sum_the_plays_of_their_tracks() {
        let mut one = track(1);
        let mut two = track(2);
        let mut three = track(3);
        let marea = ArtistRef { id: Uuid::from_u128(50), name: "Marea".into() };
        one.artist = marea.clone();
        two.artist = marea.clone();
        three.artist = ArtistRef { id: Uuid::from_u128(60), name: "Sale".into() };
        let mut stats = Vec::new();
        for t in [&one, &two, &three, &three] {
            count_play(&mut stats, t, 1);
        }
        let top = top_artists(&stats);
        assert_eq!((top[0].artist.id, top[0].plays), (marea.id, 2));
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn a_mix_is_made_of_the_artist_and_a_few_others() {
        let marea = ArtistRef { id: Uuid::from_u128(50), name: "Marea".into() };
        let mut pool: Vec<Track> = (1..=30).map(track).collect();
        for t in pool.iter_mut().take(10) {
            t.artist = marea.clone();
        }
        let mut stats = Vec::new();
        count_play(&mut stats, &pool[0], 1);
        let mixes = mixes(&stats, &pool, 3, 9);
        assert_eq!(mixes.len(), 1);
        let mix = &mixes[0];
        assert_eq!(mix.title, "Mix di Marea");
        assert!(mix.tracks.len() <= MIX_SIZE);
        assert!(mix.tracks.iter().filter(|t| t.artist.id == marea.id).count() >= 10);
    }

    #[test]
    fn a_radio_starts_with_its_track_and_mixes_in_others() {
        let marea = ArtistRef { id: Uuid::from_u128(50), name: "Marea".into() };
        let mut pool: Vec<Track> = (1..=40).map(track).collect();
        for t in pool.iter_mut().take(15) {
            t.artist = marea.clone();
        }
        let own: Vec<Track> = pool.iter().take(15).cloned().collect();
        let queue = radio(&own[0], &own, &pool, 13, 7);
        assert_eq!(queue[0].id, own[0].id);
        assert_eq!(queue.len(), 13);
        let others = queue.iter().filter(|t| t.artist.id != marea.id).count();
        assert!(others >= 3, "{others}");
        let mut ids: Vec<_> = queue.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), queue.len(), "no repeats");
    }

    #[test]
    fn a_radio_with_nothing_else_is_just_the_track() {
        let only = track(1);
        let alone = std::slice::from_ref(&only);
        assert_eq!(radio(&only, alone, alone, 20, 1).len(), 1);
    }

    #[test]
    fn shuffling_is_repeatable_and_keeps_every_item() {
        let mut a: Vec<u32> = (0..20).collect();
        let mut b = a.clone();
        shuffle(&mut a, 3);
        shuffle(&mut b, 3);
        assert_eq!(a, b);
        a.sort_unstable();
        assert_eq!(a, (0..20).collect::<Vec<u32>>());
    }

    #[test]
    fn a_mix_needs_at_least_two_tracks() {
        let mut stats = Vec::new();
        count_play(&mut stats, &track(1), 1);
        assert!(mixes(&stats, &[], 3, 1).is_empty());
    }

    #[test]
    fn the_same_seed_gives_the_same_mix() {
        let pool: Vec<Track> = (1..=20).map(track).collect();
        let mut stats = Vec::new();
        count_play(&mut stats, &pool[0], 1);
        assert_eq!(mixes(&stats, &pool, 1, 5), mixes(&stats, &pool, 1, 5));
    }
}
