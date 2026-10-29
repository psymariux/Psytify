//! Ciò che l'app sa di chi ascolta: gli artisti seguiti, gli album salvati, il feed della Home
//! (uscite, novità, mix, artisti), il carosello in cima, il timer di spegnimento e la riproduzione
//! automatica a fine coda. Il backend non ha utenti, quindi tutto si ricava dal catalogo e dal file
//! di configurazione.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use uuid::Uuid;

use super::{App, Message, Notice, Screen, seed};
use crate::api::models::ArtistRef;
use crate::api::{Album, ArtistPage, Track};
use crate::config::taste::{self, ArtistTaste, Mix, PlayStat};
use crate::config::{FollowedArtist, SavedAlbum};
use crate::feed::{self, ArtistCard, Release};

/// Uscite nel carosello in cima alla Home.
pub const HERO_COUNT: usize = 5;
/// Ogni quanto il carosello passa da solo all'uscita successiva.
pub const HERO_PERIOD: Duration = Duration::from_secs(8);
/// Durata del passaggio da un'uscita all'altra.
pub const HERO_SWAP: Duration = Duration::from_millis(420);
/// Mix mostrati nella Home.
const MIX_COUNT: usize = 4;
/// Artisti nella fila «I tuoi artisti».
const ARTIST_COUNT: usize = 20;
/// Brani aggiunti dalla riproduzione automatica.
const AUTOPLAY_COUNT: usize = 10;
/// Dopo quanto un brano si considera ascoltato.
const HEARD_AFTER: Duration = Duration::from_secs(30);
/// Brani di una radio.
const RADIO_SIZE: usize = 25;

/// Il feed della Home, calcolato quando cambiano i dati e non a ogni frame.
#[derive(Debug, Default)]
pub struct Feed {
    /// Tutte le uscite note, dalla più recente.
    pub releases: Vec<Release>,
    /// Le più recenti con una copertina da mostrare in grande: il carosello.
    pub hero: Vec<Release>,
    /// Le uscite degli artisti seguiti.
    pub news: Vec<Release>,
    pub mixes: Vec<Mix>,
    /// Gli artisti da mostrare in cerchio, e se sono «tuoi» (seguiti o ascoltati).
    pub artists: Vec<ArtistCard>,
    pub artists_are_yours: bool,
    /// I più ascoltati da chi ascolta, dal primo.
    pub top: Vec<PlayStat>,
}

impl Feed {
    /// Costruisce il feed dai brani recenti del catalogo, dagli artisti seguiti e dagli ascolti.
    pub fn build(tracks: &[Track], followed: &[FollowedArtist], stats: &[PlayStat], seed: u64) -> Self {
        let releases = feed::releases(tracks);
        let mut hero: Vec<Release> = releases.iter().filter(|r| r.cover_url.is_some()).cloned().collect();
        hero.truncate(HERO_COUNT);
        let news: Vec<Release> = feed::from_followed(&releases, followed).into_iter().cloned().collect();

        let mut artists: Vec<ArtistCard> = followed
            .iter()
            .map(|a| ArtistCard {
                artist: ArtistRef { id: a.id, name: a.name.clone() },
                cover_url: a.cover_url.clone(),
                tracks: 0,
            })
            .collect();
        for ArtistTaste { artist, cover_url, .. } in taste::top_artists(stats) {
            if !artists.iter().any(|a| a.artist.id == artist.id) {
                artists.push(ArtistCard { artist, cover_url, tracks: 0 });
            }
        }
        let artists_are_yours = !artists.is_empty();
        for card in feed::artists(tracks) {
            if !artists.iter().any(|a| a.artist.id == card.artist.id) {
                artists.push(card);
            }
        }
        artists.truncate(ARTIST_COUNT);

        Self {
            mixes: taste::mixes(stats, tracks, MIX_COUNT, seed),
            top: taste::top_tracks(stats, 10).into_iter().cloned().collect(),
            releases,
            hero,
            news,
            artists,
            artists_are_yours,
        }
    }
}

/// Il carosello della Home: quale uscita si vede, e da quando.
#[derive(Debug, Default)]
pub struct Hero {
    pub index: usize,
    pub since: Option<Instant>,
}

/// Dopo quanto il timer ferma la musica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepChoice {
    Minutes(u32),
    /// Alla fine del brano che suona.
    EndOfTrack,
}

impl SleepChoice {
    /// Le scelte del menu, in ordine.
    pub const ALL: [Self; 6] = [
        Self::Minutes(5),
        Self::Minutes(15),
        Self::Minutes(30),
        Self::Minutes(45),
        Self::Minutes(60),
        Self::EndOfTrack,
    ];

    pub fn label(self) -> String {
        match self {
            Self::Minutes(60) => "1 ora".into(),
            Self::Minutes(n) => format!("{n} minuti"),
            Self::EndOfTrack => "Alla fine del brano".into(),
        }
    }
}

/// Il timer attivo.
#[derive(Debug, Clone, Copy)]
pub struct SleepTimer {
    pub choice: SleepChoice,
    pub until: Option<Instant>,
}

impl SleepTimer {
    pub fn new(choice: SleepChoice, now: Instant) -> Self {
        let until = match choice {
            SleepChoice::Minutes(n) => Some(now + Duration::from_secs(u64::from(n) * 60)),
            SleepChoice::EndOfTrack => None,
        };
        Self { choice, until }
    }

    /// Il tempo che manca, in testo breve («12 min», «fine brano»).
    pub fn remaining_label(&self, now: Instant) -> String {
        match self.until {
            Some(until) => {
                let secs = until.saturating_duration_since(now).as_secs();
                if secs >= 60 { format!("{} min", secs.div_ceil(60)) } else { format!("{secs} s") }
            }
            None => "fine brano".into(),
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| until <= now)
    }
}

/// Dopo quanto un brano si considera ascoltato: 30 secondi, o metà se dura meno di un minuto.
pub fn heard_enough(position: Duration, length: Duration) -> bool {
    position >= HEARD_AFTER.min(length / 2)
}

impl App {
    /// Rifà il feed della Home dai dati che ci sono.
    pub(super) fn rebuild_feed(&mut self) {
        let Some(tracks) = self.home.recent.ready() else { return };
        self.home.feed = Feed::build(tracks, &self.followed, &self.stats, self.taste_seed);
        if self.home.hero.index >= self.home.feed.hero.len() {
            self.home.hero.index = 0;
        }
    }

    pub fn is_following(&self, artist_id: Uuid) -> bool {
        self.followed.iter().any(|a| a.id == artist_id)
    }

    pub fn has_saved(&self, album_id: Uuid) -> bool {
        self.saved_albums.iter().any(|a| a.id == album_id)
    }

    /// Uscite degli artisti seguiti arrivate dopo l'ultima visita a «Novità».
    pub fn unread_news(&self) -> usize {
        let news: Vec<&Release> = self.home.feed.news.iter().collect();
        feed::unread(&news, &self.news_seen)
    }

    /// Ciò che c'è in «Novità» si considera visto.
    pub(super) fn mark_news_seen(&mut self) {
        let newest = self.home.feed.news.iter().map(|r| r.created_at.as_str()).max();
        if let Some(newest) = newest
            && newest > self.news_seen.as_str()
        {
            self.news_seen = newest.to_string();
        }
    }

    pub(super) fn toggle_follow(&mut self, artist: FollowedArtist) -> Task<Message> {
        let name = artist.name.clone();
        // La prima volta che si segue qualcuno, le uscite già in catalogo non sono «novità».
        if self.news_seen.is_empty()
            && let Some(newest) = self.home.feed.releases.first()
        {
            self.news_seen.clone_from(&newest.created_at);
        }
        let now_following = taste::toggle_follow(&mut self.followed, artist);
        self.notice = Some(Notice::Info(if now_following {
            format!("Segui {name}: le sue nuove uscite compaiono in Novità")
        } else {
            format!("Non segui più {name}")
        }));
        self.rebuild_feed();
        self.persist()
    }

    pub(super) fn toggle_save_album(&mut self, album: SavedAlbum) -> Task<Message> {
        let title = album.title.clone();
        let saved = taste::toggle_saved_album(&mut self.saved_albums, album);
        self.notice = Some(Notice::Info(if saved {
            format!("«{title}» è nella tua libreria")
        } else {
            format!("«{title}» tolto dalla libreria")
        }));
        self.persist()
    }

    /// Riproduce un album dall'inizio: i suoi brani non sono nel feed per intero, si chiedono al server.
    pub(super) fn play_album(&mut self, id: Uuid) -> Task<Message> {
        let api = self.api();
        Task::perform(async move { api?.album(id).await }, Message::AlbumPlayLoaded)
    }

    pub(super) fn album_play_loaded(
        &mut self,
        result: Result<Album, crate::error::AppError>,
    ) -> Task<Message> {
        match result {
            Ok(album) if !album.tracks.is_empty() => {
                let urls = album.tracks.iter().filter_map(|t| t.cover_url.clone()).collect();
                let covers = self.fetch_covers(urls);
                let play = self.handle(Message::PlayTracks(Arc::from(album.tracks), 0));
                Task::batch([covers, play])
            }
            Ok(_) => Task::none(),
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                Task::none()
            }
        }
    }

    pub(super) fn play_mix(&mut self, index: usize) -> Task<Message> {
        let Some(mix) = self.home.feed.mixes.get(index) else { return Task::none() };
        let tracks: Arc<[Track]> = Arc::from(mix.tracks.as_slice());
        self.handle(Message::PlayTracks(tracks, 0))
    }

    /// Porta il carosello all'uscita `index` (e fa partire il passaggio).
    pub(super) fn hero_go(&mut self, index: usize) {
        let count = self.home.feed.hero.len();
        if count == 0 {
            return;
        }
        self.home.hero.index = index % count;
        self.home.hero.since = Some(Instant::now());
    }

    pub(super) fn hero_next(&mut self) {
        self.hero_go(self.home.hero.index + 1);
    }

    /// Il carosello è in passaggio da un'uscita all'altra.
    pub(super) fn hero_swapping(&self) -> bool {
        self.screen == Screen::Home && self.home.hero.since.is_some_and(|since| since.elapsed() < HERO_SWAP)
    }

    /// Il carosello gira da solo: sulla Home e se ha più di un'uscita.
    pub(super) fn hero_rotates(&self) -> bool {
        self.screen == Screen::Home && self.home.feed.hero.len() > 1
    }

    // ---------- Timer di spegnimento ----------

    pub(super) fn set_sleep(&mut self, choice: Option<SleepChoice>) {
        self.sleep = choice.map(|choice| SleepTimer::new(choice, Instant::now()));
        self.notice = Some(Notice::Info(match choice {
            Some(choice) => format!("Timer di spegnimento: {}", choice.label().to_lowercase()),
            None => "Timer di spegnimento disattivato".into(),
        }));
    }

    /// Un timer a tempo scaduto mentre la musica era ferma non deve fermarla alla ripresa.
    pub(super) fn drop_expired_sleep(&mut self) {
        if self.sleep.is_some_and(|timer| timer.expired(Instant::now())) {
            self.sleep = None;
        }
    }

    /// Il timer a tempo è scaduto: mette in pausa. `true` se ha fermato la musica.
    pub(super) fn sleep_due(&mut self) -> bool {
        if !self.sleep.is_some_and(|timer| timer.expired(Instant::now())) {
            return false;
        }
        self.sleep = None;
        self.pause_for_sleep();
        true
    }

    /// Il brano è finito e il timer era «alla fine del brano»: si ferma qui, e Play riparte dal
    /// brano dopo. `true` se l'ha fatto.
    pub(super) fn sleep_at_end_of_track(&mut self) -> bool {
        if !self.sleep.is_some_and(|timer| timer.choice == SleepChoice::EndOfTrack) {
            return false;
        }
        self.sleep = None;
        self.stop();
        self.notice = Some(Notice::Info("Buonanotte: la musica si è fermata".into()));
        true
    }

    fn pause_for_sleep(&mut self) {
        if let Some(audio) = &self.audio {
            audio.pause();
        }
        self.playback.state = crate::player::PlaybackState::Paused;
        self.sync_video(None);
        self.notice = Some(Notice::Info("Buonanotte: la musica si è fermata".into()));
    }

    /// Un brano conta come ascoltato dopo 30 secondi (o metà, se è più corto): saltarlo non lo conta.
    pub(super) fn count_play_when_heard(&mut self, position: Duration) {
        if self.play_counted {
            return;
        }
        let Some(track) = self.queue.current() else { return };
        if !heard_enough(position, track.duration()) {
            return;
        }
        self.play_counted = true;
        let track = track.clone();
        taste::count_play(&mut self.stats, &track, taste::now_secs());
        // Anche il profilo lo conta, se si è collegati (`App::social_sync`).
        self.social.listen_due = Some(track.id);
    }

    // ---------- Radio ----------

    /// Avvia una radio da un brano: lui, poi altri dello stesso artista e qualche brano da scoprire.
    pub(super) fn start_radio(&mut self, track: Track) -> Task<Message> {
        let artist = track.artist.id;
        let api = self.api();
        Task::perform(async move { api?.artist(artist).await }, move |result| {
            Message::RadioLoaded(Box::new(track.clone()), result)
        })
    }

    pub(super) fn radio_loaded(
        &mut self,
        start: Track,
        result: Result<ArtistPage, crate::error::AppError>,
    ) -> Task<Message> {
        let own = result.map(|page| page.tracks).unwrap_or_default();
        let pool = self.home.recent.ready().map_or(&[][..], |tracks| &tracks[..]);
        let queue = taste::radio(&start, &own, pool, RADIO_SIZE, seed());
        let urls: Vec<String> = queue.iter().filter_map(|t| t.cover_url.clone()).collect();
        self.notice = Some(Notice::Info(format!("Radio di «{}»", start.title)));
        let covers = self.fetch_covers(urls);
        let play = self.handle(Message::PlayTracks(Arc::from(queue), 0));
        Task::batch([covers, play])
    }

    // ---------- Riproduzione automatica ----------

    /// La coda è finita: se la riproduzione automatica è attiva, chiede altri brani dell'artista.
    pub(super) fn queue_ended(&mut self) -> Task<Message> {
        let Some(artist) = self.queue.current().map(|t| t.artist.id) else {
            self.stop();
            return Task::none();
        };
        // In una jam la coda è del gruppo: non si aggiunge nulla da soli.
        if !self.autoplay || self.autoplay_busy || self.jam.is_some() {
            self.stop();
            return Task::none();
        }
        self.autoplay_busy = true;
        let api = self.api();
        Task::perform(async move { api?.artist(artist).await }, Message::AutoplayLoaded)
    }

    pub(super) fn autoplay_loaded(
        &mut self,
        result: Result<ArtistPage, crate::error::AppError>,
    ) -> Task<Message> {
        self.autoplay_busy = false;
        let queued: Vec<Uuid> = self.queue.tracks_in_order().iter().map(|t| t.id).collect();
        let pool: Vec<Track> = match result {
            Ok(page) => page.tracks,
            Err(_) => Vec::new(),
        };
        let mut fresh: Vec<Track> = pool.into_iter().filter(|t| !queued.contains(&t.id)).collect();
        if fresh.is_empty() {
            // L'artista non ha altro: si passa alle uscite recenti del catalogo.
            let current = self.queue.current().map(|t| t.artist.id);
            fresh = self
                .home
                .recent
                .ready()
                .map(|tracks| {
                    tracks
                        .iter()
                        .filter(|t| !queued.contains(&t.id) && Some(t.artist.id) != current)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
        }
        if fresh.is_empty() {
            self.stop();
            return Task::none();
        }
        taste::shuffle(&mut fresh, seed());
        fresh.truncate(AUTOPLAY_COUNT);
        let urls = fresh.iter().filter_map(|t| t.cover_url.clone()).collect();
        for track in fresh {
            self.queue.enqueue(track);
        }
        self.notice = Some(Notice::Info("Riproduzione automatica: ancora musica come questa".into()));
        let covers = self.fetch_covers(urls);
        let next = if self.queue.advance(crate::player::Advance::Finished).is_some() {
            self.start_current()
        } else {
            self.stop();
            Task::none()
        };
        Task::batch([covers, next, self.persist()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

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
    fn a_song_counts_after_thirty_seconds() {
        let long = Duration::from_secs(240);
        assert!(!heard_enough(Duration::from_secs(29), long));
        assert!(heard_enough(Duration::from_secs(30), long));
    }

    #[test]
    fn a_short_clip_counts_at_half() {
        let clip = Duration::from_secs(20);
        assert!(!heard_enough(Duration::from_secs(9), clip));
        assert!(heard_enough(Duration::from_secs(10), clip));
    }

    #[test]
    fn a_minute_timer_expires_after_its_time() {
        let start = Instant::now();
        let timer = SleepTimer::new(SleepChoice::Minutes(15), start);
        assert!(!timer.expired(start + Duration::from_secs(14 * 60)));
        assert!(timer.expired(start + Duration::from_secs(15 * 60)));
        assert_eq!(timer.remaining_label(start), "15 min");
    }

    #[test]
    fn the_end_of_track_timer_never_expires_by_itself() {
        let timer = SleepTimer::new(SleepChoice::EndOfTrack, Instant::now());
        assert!(!timer.expired(Instant::now() + Duration::from_secs(3600)));
        assert_eq!(timer.remaining_label(Instant::now()), "fine brano");
    }

    #[test]
    fn the_last_seconds_show_seconds() {
        let start = Instant::now();
        let timer = SleepTimer::new(SleepChoice::Minutes(5), start);
        assert_eq!(timer.remaining_label(start + Duration::from_secs(5 * 60 - 20)), "20 s");
    }
}
