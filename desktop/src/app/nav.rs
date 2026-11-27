//! Le pagine dell'app e la cronologia dietro i tasti «Indietro» e «Avanti» della barra del titolo
//! (Alt+← e Alt+→, e i tasti laterali del mouse). Logica pura: nessuna vista, nessun core.
//!
//! La cronologia ha un fondo ([`HISTORY_CAP`] pagine dietro): oltre, cade la più vecchia. Tornare sulla
//! pagina in cui si è già non aggiunge niente. Quando una pagina sparisce (una playlist eliminata) la si
//! toglie da tutta la cronologia ([`History::forget`]), così «Indietro» non porta su una pagina vuota.

use std::collections::VecDeque;

use psytify_design::sprite;
use uuid::Uuid;

/// Pagine tenute dietro quella corrente: oltre, cade la più vecchia.
pub const HISTORY_CAP: usize = 50;

/// Una pagina dell'app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Home,
    Search,
    Library,
    /// «Brani che ti piacciono».
    Liked,
    /// Una playlist locale.
    Playlist(Uuid),
    Downloads,
    /// «Novità»: le uscite degli artisti seguiti.
    News,
    /// Un album del catalogo del server.
    Album(Uuid),
    /// Un artista del catalogo del server.
    Artist(Uuid),
    /// Un album o una playlist del catalogo ufficiale (o di Bandcamp, YouTube): la scheda la tiene
    /// `Shell`, qui c'è solo la sua impronta, così la pagina resta `Copy`.
    Source(SourceKey),
    /// Impostazioni.
    Settings,
    /// «Accedi a Psytify»: password, codice via email, GitHub, Google, Discord.
    SignIn,
    /// Amici: ricerca di persone, richieste, amici con il brano che ascoltano.
    Friends,
    /// Il profilo di una persona (anche il proprio): il nome utente lo tiene `Shell`, qui c'è la sua
    /// impronta, così la pagina resta `Copy`.
    Profile(PersonKey),
    /// La playlist pubblica di una persona.
    FriendPlaylist(PersonKey, Uuid),
    /// Jam: l'ascolto insieme (crea, entra con un codice, inviti, chi c'è).
    Jam,
}

/// L'impronta del nome utente di una persona.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PersonKey(u64);

impl PersonKey {
    pub fn of(handle: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "person".hash(&mut hasher);
        handle.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// L'impronta di un album o di una playlist esterna: la fonte e il suo identificatore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceKey(u64);

impl SourceKey {
    pub fn of(source: psytify_core::api::SourceKind, id: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.label().hash(&mut hasher);
        id.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// Le voci della barra laterale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Home,
    Search,
    Library,
    News,
    Downloads,
    Jam,
    Friends,
    Settings,
}

impl Nav {
    /// Nell'ordine del client Iced: la Jam fra Scaricati e Amici.
    pub const ALL: [Self; 8] = [
        Self::Home,
        Self::Search,
        Self::Library,
        Self::News,
        Self::Downloads,
        Self::Jam,
        Self::Friends,
        Self::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Search => "Cerca",
            Self::Library => "Libreria",
            Self::News => "Novità",
            Self::Downloads => "Scaricati",
            Self::Jam => "Jam",
            Self::Friends => "Amici",
            Self::Settings => "Impostazioni",
        }
    }

    /// Icona a riposo e piena (quando la voce è selezionata).
    pub fn icons(self) -> (&'static str, &'static str) {
        match self {
            Self::Home => (sprite::ICON_HOME, sprite::ICON_HOME_FILL),
            Self::Search => (sprite::ICON_SEARCH, sprite::ICON_SEARCH_FILL),
            Self::Library => (sprite::ICON_LIBRARY, sprite::ICON_LIBRARY_FILL),
            Self::News => (sprite::ICON_BELL, sprite::ICON_BELL_FILL),
            // The arrow is already solid; the check of `ICON_DOWNLOADED` means "saved on this device".
            Self::Downloads => (sprite::ICON_DOWNLOAD, sprite::ICON_DOWNLOAD),
            // Come nel client Iced: il gruppo per la Jam, la persona per gli Amici.
            Self::Jam => (sprite::ICON_PEOPLE, sprite::ICON_PEOPLE_FILL),
            Self::Friends => (sprite::ICON_ACCOUNT, sprite::ICON_ACCOUNT),
            Self::Settings => (sprite::ICON_SETTINGS, sprite::ICON_SETTINGS_FILL),
        }
    }

    /// La pagina che la voce apre.
    pub fn page(self) -> Page {
        match self {
            Self::Home => Page::Home,
            Self::Search => Page::Search,
            Self::Library => Page::Library,
            Self::News => Page::News,
            Self::Downloads => Page::Downloads,
            Self::Jam => Page::Jam,
            Self::Friends => Page::Friends,
            Self::Settings => Page::Settings,
        }
    }
}

impl Page {
    /// La voce della barra laterale accesa su questa pagina: i brani piaciuti, le playlist, gli album e
    /// gli artisti stanno nella Libreria (come nel client Iced); un album del catalogo ufficiale si apre
    /// dalla ricerca; l'accesso, i profili e le playlist degli amici stanno negli Amici.
    pub fn nav(self) -> Nav {
        match self {
            Self::Home => Nav::Home,
            Self::Search | Self::Source(_) => Nav::Search,
            Self::Library | Self::Liked | Self::Playlist(_) | Self::Album(_) | Self::Artist(_) => {
                Nav::Library
            }
            Self::Downloads => Nav::Downloads,
            Self::News => Nav::News,
            Self::Settings => Nav::Settings,
            Self::Jam => Nav::Jam,
            Self::SignIn | Self::Friends | Self::Profile(_) | Self::FriendPlaylist(..) => Nav::Friends,
        }
    }

    /// Il nome della pagina sulla riga di comando (`--page`): `playlist` apre la prima playlist, `album`
    /// il primo album salvato, `artist` il primo artista seguito, `profile` il proprio profilo (o quello
    /// aperto dai dati di prova di `--fixture`).
    pub fn from_arg(name: &str) -> Option<PageArg> {
        Some(match name {
            "home" => PageArg::Page(Self::Home),
            "search" => PageArg::Page(Self::Search),
            "library" => PageArg::Page(Self::Library),
            "liked" => PageArg::Page(Self::Liked),
            "downloads" => PageArg::Page(Self::Downloads),
            "news" => PageArg::Page(Self::News),
            "settings" => PageArg::Page(Self::Settings),
            "signin" => PageArg::Page(Self::SignIn),
            "friends" => PageArg::Page(Self::Friends),
            "jam" => PageArg::Page(Self::Jam),
            "profile" => PageArg::Profile,
            "playlist" => PageArg::FirstPlaylist,
            "album" => PageArg::FirstAlbum,
            "artist" => PageArg::FirstArtist,
            _ => return None,
        })
    }
}

/// La pagina chiesta dalla riga di comando: una pagina, o la prima playlist della libreria (il suo id
/// si sa solo dopo aver letto la configurazione).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageArg {
    Page(Page),
    FirstPlaylist,
    FirstAlbum,
    FirstArtist,
    /// Il proprio profilo (serve l'accesso), o quello aperto dai dati di prova.
    Profile,
}

/// La pagina corrente, quelle dietro (la più recente in fondo) e quelle davanti (la prossima in fondo).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    back: VecDeque<Page>,
    current: Page,
    forward: Vec<Page>,
}

impl History {
    pub fn new(start: Page) -> Self {
        Self { back: VecDeque::new(), current: start, forward: Vec::new() }
    }

    pub fn current(&self) -> Page {
        self.current
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Apre `page`. `false` se è già quella corrente: la cronologia non cambia. Una pagina nuova cancella
    /// quelle davanti, come in un browser.
    pub fn go(&mut self, page: Page) -> bool {
        if page == self.current {
            return false;
        }
        self.back.push_back(self.current);
        if self.back.len() > HISTORY_CAP {
            self.back.pop_front();
        }
        self.current = page;
        self.forward.clear();
        true
    }

    /// Torna alla pagina di prima; `None` se non ce n'è.
    pub fn back(&mut self) -> Option<Page> {
        let previous = self.back.pop_back()?;
        self.forward.push(self.current);
        self.current = previous;
        Some(previous)
    }

    /// Rifà il passo annullato da [`History::back`]; `None` se non ce n'è.
    pub fn forward(&mut self) -> Option<Page> {
        let next = self.forward.pop()?;
        self.back.push_back(self.current);
        if self.back.len() > HISTORY_CAP {
            self.back.pop_front();
        }
        self.current = next;
        Some(next)
    }

    /// Le pagine nella cronologia, la corrente compresa.
    pub fn pages(&self) -> impl Iterator<Item = Page> + '_ {
        self.back.iter().copied().chain(std::iter::once(self.current)).chain(self.forward.iter().copied())
    }

    /// Toglie dalla cronologia le pagine che non esistono più, e i doppioni che restano accostati (due
    /// pagine uguali di fila, o accanto a quella corrente). La pagina corrente la cambia chi chiama,
    /// prima, con [`History::go`].
    pub fn forget(&mut self, gone: impl Fn(Page) -> bool) {
        self.back.retain(|page| !gone(*page));
        self.forward.retain(|page| !gone(*page));
        // Indietro: dalla più vecchia alla corrente.
        let mut back: VecDeque<Page> = VecDeque::with_capacity(self.back.len());
        for page in self.back.drain(..) {
            if back.back() != Some(&page) {
                back.push_back(page);
            }
        }
        while back.back() == Some(&self.current) {
            back.pop_back();
        }
        self.back = back;
        // Avanti: dalla più lontana alla prossima, che non deve essere la corrente.
        self.forward.dedup();
        while self.forward.last() == Some(&self.current) {
            self.forward.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(n: u128) -> Page {
        Page::Playlist(Uuid::from_u128(n))
    }

    #[test]
    fn a_new_page_goes_behind_and_clears_what_was_ahead() {
        let mut history = History::new(Page::Search);
        assert!(!history.can_back() && !history.can_forward());
        assert!(history.go(Page::Library));
        assert!(history.go(Page::Liked));
        assert_eq!(history.back(), Some(Page::Library));
        assert!(history.can_forward());
        assert!(history.go(Page::Downloads));
        assert!(!history.can_forward(), "una pagina nuova cancella quelle davanti");
        assert_eq!(history.back(), Some(Page::Library));
        assert_eq!(history.back(), Some(Page::Search));
        assert_eq!(history.back(), None);
        assert_eq!(history.current(), Page::Search);
    }

    #[test]
    fn the_same_page_twice_is_not_a_step() {
        let mut history = History::new(Page::Search);
        assert!(!history.go(Page::Search));
        assert!(!history.can_back());
        assert!(history.go(playlist(1)));
        assert!(!history.go(playlist(1)));
        assert!(history.go(playlist(2)), "un'altra playlist è un'altra pagina");
        assert_eq!(history.back(), Some(playlist(1)));
    }

    #[test]
    fn back_and_forward_walk_the_same_steps() {
        let mut history = History::new(Page::Home);
        for page in [Page::Search, Page::Library, Page::Liked] {
            history.go(page);
        }
        assert_eq!(history.back(), Some(Page::Library));
        assert_eq!(history.back(), Some(Page::Search));
        assert_eq!(history.forward(), Some(Page::Library));
        assert_eq!(history.forward(), Some(Page::Liked));
        assert_eq!(history.forward(), None);
        assert_eq!(history.current(), Page::Liked);
    }

    #[test]
    fn the_history_has_a_bottom() {
        let mut history = History::new(playlist(0));
        for n in 1..=(HISTORY_CAP as u128 + 20) {
            history.go(playlist(n));
        }
        let mut steps = 0;
        while history.back().is_some() {
            steps += 1;
        }
        assert_eq!(steps, HISTORY_CAP);
        assert_eq!(history.current(), playlist(20), "le pagine più vecchie sono cadute");
        let mut ahead = 0;
        while history.forward().is_some() {
            ahead += 1;
        }
        assert_eq!(ahead, HISTORY_CAP);
    }

    #[test]
    fn a_deleted_playlist_leaves_the_history_without_doubles() {
        let mut history = History::new(Page::Library);
        for page in [playlist(1), Page::Library, playlist(1), Page::Liked, playlist(1)] {
            history.go(page);
        }
        // Chi elimina la playlist aperta prima porta via la pagina corrente, poi la dimentica.
        history.go(Page::Library);
        history.forget(|page| page == playlist(1));
        // Dietro restavano Library, Library, Liked: i due Library accostati diventano uno.
        assert_eq!(history.back(), Some(Page::Liked));
        assert_eq!(history.back(), Some(Page::Library));
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), Some(Page::Liked));
        assert_eq!(history.forward(), Some(Page::Library));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn a_double_next_to_the_current_page_goes_too() {
        let mut history = History::new(Page::Library);
        history.go(playlist(1));
        history.go(Page::Library);
        history.go(playlist(1));
        history.go(Page::Library);
        history.forget(|page| page == playlist(1));
        assert!(!history.can_back(), "Library dietro a Library non è un passo");
    }

    #[test]
    fn forgetting_also_clears_the_pages_ahead() {
        let mut history = History::new(Page::Search);
        history.go(playlist(7));
        history.go(Page::Downloads);
        history.back();
        history.back();
        assert_eq!(history.current(), Page::Search);
        history.forget(|page| page == playlist(7));
        assert_eq!(history.forward(), Some(Page::Downloads));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn liked_songs_and_playlists_light_the_library() {
        assert_eq!(Page::Liked.nav(), Nav::Library);
        assert_eq!(playlist(3).nav(), Nav::Library);
        assert_eq!(Page::Downloads.nav(), Nav::Downloads);
        for nav in Nav::ALL {
            assert_eq!(nav.page().nav(), nav);
        }
    }

    #[test]
    fn albums_and_artists_light_the_library_and_official_albums_the_search() {
        assert_eq!(Page::Album(Uuid::nil()).nav(), Nav::Library);
        assert_eq!(Page::Artist(Uuid::nil()).nav(), Nav::Library);
        let key = SourceKey::of(psytify_core::api::SourceKind::Deezer, "album/1");
        assert_eq!(Page::Source(key).nav(), Nav::Search);
        assert_eq!(key, SourceKey::of(psytify_core::api::SourceKind::Deezer, "album/1"));
        assert_ne!(key, SourceKey::of(psytify_core::api::SourceKind::Deezer, "album/2"));
        assert_ne!(key, SourceKey::of(psytify_core::api::SourceKind::Bandcamp, "album/1"));
    }

    #[test]
    fn sign_in_profiles_and_friend_playlists_light_the_friends() {
        let luca = PersonKey::of("luca");
        assert_eq!(luca, PersonKey::of("luca"));
        assert_ne!(luca, PersonKey::of("sara"));
        for page in
            [Page::SignIn, Page::Friends, Page::Profile(luca), Page::FriendPlaylist(luca, Uuid::nil())]
        {
            assert_eq!(page.nav(), Nav::Friends);
        }
        // Due profili sono due passi della cronologia; lo stesso due volte no.
        let mut history = History::new(Page::Friends);
        assert!(history.go(Page::Profile(luca)));
        assert!(!history.go(Page::Profile(luca)));
        assert!(history.go(Page::Profile(PersonKey::of("sara"))));
        assert_eq!(history.back(), Some(Page::Profile(luca)));
    }

    #[test]
    fn the_history_lists_every_page_it_holds() {
        let mut history = History::new(Page::Search);
        history.go(Page::Album(Uuid::from_u128(1)));
        history.go(Page::Library);
        history.back();
        let pages: Vec<Page> = history.pages().collect();
        assert_eq!(pages, vec![Page::Search, Page::Album(Uuid::from_u128(1)), Page::Library]);
    }

    #[test]
    fn pages_have_a_command_line_name() {
        assert_eq!(Page::from_arg("liked"), Some(PageArg::Page(Page::Liked)));
        assert_eq!(Page::from_arg("downloads"), Some(PageArg::Page(Page::Downloads)));
        assert_eq!(Page::from_arg("settings"), Some(PageArg::Page(Page::Settings)));
        assert_eq!(Page::from_arg("playlist"), Some(PageArg::FirstPlaylist));
        assert_eq!(Page::from_arg("album"), Some(PageArg::FirstAlbum));
        assert_eq!(Page::from_arg("artist"), Some(PageArg::FirstArtist));
        assert_eq!(Page::from_arg("signin"), Some(PageArg::Page(Page::SignIn)));
        assert_eq!(Page::from_arg("friends"), Some(PageArg::Page(Page::Friends)));
        assert_eq!(Page::from_arg("jam"), Some(PageArg::Page(Page::Jam)));
        assert_eq!(Page::from_arg("profile"), Some(PageArg::Profile));
        assert_eq!(Page::from_arg("disco"), None);
    }
}
