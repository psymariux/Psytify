//! Libreria e gusti (passo 4.2): playlist locali, brani piaciuti, album salvati, artisti seguiti,
//! ascolti e tempo ascoltato, «Novità» già viste, cronologia delle ricerche e feed della Home.
//!
//! Il dominio possiede questi campi di `config.json` e li consegna alle impostazioni
//! ([`Settings::store_library`](super::settings::Settings::store_library)), che scrivono il file:
//! un comando che non cambia niente non pubblica e non fa scrivere. Le liste sono [`Shared`]: la
//! vista, la configurazione e le copie dei frontend sono gli stessi `Arc`, e una lista si copia
//! solo quando cambia mentre un frontend tiene ancora la precedente.
//!
//! Il feed della Home si calcola qui, una volta per cambiamento (arrivo dei brani recenti, un
//! artista seguito, e a ogni rientro sulla Home se gli ascolti sono cambiati), mai nella `view`. Il
//! conteggio degli ascolti resta un comando: chi sente il brano (il dominio della riproduzione, dal
//! passo 4.3) decide quando mandarlo.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use crate::api::{SourceItem, SourceKind, Track};
use crate::config::taste::{self, FollowedArtist, PlayStat, SavedAlbum};
use crate::config::{self, LibraryFields, MAX_PLAYLISTS, Playlist};
use crate::effect::Effect;
use crate::feed::Feed;
use crate::shared::Shared;

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum LibraryCmd {
    /// Mette o toglie «mi piace»: il brano appena piaciuto va in testa.
    ToggleLike(Track),
    /// Una playlist nuova, con l'identificatore scelto da chi la chiede (il frontend la apre appena
    /// compare nella vista). `pending` sono i brani di una playlist esterna ancora da importare.
    /// Oltre [`MAX_PLAYLISTS`] non si crea.
    CreatePlaylist {
        id: Uuid,
        name: String,
        tracks: Vec<Track>,
        pending: Vec<SourceItem>,
    },
    RenamePlaylist {
        id: Uuid,
        name: String,
    },
    DeletePlaylist(Uuid),
    /// In fondo alla playlist; se c'è già, non cambia niente.
    AddToPlaylist {
        id: Uuid,
        track: Track,
    },
    RemoveFromPlaylist {
        id: Uuid,
        index: usize,
    },
    /// Visibile sul profilo, o di nuovo privata.
    SetPlaylistPublic {
        id: Uuid,
        public: bool,
    },
    /// Un brano della playlist esterna è stato importato (`track`) o non è disponibile (`None`):
    /// esce dai brani ancora da importare.
    ImportedToPlaylist {
        id: Uuid,
        source: SourceKind,
        item: String,
        track: Option<Track>,
    },
    /// Segue o smette di seguire. La prima volta che si segue qualcuno, le uscite già in catalogo
    /// non sono «novità».
    ToggleFollow(FollowedArtist),
    ToggleSaveAlbum(SavedAlbum),
    /// L'immagine di un artista seguito. Con `only_if_missing` solo se non ne ha una.
    SetFollowedCover {
        id: Uuid,
        cover_url: Option<String>,
        only_if_missing: bool,
    },
    SetSavedAlbumCover {
        id: Uuid,
        cover_url: Option<String>,
    },
    /// Un brano contato come ascoltato, a `at` secondi dal 1970.
    CountPlay {
        track: Track,
        at: u64,
    },
    /// Il brano è partito: va in testa agli ascoltati di recente.
    RememberListen(Track),
    /// Altri millisecondi ascoltati: si sommano al totale. Li manda il dominio della riproduzione, che
    /// li conta con la posizione vera dell'audio e li consegna a gruppi (mai a ogni battito).
    Listen(u64),
    RememberSearch(String),
    /// Toglie una ricerca dalla cronologia, o tutte con `None`.
    ForgetSearch(Option<String>),
    /// Ciò che c'è in «Novità» si considera visto.
    MarkNewsSeen,
    /// `new` al posto di ogni copia salvata del brano `old` (mi piace, recenti, playlist, ascolti).
    ReplaceTrack {
        old: Uuid,
        new: Track,
    },
    /// Le copertine che il catalogo ha appena rimandato, per brano.
    PatchCovers(Arc<HashMap<Uuid, Option<String>>>),
    /// I brani recenti del catalogo, da cui nasce il feed della Home; `None` quando il catalogo
    /// cambia e la Home si ricarica.
    HomeTracks(Option<Arc<[Track]>>),
    /// Rientrando sulla Home: se gli ascolti o gli artisti sono cambiati dall'ultima volta, il feed
    /// si rifà (i mix restano gli stessi: li tiene fermi il seme della sessione).
    RefreshFeed,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct LibraryView {
    pub playlists: Shared<Playlist>,
    /// Il più recente in testa.
    pub liked: Shared<Track>,
    /// Ultimi brani ascoltati, il più recente in testa.
    pub recent: Shared<Track>,
    pub followed: Shared<FollowedArtist>,
    pub saved_albums: Shared<SavedAlbum>,
    /// Ascolti per brano, i più ascoltati prima.
    pub stats: Shared<PlayStat>,
    /// Tempo ascoltato in tutto, in millisecondi, all'ultima consegna di [`LibraryCmd::Listen`].
    pub listened_ms: u64,
    /// L'uscita più recente vista in «Novità» (`created_at` del brano).
    pub news_seen: String,
    /// Uscite degli artisti seguiti arrivate dopo `news_seen`.
    pub unread_news: usize,
    pub search_history: Shared<String>,
    /// Il feed della Home, già calcolato: la vista non fa conti.
    pub feed: Arc<Feed>,
    /// I brani recenti da cui nasce `feed`: un frontend che li ha mandati sa così che il feed è
    /// il loro (`Arc::ptr_eq`).
    pub feed_source: Option<Arc<[Track]>>,
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {}

/// I lavori asincroni del dominio (rete, file), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug, Default)]
pub(crate) struct Library {
    pub(crate) view: Arc<LibraryView>,
    /// Il seme dei mix: lo stesso per tutta la sessione.
    seed: u64,
    /// Un campo di `config.json` è cambiato e le impostazioni non l'hanno ancora (`take_persist`).
    persist: bool,
    /// Gli ascolti o gli artisti sono cambiati dopo l'ultima costruzione del feed.
    feed_stale: bool,
}

impl Library {
    /// Il dominio con i campi letti all'avvio. Una ricerca senza lettere né cifre (un «,» scappato
    /// con Ctrl+,) non torna, e il file si corregge alla prossima scrittura.
    pub(crate) fn new(fields: LibraryFields, seed: u64) -> Self {
        let mut search_history = fields.search_history;
        let worth = |query: &String| config::worth_remembering(query);
        let persist = !search_history.iter().all(worth);
        if persist {
            search_history = search_history.iter().filter(|query| worth(query)).cloned().collect();
        }
        let view = LibraryView {
            playlists: fields.playlists,
            liked: fields.liked,
            recent: fields.recent,
            followed: fields.followed,
            saved_albums: fields.saved_albums,
            stats: fields.stats,
            listened_ms: fields.listened_ms,
            news_seen: fields.news_seen,
            search_history,
            ..LibraryView::default()
        };
        Self { view: Arc::new(view), seed, persist, feed_stale: false }
    }

    /// I campi di `config.json` di questo dominio come stanno adesso.
    pub(crate) fn fields(&self) -> LibraryFields {
        LibraryFields {
            playlists: self.view.playlists.clone(),
            liked: self.view.liked.clone(),
            recent: self.view.recent.clone(),
            followed: self.view.followed.clone(),
            saved_albums: self.view.saved_albums.clone(),
            stats: self.view.stats.clone(),
            listened_ms: self.view.listened_ms,
            news_seen: self.view.news_seen.clone(),
            search_history: self.view.search_history.clone(),
        }
    }

    /// Dice se c'è qualcosa da consegnare alle impostazioni, e lo azzera.
    pub(crate) fn take_persist(&mut self) -> bool {
        std::mem::take(&mut self.persist)
    }

    pub(crate) fn apply(&mut self, command: LibraryCmd, _now: Instant, _effects: &mut Vec<Effect>) {
        match command {
            LibraryCmd::ToggleLike(track) => {
                config::toggle_like(self.edit().liked.make_mut(), track);
            }
            LibraryCmd::CreatePlaylist { id, name, tracks, pending } => {
                let full = self.view.playlists.len() >= MAX_PLAYLISTS;
                if full || self.view.playlists.iter().any(|p| p.id == id) {
                    return;
                }
                let name = config::clean_name(&name);
                let playlist = Playlist { id, name, tracks, pending, public: false };
                self.edit().playlists.make_mut().push(playlist);
            }
            LibraryCmd::RenamePlaylist { id, name } => {
                let name = config::clean_name(&name);
                if self.view.playlists.iter().any(|p| p.id == id && p.name != name)
                    && let Some(playlist) = self.playlist_mut(id)
                {
                    playlist.name = name;
                }
            }
            LibraryCmd::DeletePlaylist(id) => {
                if self.view.playlists.iter().any(|p| p.id == id) {
                    self.edit().playlists.make_mut().retain(|p| p.id != id);
                }
            }
            LibraryCmd::AddToPlaylist { id, track } => {
                if self.view.playlists.iter().any(|p| p.id == id && !p.contains(track.id))
                    && let Some(playlist) = self.playlist_mut(id)
                {
                    playlist.add(track);
                }
            }
            LibraryCmd::RemoveFromPlaylist { id, index } => {
                if self.view.playlists.iter().any(|p| p.id == id && index < p.tracks.len())
                    && let Some(playlist) = self.playlist_mut(id)
                {
                    playlist.remove_at(index);
                }
            }
            LibraryCmd::SetPlaylistPublic { id, public } => {
                if self.view.playlists.iter().any(|p| p.id == id && p.public != public)
                    && let Some(playlist) = self.playlist_mut(id)
                {
                    playlist.public = public;
                }
            }
            LibraryCmd::ImportedToPlaylist { id, source, item, track } => {
                let waiting = |p: &Playlist| p.pending.iter().any(|i| i.source == source && i.id == item);
                if self.view.playlists.iter().any(|p| p.id == id && waiting(p))
                    && let Some(playlist) = self.playlist_mut(id)
                {
                    playlist.pending.retain(|i| !(i.source == source && i.id == item));
                    if let Some(track) = track {
                        playlist.add(track);
                    }
                }
            }
            LibraryCmd::ToggleFollow(artist) => self.toggle_follow(artist),
            LibraryCmd::ToggleSaveAlbum(album) => {
                taste::toggle_saved_album(self.edit().saved_albums.make_mut(), album);
            }
            LibraryCmd::SetFollowedCover { id, cover_url, only_if_missing } => {
                let applies = |f: &FollowedArtist| {
                    f.id == id && f.cover_url != cover_url && (!only_if_missing || f.cover_url.is_none())
                };
                if self.view.followed.iter().any(applies) {
                    let view = self.edit();
                    for artist in view.followed.make_mut().iter_mut().filter(|f| applies(f)) {
                        artist.cover_url.clone_from(&cover_url);
                    }
                    self.feed_stale = true;
                }
            }
            LibraryCmd::SetSavedAlbumCover { id, cover_url } => {
                if self.view.saved_albums.iter().any(|a| a.id == id && a.cover_url != cover_url) {
                    let view = self.edit();
                    for album in view.saved_albums.make_mut().iter_mut().filter(|a| a.id == id) {
                        album.cover_url.clone_from(&cover_url);
                    }
                }
            }
            LibraryCmd::CountPlay { track, at } => {
                taste::count_play(self.edit().stats.make_mut(), &track, at);
                self.feed_stale = true;
            }
            LibraryCmd::RememberListen(track) => {
                // Lo stesso brano già in testa: la lista resterebbe uguale.
                if self.view.recent.first() != Some(&track) {
                    config::remember_listen(self.edit().recent.make_mut(), &track);
                }
            }
            LibraryCmd::Listen(ms) => {
                if ms > 0 {
                    let edit = self.edit();
                    edit.listened_ms = edit.listened_ms.saturating_add(ms);
                }
            }
            LibraryCmd::RememberSearch(query) => {
                let mut next = Vec::clone(&self.view.search_history);
                config::remember_search(&mut next, &query);
                if next != *self.view.search_history {
                    self.edit().search_history = next.into();
                }
            }
            LibraryCmd::ForgetSearch(Some(query)) => {
                if self.view.search_history.contains(&query) {
                    self.edit().search_history.make_mut().retain(|old| *old != query);
                }
            }
            LibraryCmd::ForgetSearch(None) => {
                if !self.view.search_history.is_empty() {
                    self.edit().search_history = Shared::default();
                }
            }
            LibraryCmd::MarkNewsSeen => {
                let newest = self.view.feed.news.iter().map(|r| r.created_at.as_str()).max();
                if let Some(newest) = newest
                    && newest > self.view.news_seen.as_str()
                {
                    let newest = newest.to_owned();
                    let view = self.edit();
                    view.news_seen = newest;
                    refresh_unread(view);
                }
            }
            LibraryCmd::ReplaceTrack { old, new } => self.replace_track(old, &new),
            LibraryCmd::PatchCovers(fresh) => self.patch_covers(&fresh),
            LibraryCmd::HomeTracks(Some(tracks)) => self.rebuild_feed(Some(tracks)),
            LibraryCmd::HomeTracks(None) => {
                if self.view.feed_source.is_some() {
                    let view = Arc::make_mut(&mut self.view);
                    view.feed = Arc::default();
                    view.feed_source = None;
                    refresh_unread(view);
                }
                self.feed_stale = false;
            }
            LibraryCmd::RefreshFeed => {
                if std::mem::take(&mut self.feed_stale) {
                    self.rebuild_feed(None);
                }
            }
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, _effects: &mut Vec<Effect>) {
        match timer {}
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, _effects: &mut Vec<Effect>) {
        match done {}
    }

    /// La vista da cambiare: copiata se un frontend tiene ancora la precedente. Si chiama solo
    /// quando qualcosa cambia davvero, e segna che le impostazioni devono scrivere il file.
    fn edit(&mut self) -> &mut LibraryView {
        self.persist = true;
        Arc::make_mut(&mut self.view)
    }

    /// La playlist da cambiare, dopo che chi chiama ha controllato che cambia.
    fn playlist_mut(&mut self, id: Uuid) -> Option<&mut Playlist> {
        self.edit().playlists.make_mut().iter_mut().find(|p| p.id == id)
    }

    fn toggle_follow(&mut self, artist: FollowedArtist) {
        // La prima volta che si segue qualcuno, le uscite già in catalogo non sono «novità».
        let first_seen = match self.view.feed.releases.first() {
            Some(newest) if self.view.news_seen.is_empty() => Some(newest.created_at.clone()),
            _ => None,
        };
        let view = self.edit();
        if let Some(created_at) = first_seen {
            view.news_seen = created_at;
        }
        taste::toggle_follow(view.followed.make_mut(), artist);
        refresh_unread(view);
        self.rebuild_feed(None);
    }

    /// Rifà il feed dai brani recenti (`tracks`, o quelli del feed attuale se `None`). Senza brani
    /// recenti non c'è feed. Se il risultato è quello che c'è già, non pubblica.
    fn rebuild_feed(&mut self, tracks: Option<Arc<[Track]>>) {
        self.feed_stale = false;
        let Some(source) = tracks.or_else(|| self.view.feed_source.clone()) else { return };
        let feed = Feed::build(&source, &self.view.followed, &self.view.stats, self.seed);
        let same_source = self.view.feed_source.as_ref().is_some_and(|known| Arc::ptr_eq(known, &source));
        if same_source && *self.view.feed == feed {
            return;
        }
        // Il feed non si scrive su disco: niente `edit`, quindi niente `persist`.
        let view = Arc::make_mut(&mut self.view);
        view.feed = Arc::new(feed);
        view.feed_source = Some(source);
        refresh_unread(view);
    }

    fn replace_track(&mut self, old: Uuid, new: &Track) {
        let stale = |t: &Track| t.id == old && t != new;
        let view = &self.view;
        let liked = view.liked.iter().any(stale);
        let recent = view.recent.iter().any(stale);
        let playlists = view.playlists.iter().any(|p| p.tracks.iter().any(stale));
        let stats = view.stats.iter().any(|s| stale(&s.track));
        if !(liked || recent || playlists || stats) {
            return;
        }
        let swap = |t: &mut Track| {
            if stale(t) {
                t.clone_from(new);
            }
        };
        let view = self.edit();
        if liked {
            view.liked.make_mut().iter_mut().for_each(swap);
        }
        if recent {
            view.recent.make_mut().iter_mut().for_each(swap);
        }
        if playlists {
            for playlist in view.playlists.make_mut() {
                playlist.tracks.iter_mut().for_each(swap);
            }
        }
        if stats {
            view.stats.make_mut().iter_mut().for_each(|s| swap(&mut s.track));
            self.feed_stale = true;
        }
    }

    fn patch_covers(&mut self, fresh: &HashMap<Uuid, Option<String>>) {
        let differs = |t: &Track| fresh.get(&t.id).is_some_and(|cover| t.cover_url != *cover);
        let view = &self.view;
        let recent = view.recent.iter().any(differs);
        let liked = view.liked.iter().any(differs);
        let playlists = view.playlists.iter().any(|p| p.tracks.iter().any(differs));
        let stats = view.stats.iter().any(|s| differs(&s.track));
        if !(recent || liked || playlists || stats) {
            return;
        }
        let view = self.edit();
        if recent {
            taste::patch_covers(view.recent.make_mut(), fresh);
        }
        if liked {
            taste::patch_covers(view.liked.make_mut(), fresh);
        }
        if playlists {
            for playlist in view.playlists.make_mut() {
                taste::patch_covers(&mut playlist.tracks, fresh);
            }
        }
        if stats {
            for stat in view.stats.make_mut() {
                taste::patch_covers(std::slice::from_mut(&mut stat.track), fresh);
            }
            self.feed_stale = true;
        }
    }
}

/// Aggiorna il conteggio delle uscite non viste, se è cambiato.
fn refresh_unread(view: &mut LibraryView) {
    let seen = view.news_seen.as_str();
    let unread = view.feed.news.iter().filter(|release| release.created_at.as_str() > seen).count();
    if view.unread_news != unread {
        view.unread_news = unread;
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {}
    }

    pub(crate) fn lost(&self) -> Done {
        match *self {}
    }
}

/// I passi 0 di questo dominio (`mixes_stay_fixed_for_the_session`, gli ascolti e i brani di
/// recente di `playback.rs`, la cronologia delle ricerche), portati sul riduttore: i comandi si
/// danno a mano e si guarda la vista.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::ArtistRef;
    use crate::api::models::fixtures::track;

    fn library() -> Library {
        Library::new(LibraryFields::default(), 7)
    }

    fn send(library: &mut Library, command: LibraryCmd) {
        library.apply(command, Instant::now(), &mut Vec::new());
    }

    /// Una vista che non ha cambiato puntatore non viene pubblicata.
    fn changes(library: &mut Library, command: LibraryCmd) -> bool {
        let before = Arc::clone(&library.view);
        send(library, command);
        !Arc::ptr_eq(&before, &library.view)
    }

    fn ids(tracks: &[Track]) -> Vec<Uuid> {
        tracks.iter().map(|t| t.id).collect()
    }

    fn page(count: u128) -> Arc<[Track]> {
        (1..=count).map(track).collect()
    }

    fn artist(n: u128) -> FollowedArtist {
        FollowedArtist { id: Uuid::from_u128(n), name: format!("Artista {n}"), cover_url: None }
    }

    #[test]
    fn a_like_goes_first_and_the_same_track_again_removes_it() {
        let mut library = library();
        send(&mut library, LibraryCmd::ToggleLike(track(1)));
        send(&mut library, LibraryCmd::ToggleLike(track(2)));
        assert_eq!(ids(&library.view.liked), [track(2).id, track(1).id]);
        send(&mut library, LibraryCmd::ToggleLike(track(2)));
        assert_eq!(ids(&library.view.liked), [track(1).id]);
        assert!(library.take_persist(), "the file has to follow");
        assert!(!library.take_persist(), "handed over once");
    }

    #[test]
    fn a_playlist_is_created_renamed_filled_and_deleted() {
        let mut library = library();
        let id = Uuid::from_u128(900);
        send(
            &mut library,
            LibraryCmd::CreatePlaylist {
                id,
                name: "  La   mia\u{7} sera ".into(),
                tracks: vec![track(1)],
                pending: Vec::new(),
            },
        );
        assert_eq!(library.view.playlists[0].name, "La mia sera", "the name is cleaned");

        assert!(!changes(&mut library, LibraryCmd::RenamePlaylist { id, name: "La mia sera".into() }));
        assert!(changes(&mut library, LibraryCmd::RenamePlaylist { id, name: "Mattina".into() }));
        assert!(changes(&mut library, LibraryCmd::AddToPlaylist { id, track: track(2) }));
        assert!(
            !changes(&mut library, LibraryCmd::AddToPlaylist { id, track: track(2) }),
            "a playlist never repeats a track"
        );
        assert!(!changes(&mut library, LibraryCmd::RemoveFromPlaylist { id, index: 5 }));
        assert!(changes(&mut library, LibraryCmd::RemoveFromPlaylist { id, index: 0 }));
        assert_eq!(ids(&library.view.playlists[0].tracks), [track(2).id]);
        assert!(changes(&mut library, LibraryCmd::SetPlaylistPublic { id, public: true }));
        assert!(!changes(&mut library, LibraryCmd::SetPlaylistPublic { id, public: true }));

        assert!(!changes(&mut library, LibraryCmd::DeletePlaylist(Uuid::from_u128(1))));
        assert!(changes(&mut library, LibraryCmd::DeletePlaylist(id)));
        assert!(library.view.playlists.is_empty());
    }

    #[test]
    fn playlists_are_capped_and_ids_never_repeat() {
        let mut library = library();
        let create = |n: u128| LibraryCmd::CreatePlaylist {
            id: Uuid::from_u128(n),
            name: "P".into(),
            tracks: Vec::new(),
            pending: Vec::new(),
        };
        assert!(changes(&mut library, create(1)));
        assert!(!changes(&mut library, create(1)), "the same id twice");
        for n in 2..=MAX_PLAYLISTS as u128 + 5 {
            send(&mut library, create(n));
        }
        assert_eq!(library.view.playlists.len(), MAX_PLAYLISTS);
    }

    /// Una playlist salvata da una fonte esterna: i brani escono da `pending` uno alla volta, anche
    /// quando non sono disponibili.
    #[test]
    fn imports_leave_the_pending_list_one_at_a_time() {
        let item = |id: &str| SourceItem {
            source: SourceKind::Deezer,
            id: id.into(),
            title: format!("Brano {id}"),
            artist: "Artista".into(),
            duration_ms: Some(180_000),
            cover_url: None,
        };
        let mut library = library();
        let id = Uuid::from_u128(5);
        send(
            &mut library,
            LibraryCmd::CreatePlaylist {
                id,
                name: "Sera".into(),
                tracks: Vec::new(),
                pending: vec![item("a"), item("b")],
            },
        );
        let imported = |item: &str, track| LibraryCmd::ImportedToPlaylist {
            id,
            source: SourceKind::Deezer,
            item: item.into(),
            track,
        };
        assert!(changes(&mut library, imported("a", Some(track(1)))));
        assert!(!changes(&mut library, imported("a", Some(track(1)))), "already imported");
        assert!(changes(&mut library, imported("b", None)));
        let playlist = &library.view.playlists[0];
        assert_eq!((playlist.tracks.len(), playlist.pending.len()), (1, 0));
    }

    #[test]
    fn plays_are_counted_and_the_most_played_comes_first() {
        let mut library = library();
        for (n, at) in [(1, 100), (2, 200), (1, 300)] {
            send(&mut library, LibraryCmd::CountPlay { track: track(n), at });
        }
        let stats = &library.view.stats;
        assert_eq!((stats[0].track.id, stats[0].plays, stats[0].last_played), (track(1).id, 2, 300));
        assert_eq!(taste::total_plays(stats), 3);
    }

    #[test]
    fn a_remembered_listen_goes_first_without_repeats_and_the_same_one_changes_nothing() {
        let mut library = library();
        assert!(changes(&mut library, LibraryCmd::RememberListen(track(1))));
        assert!(changes(&mut library, LibraryCmd::RememberListen(track(2))));
        assert!(!changes(&mut library, LibraryCmd::RememberListen(track(2))), "already first");
        assert!(changes(&mut library, LibraryCmd::RememberListen(track(1))));
        assert_eq!(ids(&library.view.recent), [track(1).id, track(2).id]);
    }

    #[test]
    fn listened_time_adds_up_and_zero_changes_nothing() {
        let mut library = library();
        assert!(changes(&mut library, LibraryCmd::Listen(750)));
        assert!(changes(&mut library, LibraryCmd::Listen(250)));
        assert!(!changes(&mut library, LibraryCmd::Listen(0)));
        assert_eq!(library.view.listened_ms, 1_000);
    }

    #[test]
    fn typing_a_search_replaces_the_shorter_one_and_forgetting_removes_it() {
        let mut library = library();
        for query in ["emi", "emin", "eminem", "  ,  ", "low"] {
            send(&mut library, LibraryCmd::RememberSearch(query.into()));
        }
        assert_eq!(*library.view.search_history, ["low", "eminem"], "no letter or digit, no memory");
        assert!(!changes(&mut library, LibraryCmd::RememberSearch("low".into())), "already first");
        assert!(!changes(&mut library, LibraryCmd::ForgetSearch(Some("altro".into()))));
        assert!(changes(&mut library, LibraryCmd::ForgetSearch(Some("low".into()))));
        assert_eq!(*library.view.search_history, ["eminem"]);
        assert!(changes(&mut library, LibraryCmd::ForgetSearch(None)));
        assert!(!changes(&mut library, LibraryCmd::ForgetSearch(None)));
    }

    #[test]
    fn a_saved_history_without_letters_is_cleaned_on_start() {
        let fields = LibraryFields {
            search_history: vec!["eminem".to_owned(), ",".to_owned()].into(),
            ..LibraryFields::default()
        };
        let mut library = Library::new(fields, 1);
        assert_eq!(*library.view.search_history, ["eminem"]);
        assert!(library.take_persist(), "the file is corrected with the next write");
    }

    #[test]
    fn saving_an_album_twice_removes_it_and_the_cover_follows_the_catalogue() {
        let mut library = library();
        let album = SavedAlbum {
            id: Uuid::from_u128(7),
            title: "Banchi".into(),
            artist: ArtistRef { id: Uuid::from_u128(1), name: "Marea".into() },
            cover_url: None,
        };
        send(&mut library, LibraryCmd::ToggleSaveAlbum(album.clone()));
        let cover = |url: Option<&str>| LibraryCmd::SetSavedAlbumCover {
            id: album.id,
            cover_url: url.map(str::to_owned),
        };
        assert!(changes(&mut library, cover(Some("https://archive.org/img/7"))));
        assert!(!changes(&mut library, cover(Some("https://archive.org/img/7"))));
        send(&mut library, LibraryCmd::ToggleSaveAlbum(album));
        assert!(library.view.saved_albums.is_empty());
    }

    #[test]
    fn a_followed_cover_is_set_or_only_filled_in() {
        let mut library = library();
        send(&mut library, LibraryCmd::ToggleFollow(artist(1)));
        let id = Uuid::from_u128(1);
        let set = |url: &str, only_if_missing| LibraryCmd::SetFollowedCover {
            id,
            cover_url: Some(url.into()),
            only_if_missing,
        };
        assert!(changes(&mut library, set("uno", true)));
        assert!(!changes(&mut library, set("due", true)), "it already has one");
        assert!(changes(&mut library, set("due", false)));
        assert_eq!(library.view.followed[0].cover_url.as_deref(), Some("due"));
    }

    #[test]
    fn covers_and_healed_tracks_reach_every_saved_copy() {
        let mut library = library();
        let mut old = track(1);
        old.cover_url = Some("onda".into());
        send(&mut library, LibraryCmd::ToggleLike(old.clone()));
        send(&mut library, LibraryCmd::RememberListen(old.clone()));
        send(&mut library, LibraryCmd::CountPlay { track: old.clone(), at: 1 });
        send(
            &mut library,
            LibraryCmd::CreatePlaylist {
                id: Uuid::from_u128(3),
                name: "P".into(),
                tracks: vec![old.clone()],
                pending: Vec::new(),
            },
        );
        let fresh = Arc::new(HashMap::from([(old.id, None), (Uuid::from_u128(99), Some("x".to_owned()))]));
        assert!(changes(&mut library, LibraryCmd::PatchCovers(Arc::clone(&fresh))));
        let view = &library.view;
        for cover in [
            &view.liked[0].cover_url,
            &view.recent[0].cover_url,
            &view.stats[0].track.cover_url,
            &view.playlists[0].tracks[0].cover_url,
        ] {
            assert_eq!(*cover, None, "the wave of Archive is no longer a cover");
        }
        assert!(!changes(&mut library, LibraryCmd::PatchCovers(fresh)), "the second time nothing changes");

        let healed = track(50);
        assert!(changes(&mut library, LibraryCmd::ReplaceTrack { old: old.id, new: healed.clone() }));
        let view = &library.view;
        assert_eq!(view.liked[0].id, healed.id);
        assert_eq!(view.playlists[0].tracks[0].id, healed.id);
        assert_eq!(view.stats[0].track.id, healed.id);
        assert!(!changes(&mut library, LibraryCmd::ReplaceTrack { old: old.id, new: healed }));
    }

    // ---------- Feed della Home ----------

    fn played(library: &mut Library, plays: &[(u128, u32)]) {
        for &(n, times) in plays {
            for _ in 0..times {
                send(library, LibraryCmd::CountPlay { track: track(n), at: 100 });
            }
        }
    }

    /// `mixes_stay_fixed_for_the_session`: i mix nascono dal seme della sessione e il feed si rifà
    /// solo quando serve, senza cambiarli.
    #[test]
    fn mixes_stay_fixed_for_the_session() {
        let mut library = library();
        let recent = page(12);
        played(&mut library, &[(1, 3), (2, 2), (3, 1)]);
        send(&mut library, LibraryCmd::HomeTracks(Some(Arc::clone(&recent))));
        let mixes = library.view.feed.mixes.clone();
        assert_eq!(mixes.len(), 3, "un mix per ciascuno dei tre artisti ascoltati");
        assert_eq!(mixes, Feed::build(&recent, &[], &library.view.stats, 7).mixes, "from the session seed");

        // Un artista seguito dopo: il feed si rifà (l'artista compare), e rientrando sulla Home
        // ancora, ma i mix restano gli stessi, brano per brano.
        send(&mut library, LibraryCmd::ToggleFollow(artist(5000)));
        send(&mut library, LibraryCmd::RefreshFeed);
        assert_eq!(library.view.feed.artists.first().map(|card| card.artist.id), Some(artist(5000).id));
        assert_eq!(library.view.feed.mixes, mixes);
        // È il seme a tenerli fermi: con un altro, gli stessi ascolti danno un altro ordine.
        let stats = &library.view.stats;
        assert_ne!(Feed::build(&recent, &[], stats, 1).mixes, Feed::build(&recent, &[], stats, 2).mixes);
    }

    #[test]
    fn the_feed_follows_plays_only_when_the_home_is_shown_again() {
        let mut library = library();
        send(&mut library, LibraryCmd::HomeTracks(Some(page(6))));
        assert!(library.view.feed.top.is_empty());
        played(&mut library, &[(1, 2)]);
        assert!(library.view.feed.top.is_empty(), "a play heard while on the Home does not move it");
        assert!(changes(&mut library, LibraryCmd::RefreshFeed));
        assert_eq!(library.view.feed.top.len(), 1);
        assert!(!changes(&mut library, LibraryCmd::RefreshFeed), "nothing changed since");
    }

    #[test]
    fn a_new_catalogue_clears_the_feed_until_its_tracks_arrive() {
        let mut library = library();
        let recent = page(4);
        assert!(changes(&mut library, LibraryCmd::HomeTracks(Some(Arc::clone(&recent)))));
        let source = library.view.feed_source.clone().unwrap();
        assert!(Arc::ptr_eq(&source, &recent), "the frontend recognizes its own tracks");
        assert!(!library.view.feed.releases.is_empty());
        assert!(
            !changes(&mut library, LibraryCmd::HomeTracks(Some(recent))),
            "the same tracks and the same inputs: the same feed"
        );
        assert!(changes(&mut library, LibraryCmd::HomeTracks(None)));
        assert!(library.view.feed.releases.is_empty() && library.view.feed_source.is_none());
        assert!(!changes(&mut library, LibraryCmd::HomeTracks(None)));
        assert!(!changes(&mut library, LibraryCmd::RefreshFeed), "no tracks, no feed");
    }

    /// La prima volta che si segue qualcuno le uscite già in catalogo non sono «novità»; da lì in
    /// poi contano quelle più recenti, finché non si guarda «Novità».
    #[test]
    fn news_count_only_what_comes_after_the_first_follow_and_after_the_last_visit() {
        let dated = |n: u128, created: &str, by: u128| {
            let mut t = track(n);
            t.album = None;
            t.created_at = created.into();
            t.artist = ArtistRef { id: Uuid::from_u128(by), name: format!("Artista {by}") };
            t
        };
        let first = dated(1, "2026-01-01", 10);
        let second = dated(2, "2026-02-01", 10);
        let mut library = library();
        send(&mut library, LibraryCmd::HomeTracks(Some(Arc::from([first.clone()]))));
        send(&mut library, LibraryCmd::ToggleFollow(artist(10)));
        assert_eq!(library.view.news_seen, "2026-01-01");
        assert_eq!((library.view.feed.news.len(), library.view.unread_news), (1, 0));

        send(&mut library, LibraryCmd::HomeTracks(Some(Arc::from([first, second]))));
        assert_eq!(library.view.unread_news, 1);
        assert!(changes(&mut library, LibraryCmd::MarkNewsSeen));
        assert_eq!((library.view.news_seen.as_str(), library.view.unread_news), ("2026-02-01", 0));
        assert!(!changes(&mut library, LibraryCmd::MarkNewsSeen));

        // Smettere di seguire non cancella ciò che si è visto.
        send(&mut library, LibraryCmd::ToggleFollow(artist(10)));
        assert!(library.view.feed.news.is_empty() && library.view.followed.is_empty());
        assert_eq!(library.view.news_seen, "2026-02-01");
    }

    #[test]
    fn the_feed_is_not_written_to_the_file() {
        let mut library = library();
        send(&mut library, LibraryCmd::HomeTracks(Some(page(3))));
        send(&mut library, LibraryCmd::RefreshFeed);
        assert!(!library.take_persist(), "only the user's data goes to config.json");
    }
}
