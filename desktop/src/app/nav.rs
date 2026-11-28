//! The app's pages and the history behind the «Indietro» and «Avanti» keys of the title bar
//! (Alt+← and Alt+→, and the mouse side keys). Pure logic: no view, no core.
//!
//! The history has a cap ([`HISTORY_CAP`] pages behind): beyond it, the oldest falls off. Going back to the
//! page one is already on adds nothing. When a page disappears (a deleted playlist) it is
//! removed from the whole history ([`History::forget`]), so «Indietro» does not lead to an empty page.

use std::collections::VecDeque;

use psytify_design::sprite;
use uuid::Uuid;

/// Pages kept behind the current one: beyond that, the oldest falls off.
pub const HISTORY_CAP: usize = 50;

/// One page of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Home,
    Search,
    Library,
    /// «Brani che ti piacciono».
    Liked,
    /// A local playlist.
    Playlist(Uuid),
    Downloads,
    /// «Novità»: the releases of the artists you follow.
    News,
    /// An album of the server's catalogue.
    Album(Uuid),
    /// An artist of the server's catalogue.
    Artist(Uuid),
    /// An album or a playlist from the official catalogue (or from Bandcamp, YouTube): the tab is kept by
    /// `Shell`, here there is only its fingerprint, so the page stays `Copy`.
    Source(SourceKey),
    /// Settings.
    Settings,
    /// «Accedi a Psytify»: password, email code, GitHub, Google, Discord.
    SignIn,
    /// Amici: searching for people, requests, friends with the track they are listening to.
    Friends,
    /// A person's profile (also one's own): `Shell` keeps the username, here there is its
    /// fingerprint, so the page stays `Copy`.
    Profile(PersonKey),
    /// A person's public playlist.
    FriendPlaylist(PersonKey, Uuid),
    /// Jam: listening together (create, join with a code, invitations, who is there).
    Jam,
}

/// The username fingerprint of a person.
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

/// The fingerprint of an external album or playlist: the source and its identifier.
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

/// The entries of the sidebar.
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
    /// The order: the Jam between Scaricati and Amici.
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

    /// Icon at rest and filled (when the entry is selected).
    pub fn icons(self) -> (&'static str, &'static str) {
        match self {
            Self::Home => (sprite::ICON_HOME, sprite::ICON_HOME_FILL),
            Self::Search => (sprite::ICON_SEARCH, sprite::ICON_SEARCH_FILL),
            Self::Library => (sprite::ICON_LIBRARY, sprite::ICON_LIBRARY_FILL),
            Self::News => (sprite::ICON_BELL, sprite::ICON_BELL_FILL),
            // The arrow is already solid; the check of `ICON_DOWNLOADED` means "saved on this device".
            Self::Downloads => (sprite::ICON_DOWNLOAD, sprite::ICON_DOWNLOAD),
            // The group for the Jam, the person for Amici.
            Self::Jam => (sprite::ICON_PEOPLE, sprite::ICON_PEOPLE_FILL),
            Self::Friends => (sprite::ICON_ACCOUNT, sprite::ICON_ACCOUNT),
            Self::Settings => (sprite::ICON_SETTINGS, sprite::ICON_SETTINGS_FILL),
        }
    }

    /// The page the entry opens.
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
    /// The sidebar entry lit on this page: the liked tracks, the playlists, the albums and
    /// the artists are in the Libreria; an album from the official catalogue opens
    /// from the search; sign-in, the profiles and the friends' playlists are in Amici.
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

    /// The page's name on the command line (`--page`): `playlist` opens the first playlist, `album`
    /// the first saved album, `artist` the first followed artist, `profile` one's own profile (or the one
    /// opened by the `--fixture` test data).
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

/// The page asked for on the command line: one page, or the library's first playlist (its id
/// is known only after reading the configuration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageArg {
    Page(Page),
    FirstPlaylist,
    FirstAlbum,
    FirstArtist,
    /// One's own profile (needs sign-in), or the one opened by the test data.
    Profile,
}

/// The current page, the ones behind (the most recent at the end) and the ones in front (the next at the end).
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

    /// Opens `page`. `false` if it is already the current one: the history does not change. A new page cancels
    /// the ones in front, as in a browser.
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

    /// Goes back to the previous page; `None` if there is none.
    pub fn back(&mut self) -> Option<Page> {
        let previous = self.back.pop_back()?;
        self.forward.push(self.current);
        self.current = previous;
        Some(previous)
    }

    /// Redoes the step undone by [`History::back`]; `None` if there is none.
    pub fn forward(&mut self) -> Option<Page> {
        let next = self.forward.pop()?;
        self.back.push_back(self.current);
        if self.back.len() > HISTORY_CAP {
            self.back.pop_front();
        }
        self.current = next;
        Some(next)
    }

    /// The pages in the history, the current one included.
    pub fn pages(&self) -> impl Iterator<Item = Page> + '_ {
        self.back.iter().copied().chain(std::iter::once(self.current)).chain(self.forward.iter().copied())
    }

    /// Removes from the history the pages that no longer exist, and the duplicates left side by side (two
    /// equal pages in a row, or next to the current one). The caller changes the current page
    /// first, with [`History::go`].
    pub fn forget(&mut self, gone: impl Fn(Page) -> bool) {
        self.back.retain(|page| !gone(*page));
        self.forward.retain(|page| !gone(*page));
        // Back: from the oldest to the current one.
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
        // Forward: from the farthest to the next one, which must not be the current one.
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
        // Whoever deletes the open playlist takes the current page away first, then forgets it.
        history.go(Page::Library);
        history.forget(|page| page == playlist(1));
        // Behind there were Library, Library, Liked: the two Library side by side become one.
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
        // Two profiles are two steps of the history; the same one twice is not.
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
