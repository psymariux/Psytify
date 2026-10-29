//! Le pagine degli artisti si tengono in memoria e si caricano in anticipo, come fa Spotify: aprire
//! un artista già visto, o uno che sta in Home, fra i seguiti o nei risultati di una ricerca, è
//! immediato. Una pagina vecchia si mostra subito e si aggiorna in silenzio. Qui anche la correzione
//! delle copertine salvate (coda, recenti, playlist, ascolti) con quelle appena lette dal catalogo.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use uuid::Uuid;

use super::{App, Load, Message};
use crate::api::{ArtistPage, Track};
use crate::error::AppError;

/// Pagine tenute in memoria: oltre, esce la meno recente.
const CACHED_PAGES: usize = 40;
/// Dopo quanto una pagina in cache si riscarica (in silenzio) aprendola.
const STALE_AFTER: Duration = Duration::from_secs(90);
/// Pagine caricate in anticipo per volta: l'abbondanza costerebbe al server.
const PREFETCH_BATCH: usize = 8;
/// Copertine scaricate subito per una pagina in anticipo (le prime sono quelle che si vedono).
const WARM_COVERS: usize = 24;

/// Come si filtra la discografia nella pagina di un artista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Discography {
    #[default]
    All,
    Albums,
    Singles,
}

impl Discography {
    pub const ALL: [Self; 3] = [Self::All, Self::Albums, Self::Singles];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Tutto",
            Self::Albums => "Album",
            Self::Singles => "Singoli",
        }
    }
}

/// Cache, caricamenti in corso e scelte di vista della pagina artista.
#[derive(Debug, Default)]
pub struct ArtistsState {
    pages: HashMap<Uuid, (Instant, Arc<ArtistPage>)>,
    /// Dal meno al più recente: l'ordine di uscita dalla cache.
    order: Vec<Uuid>,
    loading: HashSet<Uuid>,
    pub filter: Discography,
    /// «Popolari» mostra dieci brani invece di cinque.
    pub more_popular: bool,
}

impl ArtistsState {
    pub fn get(&self, id: Uuid) -> Option<(&Arc<ArtistPage>, bool)> {
        self.pages.get(&id).map(|(at, page)| (page, at.elapsed() > STALE_AFTER))
    }

    pub fn store(&mut self, page: ArtistPage, now: Instant) -> Arc<ArtistPage> {
        let id = page.id;
        let page = Arc::new(page);
        self.pages.insert(id, (now, page.clone()));
        self.order.retain(|&known| known != id);
        self.order.push(id);
        while self.order.len() > CACHED_PAGES {
            let oldest = self.order.remove(0);
            self.pages.remove(&oldest);
        }
        page
    }

    /// Fra `ids`, quelli da caricare in anticipo: non in cache e non già in arrivo, al massimo
    /// `PREFETCH_BATCH`, senza ripetizioni. Li segna in arrivo.
    pub fn claim(&mut self, ids: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
        let mut wanted = Vec::new();
        for id in ids {
            if wanted.len() == PREFETCH_BATCH {
                break;
            }
            if !self.pages.contains_key(&id) && !wanted.contains(&id) && self.loading.insert(id) {
                wanted.push(id);
            }
        }
        wanted
    }

    pub fn arrived(&mut self, id: Uuid) {
        self.loading.remove(&id);
    }
}

/// Cambia la copertina dei brani che il catalogo ha appena rimandato con un'altra (o senza).
/// `true` se qualcosa è cambiato.
pub fn patch_covers(tracks: &mut [Track], fresh: &HashMap<Uuid, Option<String>>) -> bool {
    let mut changed = false;
    for track in tracks {
        if let Some(cover) = fresh.get(&track.id)
            && track.cover_url != *cover
        {
            track.cover_url = cover.clone();
            changed = true;
        }
    }
    changed
}

impl App {
    /// Apre la pagina di un artista: dalla cache se c'è, altrimenti dalla rete.
    pub(super) fn open_artist(&mut self, id: Uuid) -> Task<Message> {
        // Un'altra pagina artista: il filtro e «mostra altri» ripartono.
        if !matches!(&self.artist, Some((open, _)) if *open == id) {
            self.artists.filter = Discography::All;
            self.artists.more_popular = false;
        }
        match self.artists.get(id).map(|(page, stale)| (page.clone(), stale)) {
            Some((page, stale)) => {
                self.artist = Some((id, Load::Ready((*page).clone())));
                let covers = self.fetch_covers(page_covers(&page, usize::MAX));
                let refresh = if stale { self.refresh_artist(id) } else { Task::none() };
                let extras = self.load_artist_extras(id, page.name.clone());
                Task::batch([covers, refresh, extras])
            }
            None => self.load_artist(id),
        }
    }

    /// Riscarica una pagina già in cache senza mostrare un caricamento.
    fn refresh_artist(&mut self, id: Uuid) -> Task<Message> {
        if !self.artists.loading.insert(id) {
            return Task::none();
        }
        let api = self.api();
        Task::perform(async move { api?.artist(id).await }, move |result| {
            Message::ArtistPrefetched(id, result)
        })
    }

    /// Carica in anticipo le pagine di questi artisti, se non le ha già.
    pub(super) fn prefetch_artists(&mut self, ids: impl IntoIterator<Item = Uuid>) -> Task<Message> {
        let wanted = self.artists.claim(ids);
        Task::batch(wanted.into_iter().map(|id| {
            let api = self.api();
            Task::perform(async move { api?.artist(id).await }, move |result| {
                Message::ArtistPrefetched(id, result)
            })
        }))
    }

    /// Gli artisti che la Home mette in vista (seguiti e primi del catalogo): si caricano subito,
    /// così aprirne uno è immediato.
    pub(super) fn prefetch_for_home(&mut self) -> Task<Message> {
        let followed = self.followed.iter().take(4).map(|a| a.id);
        let shown = self.home.feed.artists.iter().take(6).map(|card| card.artist.id);
        let ids: Vec<Uuid> = followed.chain(shown).collect();
        self.prefetch_artists(ids)
    }

    /// Una pagina arrivata in anticipo (o un aggiornamento in silenzio): va in cache, e se è
    /// quella aperta la sostituisce senza che la vista salti.
    pub(super) fn artist_prefetched(
        &mut self,
        id: Uuid,
        result: Result<ArtistPage, AppError>,
    ) -> Task<Message> {
        self.artists.arrived(id);
        let Ok(page) = result else { return Task::none() };
        let page = self.artists.store(page, Instant::now());
        let showing = matches!(&self.artist, Some((open, Load::Ready(_) | Load::Idle)) if *open == id);
        if showing {
            self.artist = Some((id, Load::Ready((*page).clone())));
        }
        let covers = self.fetch_covers(page_covers(&page, WARM_COVERS));
        let fixed = self.refresh_saved_covers(&page.tracks);
        Task::batch([covers, fixed])
    }

    /// Una pagina arrivata da un'apertura vera: in cache e nelle copertine salvate.
    pub(super) fn artist_loaded(&mut self, id: Uuid, result: Result<ArtistPage, AppError>) -> Task<Message> {
        self.artists.arrived(id);
        let mut urls = Vec::new();
        let mut fixed = Task::none();
        if let Some((current, load)) = &mut self.artist
            && *current == id
        {
            *load = match result {
                Ok(page) => {
                    let page = self.artists.store(page, Instant::now());
                    urls = page_covers(&page, usize::MAX);
                    Load::Ready((*page).clone())
                }
                Err(err) => Load::Failed(err),
            };
            if let Load::Ready(page) = &*load {
                let tracks = page.tracks.clone();
                let name = page.name.clone();
                let portrait = crate::ui::pages::artist::portrait_url(page).map(str::to_owned);
                self.update_followed_cover(id, portrait);
                fixed = Task::batch([self.refresh_saved_covers(&tracks), self.load_artist_extras(id, name)]);
            }
        }
        Task::batch([self.fetch_covers(urls), fixed])
    }

    /// L'immagine di un artista seguito segue quella che il catalogo ha adesso.
    fn update_followed_cover(&mut self, id: Uuid, portrait: Option<String>) {
        if let Some(followed) = self.followed.iter_mut().find(|a| a.id == id)
            && followed.cover_url != portrait
        {
            followed.cover_url = portrait;
        }
    }

    /// Corregge le copertine di ciò che si è salvato (coda, recenti, mi piace, playlist, ascolti)
    /// con quelle dei brani appena letti dal catalogo: un brano salvato prima che il server
    /// smettesse di dare le onde di Archive come copertina ne conserverebbe l'indirizzo.
    pub(super) fn refresh_saved_covers(&mut self, fresh: &[Track]) -> Task<Message> {
        if fresh.is_empty() {
            return Task::none();
        }
        let fresh: HashMap<Uuid, Option<String>> =
            fresh.iter().map(|t| (t.id, t.cover_url.clone())).collect();
        let mut changed = patch_covers(&mut self.recent, &fresh);
        changed |= patch_covers(&mut self.liked, &fresh);
        for playlist in &mut self.playlists {
            changed |= patch_covers(&mut playlist.tracks, &fresh);
        }
        for stat in &mut self.stats {
            changed |= patch_covers(std::slice::from_mut(&mut stat.track), &fresh);
        }
        changed |= self.queue.update_tracks(|track| patch_covers(std::slice::from_mut(track), &fresh));
        if changed { self.persist() } else { Task::none() }
    }
}

/// Le copertine di una pagina artista, nell'ordine in cui si vedono: la foto, gli album, i brani.
fn page_covers(page: &ArtistPage, limit: usize) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for url in page
        .image_url
        .as_deref()
        .into_iter()
        .chain(page.albums.iter().filter_map(|a| a.cover_url.as_deref()))
        .chain(page.tracks.iter().filter_map(|t| t.cover_url.as_deref()))
    {
        if !urls.iter().any(|known| known == url) {
            urls.push(url.to_owned());
        }
        if urls.len() == limit {
            break;
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track as fixture;

    fn page(n: u128) -> ArtistPage {
        ArtistPage {
            id: Uuid::from_u128(n),
            name: format!("Artista {n}"),
            image_url: None,
            albums: Vec::new(),
            tracks: Vec::new(),
        }
    }

    fn track(n: u128, cover: Option<&str>) -> Track {
        Track { cover_url: cover.map(str::to_owned), ..fixture(n) }
    }

    #[test]
    fn a_cached_page_is_fresh_then_stale() {
        let mut state = ArtistsState::default();
        let now = Instant::now();
        state.store(page(1), now);
        assert_eq!(state.get(Uuid::from_u128(1)).map(|(_, stale)| stale), Some(false));
        state.store(page(2), now.checked_sub(STALE_AFTER + Duration::from_secs(1)).unwrap_or(now));
        assert_eq!(state.get(Uuid::from_u128(2)).map(|(_, stale)| stale), Some(true));
        assert!(state.get(Uuid::from_u128(3)).is_none());
    }

    #[test]
    fn the_cache_forgets_the_oldest_page() {
        let mut state = ArtistsState::default();
        let now = Instant::now();
        for n in 0..=CACHED_PAGES as u128 {
            state.store(page(n), now);
        }
        assert!(state.get(Uuid::from_u128(0)).is_none());
        assert!(state.get(Uuid::from_u128(CACHED_PAGES as u128)).is_some());
        // Rimetterla la porta in fondo: non è più la prossima a uscire.
        state.store(page(1), now);
        state.store(page(1000), now);
        assert!(state.get(Uuid::from_u128(1)).is_some());
    }

    #[test]
    fn prefetching_skips_known_and_pending_artists() {
        let mut state = ArtistsState::default();
        state.store(page(1), Instant::now());
        let ids = [1, 2, 2, 3].map(Uuid::from_u128);
        assert_eq!(state.claim(ids), vec![Uuid::from_u128(2), Uuid::from_u128(3)]);
        // Già in arrivo: nessuna seconda richiesta.
        assert!(state.claim([Uuid::from_u128(2)]).is_empty());
        state.arrived(Uuid::from_u128(2));
        assert_eq!(state.claim([Uuid::from_u128(2)]), vec![Uuid::from_u128(2)]);
    }

    #[test]
    fn prefetching_is_capped_per_batch() {
        let mut state = ArtistsState::default();
        let wanted = state.claim((0..50).map(Uuid::from_u128));
        assert_eq!(wanted.len(), PREFETCH_BATCH);
    }

    #[test]
    fn covers_follow_the_catalogue() {
        let mut saved = vec![track(1, Some("onda")), track(2, Some("vera")), track(3, Some("altra"))];
        let fresh =
            HashMap::from([(Uuid::from_u128(1), None), (Uuid::from_u128(2), Some("vera".to_owned()))]);
        assert!(patch_covers(&mut saved, &fresh));
        assert_eq!(saved[0].cover_url, None, "l'onda di Archive non è più una copertina");
        assert_eq!(saved[1].cover_url.as_deref(), Some("vera"));
        assert_eq!(saved[2].cover_url.as_deref(), Some("altra"), "un brano non riletto resta com'è");
        assert!(!patch_covers(&mut saved, &fresh), "la seconda volta non cambia nulla");
    }

    #[test]
    fn page_covers_are_unique_and_capped() {
        let mut p = page(1);
        p.tracks = vec![track(1, Some("a")), track(2, Some("a")), track(3, Some("b")), track(4, None)];
        assert_eq!(page_covers(&p, usize::MAX), vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(page_covers(&p, 1), vec!["a".to_owned()]);
    }
}
