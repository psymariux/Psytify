//! Il catalogo: Home, ricerca, libreria, album, artisti, librerie di Archive, catalogo ufficiale e
//! copertine. Dal passo 4.4 lo possiede il core (`core/src/state/catalog.rs`): qui i messaggi
//! diventano `CatalogCmd` e l'app legge le copie della sua vista (`App::home`, `App::search`, …),
//! che [`App::catalog_published`] rinnova a ogni pubblicazione. Le copertine, che sono texture della
//! finestra, le chiede l'interfaccia a ogni pubblicazione che porta dati nuovi.

use std::sync::Arc;

use iced::Task;
use psytify_core::Command;
use psytify_core::command::{Artwork, CatalogCmd, PlaybackCmd};
use psytify_core::shared::Shared;
use psytify_core::snapshot::{CatalogEvent, CatalogView};

use super::artists::page_covers;
use super::{
    App, Hero, Load, Message, Notice, Screen, album_covers, clean_query, inserted_char, track_covers,
};

/// Copertine scaricate subito per una pagina artista arrivata in anticipo (le prime sono quelle che
/// si vedono).
const WARM_COVERS: usize = 24;

/// Lo stesso dato: i due `Arc` sono la stessa allocazione (o lo stato non ne ha, ed è uguale).
fn same<T: ?Sized>(a: &Load<Arc<T>>, b: &Load<Arc<T>>) -> bool {
    match (a, b) {
        (Load::Ready(a), Load::Ready(b)) => Arc::ptr_eq(a, b),
        (Load::Idle, Load::Idle) | (Load::Loading, Load::Loading) => true,
        (Load::Failed(a), Load::Failed(b)) => a == b,
        _ => false,
    }
}

/// Come [`same`], per una lista condivisa.
fn same_list<T>(a: &Load<Shared<T>>, b: &Load<Shared<T>>) -> bool {
    match (a, b) {
        (Load::Ready(a), Load::Ready(b)) => a.same(b),
        (Load::Idle, Load::Idle) | (Load::Loading, Load::Loading) => true,
        (Load::Failed(a), Load::Failed(b)) => a == b,
        _ => false,
    }
}

impl App {
    /// Un comando al catalogo del core. Il core lo applica al suo turno e pubblica la vista nuova: chi
    /// lo manda non la trova ancora in `App::home` & co., e ciò che deve fare con il risultato
    /// (le copertine, aprire l'artista, un avviso) lo fa in [`App::catalog_published`].
    pub(super) fn send_catalog(&mut self, command: CatalogCmd) -> Task<Message> {
        self.send_core(Command::Catalog(command))
    }

    /// Copia nell'app tutta la vista del catalogo dell'ultima istantanea, senza chiedere niente: lo
    /// stato iniziale. Gli eventi già nella vista non si rimostrano.
    pub(super) fn copy_catalog(&mut self) {
        let view = Arc::clone(&self.core_snapshot.catalog);
        self.catalog_event = view.events.back().map_or(0, |raised| raised.seq);
        self.home.recent = view.home.recent.clone();
        self.home.alphabetical = view.home.alphabetical.clone();
        self.home.albums = view.home.albums.clone();
        let listing = &view.library;
        self.library.tracks = listing.tracks.clone();
        self.library.next_offset = listing.next_offset;
        self.library.loading_more = listing.loading_more;
        self.library.more_error.clone_from(&listing.more_error);
        self.library.sort = listing.sort;
        self.library.albums = listing.albums.clone();
        self.library.albums_truncated = listing.albums_truncated;
        let search = &view.search;
        self.search.submitted.clone_from(&search.submitted);
        self.search.results = search.results.clone();
        self.search.discover = search.discover.clone();
        self.search.libraries = Arc::clone(&search.libraries);
        self.search.open_library.clone_from(&search.open_library);
        self.search.sources = search.sources.clone();
        self.search.playlists = search.playlists.clone();
        self.search.artists = Arc::clone(&search.artists);
        self.album = view.album.clone();
        self.artist = view.artist.clone();
    }

    /// Il catalogo appena pubblicato dal core, al posto di `previous`: ciò che è cambiato si copia, le
    /// copertine nuove si chiedono, e si fa ciò che gli eventi dicono (aprire l'artista, un avviso).
    pub(super) fn catalog_published(&mut self, previous: &CatalogView) -> Task<Message> {
        let view = Arc::clone(&self.core_snapshot.catalog);
        let mut urls: Vec<String> = Vec::new();
        let mut tasks = Vec::new();

        // Home.
        if !same(&previous.home.recent, &view.home.recent) {
            self.home.recent = view.home.recent.clone();
            if let Load::Ready(tracks) = &view.home.recent {
                urls.extend(track_covers(Some(tracks)));
                // Il feed è già nella stessa istantanea (`library_published` gira prima): il carosello
                // riparte dalla prima uscita.
                self.hero_go(0);
            } else {
                self.home.hero = Hero::default();
            }
        }
        if !same(&previous.home.alphabetical, &view.home.alphabetical) {
            self.home.alphabetical = view.home.alphabetical.clone();
            if let Load::Ready(tracks) = &view.home.alphabetical {
                urls.extend(track_covers(Some(tracks)));
            }
        }
        if !same(&previous.home.albums, &view.home.albums) {
            self.home.albums = view.home.albums.clone();
            urls.extend(album_covers(view.home.albums.ready().map(|a| &a[..])));
        }

        // Libreria.
        let (before, listing) = (&previous.library, &view.library);
        if !same_list(&before.tracks, &listing.tracks) {
            self.library.tracks = listing.tracks.clone();
            urls.extend(track_covers(listing.tracks.ready().map(|t| &t[..])));
        }
        if !same(&before.albums, &listing.albums) {
            self.library.albums = listing.albums.clone();
            urls.extend(album_covers(listing.albums.ready().map(|a| &a[..])));
        }
        self.library.next_offset = listing.next_offset;
        self.library.loading_more = listing.loading_more;
        self.library.more_error.clone_from(&listing.more_error);
        self.library.sort = listing.sort;
        self.library.albums_truncated = listing.albums_truncated;

        // Ricerca.
        let (before, search) = (&previous.search, &view.search);
        if before.submitted != search.submitted {
            self.search.submitted.clone_from(&search.submitted);
        }
        if !same(&before.results, &search.results) {
            self.search.results = search.results.clone();
            if let Load::Ready(results) = &search.results {
                urls.extend(track_covers(Some(&results.tracks)));
                urls.extend(album_covers(Some(&results.albums)));
                urls.extend(results.artists.iter().filter_map(|a| a.cover_url.clone()));
            }
        }
        if !same(&before.discover, &search.discover) || !Arc::ptr_eq(&before.libraries, &search.libraries) {
            self.search.discover = search.discover.clone();
            self.search.libraries = Arc::clone(&search.libraries);
            urls.extend(search.libraries.iter().filter_map(|l| l.cover_url.clone()));
            if let Load::Ready(items) = &search.discover {
                urls.extend(items.iter().filter_map(|i| i.cover_url.clone()));
            }
        }
        if !same(&before.sources, &search.sources)
            || !same(&before.playlists, &search.playlists)
            || !Arc::ptr_eq(&before.artists, &search.artists)
        {
            self.search.sources = search.sources.clone();
            self.search.playlists = search.playlists.clone();
            self.search.artists = Arc::clone(&search.artists);
            urls.extend(self.search_covers());
        }
        if before.open_library.as_ref().map(|o| (&o.items, o.next_page, o.loading_more))
            != search.open_library.as_ref().map(|o| (&o.items, o.next_page, o.loading_more))
            || before.open_library.as_ref().map(|o| &o.library)
                != search.open_library.as_ref().map(|o| &o.library)
        {
            self.search.open_library.clone_from(&search.open_library);
            if let Some(Load::Ready(items)) = search.open_library.as_ref().map(|o| &o.items) {
                urls.extend(items.iter().filter_map(|i| i.cover_url.clone()));
            }
        }

        // Album e artista aperti.
        if !same_open(&previous.album, &view.album) {
            self.album = view.album.clone();
            if let Some((_, Load::Ready(album))) = &view.album {
                urls.extend(track_covers(Some(&album.tracks)));
                urls.extend(album.cover_url.clone());
            }
        }
        if !same_open(&previous.artist, &view.artist) {
            self.artist = view.artist.clone();
            // Le informazioni dal catalogo ufficiale («Altro di …») le chiede il core da sé.
            if let Some((_, Load::Ready(page))) = &view.artist {
                urls.extend(page_covers(page, usize::MAX));
            }
        }

        // Ciò che il core ha voluto far sapere, in ordine.
        let shown = self.catalog_event;
        for raised in view.events.iter().filter(|raised| raised.seq > shown) {
            self.catalog_event = raised.seq;
            match &raised.event {
                CatalogEvent::MetaArtistOpened(id) => tasks.push(self.open(Screen::Artist(*id))),
                CatalogEvent::MetaArtistFailed(err) | CatalogEvent::LibraryPageFailed(err) => {
                    self.notice = Some(Notice::Error(err.to_string()));
                }
                // Un brano ritrovato o riletto: la sua copertina può essere nuova.
                CatalogEvent::Replaced(track) => urls.extend(track.cover_url.clone()),
                CatalogEvent::Prefetched(page) => urls.extend(page_covers(page, WARM_COVERS)),
            }
        }

        tasks.push(self.fetch_covers(urls));
        Task::batch(tasks)
    }
}

/// La pagina aperta (album o artista) è la stessa: stesso identificatore e stesso dato, per puntatore.
/// Un aggiornamento in silenzio cambia il dato e lascia l'identificatore: dev'essere un cambio.
fn same_open<T>(a: &Option<(uuid::Uuid, Load<Arc<T>>)>, b: &Option<(uuid::Uuid, Load<Arc<T>>)>) -> bool {
    match (a, b) {
        (Some((a_id, a_load)), Some((b_id, b_load))) => a_id == b_id && same(a_load, b_load),
        (None, None) => true,
        _ => false,
    }
}

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_catalog(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Retry => self.reload(),
            Message::SearchInput(query) => {
                // Iced scrive il carattere anche con Ctrl premuto: Ctrl+, o Ctrl+1 nella ricerca
                // aggiungerebbero «,» o «1» invece di aprire la schermata.
                if self.modifiers.command()
                    && inserted_char(&self.search.query, &query)
                        .is_some_and(|c| c == ',' || c.is_ascii_digit())
                {
                    return Task::none();
                }
                let clear = clean_query(&query).is_none();
                self.search.query.clone_from(&query);
                // Il core aspetta 300 ms dopo l'ultimo tasto e lancia solo l'ultima ricerca.
                let search = self.send_catalog(CatalogCmd::SearchInput(query));
                if clear || self.screen == Screen::Search {
                    return search;
                }
                Task::batch([self.open(Screen::Search), search])
            }
            Message::ArchiveLibraryOpen(library) => {
                self.send_catalog(CatalogCmd::ArchiveLibraryOpen(library))
            }
            Message::ArchiveLibraryMore => self.send_catalog(CatalogCmd::ArchiveLibraryMore),
            Message::ArchiveLibraryClose => self.send_catalog(CatalogCmd::ArchiveLibraryClose),
            Message::OpenMetaArtist(id) => self.send_catalog(CatalogCmd::OpenMetaArtist(id)),
            Message::CoverLoaded(url, pixels) => {
                // La copertina del brano in corso arriva dopo il brano: il sistema la riceve ora.
                let current =
                    self.queue.current().is_some_and(|t| t.cover_url.as_deref() == Some(url.as_str()));
                let artwork = pixels.as_ref().filter(|_| current).map(|pixels| Artwork {
                    url: url.clone(),
                    side: pixels.side,
                    rgba: pixels.rgba.clone(),
                });
                self.covers.loaded(url, pixels);
                match artwork {
                    Some(artwork) => self.send_playback(PlaybackCmd::Artwork(Box::new(artwork))),
                    None => Task::none(),
                }
            }
            Message::ArtistFilter(filter) => {
                self.artists.filter = filter;
                Task::none()
            }
            Message::ArtistMorePopular => {
                self.artists.more_popular = !self.artists.more_popular;
                Task::none()
            }
            Message::LibraryMore => self.send_catalog(CatalogCmd::LibraryMore),
            Message::LibraryFilter(filter) => {
                self.library.filter = filter;
                Task::none()
            }
            Message::LibrarySort(sort) => self.send_catalog(CatalogCmd::LibrarySort(sort)),
            Message::OpenLibrary(filter) => {
                self.library.filter = filter;
                let open = self.open(Screen::Library);
                Task::batch([open, self.persist()])
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}
