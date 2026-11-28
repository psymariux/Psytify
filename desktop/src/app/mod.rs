//! The desktop app on GPUI: the window with a client-side title bar, sidebar, page,
//! side panel (In riproduzione, Testo, Coda) and the «in riproduzione» bar, or the stage in their place
//! (`stage_view`). Each piece is a view (entity) that subscribes to the
//! `CoreModel` and redraws only when its domain changes; the root keeps the current page,
//! the open panel, the notices (one system for the events of all domains, see `notices`) and the
//! shutdown.
//!
//! Pages (`nav::Page`, with the back/forward history of `nav::History`): Cerca, Libreria, «Brani che
//! ti piacciono» and a playlist (`collection_page`), Scaricati, an album (`album_page`), an artist
//! (`artist_page`), an album or a playlist from the official catalogue or another source (`source_page`),
//! the Home (`home_page`, the start page), sign-in and Amici with the profiles, and
//! Jam (`jam_page`, listening together). Above everything, `Shell` holds the playlist dialog
//! (`playlist_dialog`, also the confirmation to remove a friend) and the «Altro» menu of a track
//! (`track_menu`), which the pages request with a `PageEvent`. Everything that can be done goes through a
//! command to the core: the window keeps no copies of the state, only the `Arc`s of the views and the
//! texts already ready.

mod album_page;
mod artist_page;
mod chrome;
mod collection_page;
mod downloads_page;
mod friends_page;
mod home_page;
mod jam_page;
mod library_page;
mod lyrics_view;
mod media;
mod nav;
mod news_page;
mod now_panel;
mod page_frame;
mod page_list;
mod people;
mod player_bar;
mod playlist_dialog;
mod profile_page;
mod queue_panel;
mod search_page;
pub mod settings_page;
pub mod sidebar;
mod sign_in_page;
mod source_page;
mod stage_view;
mod track_list;
mod track_menu;
mod update_banner;

pub use nav::{Page, PageArg};

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, NavigationDirection, ParentElement, Render, StatefulInteractiveElement, StyleRefinement,
    Styled, Subscription, Window, deferred, div,
};
use psytify_core::api::SourcePlaylist;
use psytify_core::command::{CatalogCmd, DownloadsCmd, ExternalCmd, LibraryCmd, PlaybackCmd};
use psytify_core::config::{FollowedArtist, SavedAlbum};
use psytify_core::player::PlaybackState;
use psytify_core::snapshot::{Account, PlaybackView, SocialView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SHADOW_HARD, SURFACE_CANVAS, SURFACE_CHROME};
use psytify_design::tokens::motion::{MOTION_PANEL, TOAST_HOLD_MS};
use psytify_design::tokens::size::{
    BP_PANEL_DOCK, BP_RAIL, PLAYER_BAR_H, SIDE_PANEL_W, SIDEBAR_GRIP_W, TITLE_BAR_H,
};
use psytify_design::tokens::space::{SPACE_3, SPACE_5};

use crate::actions::{
    CycleRepeat, FocusNext, FocusPrev, FocusSearch, GoBack, GoForward, NextTrack, OpenSettings,
    PreviousTrack, Quit, SHELL_CONTEXT, ToggleFocus as ToggleFocusAction, ToggleLyrics as ToggleLyricsAction,
    TogglePlay, ToggleQueue as ToggleQueueAction, ToggleShuffle, ToggleSidebar, ZoomIn, ZoomOut, ZoomReset,
};
use crate::components::indicators::PlayState;
use crate::components::menu::{MenuItem, MenuPanel, popup};
use crate::components::navigation::Tabs;
use crate::components::overlay::{Toast, scrim};
use crate::components::playing_spot::{PlayingSpot, overlay as spot_overlay};
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::motion::{Tween, ap_round};
use crate::notices::{Notice, Pushed, Seen, Toasts};
use crate::pixel::shape::hard_shadow;
use crate::theme::{Ap, col, dp, to_dp};
use crate::titlebar::{AccountChip, HistoryButtons, OnClose, TitleBarState, title_row, window_frame};
use crate::zoom::ZoomPref;
use album_page::AlbumPage;
use artist_page::ArtistPage;
use chrome::{Chrome, Key as ChromeKey, Part};
use collection_page::{Collection, CollectionPage};
use downloads_page::DownloadsPage;
use friends_page::FriendsPage;
use home_page::HomePage;
use jam_page::JamPage;
use library_page::LibraryPage;
use lyrics_view::{Look, LyricsView};
use nav::{History, PersonKey, SourceKey};
use news_page::NewsPage;
use now_panel::{NowEvent, NowPanel};
use player_bar::{BarEvent, PlayerBar};
use playlist_dialog::{DialogEvent, DialogRequest, PlaylistDialog};
use profile_page::ProfilePage;
use queue_panel::QueuePanel;
use search_page::SearchPage;
use settings_page::SettingsPage;
use sidebar::{Layout as SidebarLayout, Mode as SidebarMode, Prefs as SidebarPrefs, Sidebar};
use sign_in_page::SignInPage;
use source_page::SourcePage;
use stage_view::{StageView, StageViewEvent};
use track_menu::{
    DownloadItems, MenuChoice, MenuLevel, TrackMenu, choice_notice, copy_text, download_label, follow_item,
    playlist_items, quality_items,
};
use update_banner::{BANNER_H, Place, UpdateBanner, update_line};

#[cfg(feature = "video")]
use crate::actions::ToggleVideoFocus;
#[cfg(feature = "video")]
use crate::video::stage::{StageEvent, Surface, VideoStage};

/// How long the window waits for the core to save before quitting anyway.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// The side panel's tabs (desktop v2 §5): the record, the words, the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    Now,
    Lyrics,
    Queue,
}

impl SidePanel {
    pub const ALL: [SidePanel; 3] = [SidePanel::Now, SidePanel::Lyrics, SidePanel::Queue];

    pub fn label(self) -> &'static str {
        match self {
            SidePanel::Now => "In riproduzione",
            SidePanel::Lyrics => "Testo",
            SidePanel::Queue => "Coda",
        }
    }

    fn ix(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

/// Where the clip shows: the stage's media slot while the stage is open, else the side panel's on
/// *In riproduzione*; only with the clip on and a track that has one. Pure.
#[cfg(feature = "video")]
pub fn video_surface(panel: Option<SidePanel>, stage_open: bool, video_on: bool) -> Option<Surface> {
    if !video_on {
        None
    } else if stage_open {
        Some(Surface::Stage)
    } else {
        (panel == Some(SidePanel::Now)).then_some(Surface::Panel)
    }
}

/// The panel after the button of `kind`: it opens on that tab, or closes if it was that one.
pub fn toggled(open: Option<SidePanel>, kind: SidePanel) -> Option<SidePanel> {
    if open == Some(kind) { None } else { Some(kind) }
}

/// How to show the playing track in lists: lime and animated only while it plays.
pub fn play_state(view: &PlaybackView) -> Option<PlayState> {
    match view.state {
        PlaybackState::Playing => Some(PlayState::Playing),
        PlaybackState::Paused => Some(PlayState::Paused),
        PlaybackState::Loading { .. } => Some(PlayState::Buffering),
        PlaybackState::Stopped | PlaybackState::Error(_) => None,
    }
}

/// What a page asks of `Shell`: open another page or a dialog (which sits above the whole
/// window, not inside the page), and the gestures that give a notice (only `Shell` shows notices).
#[derive(Clone, Debug, PartialEq)]
pub enum PageEvent {
    Open(Page),
    Dialog(DialogRequest),
    /// The «Altro» menu of a track.
    TrackMenu(Box<TrackMenu>),
    /// Opens the page of an album or a playlist from an external source.
    OpenSource(Box<SourcePlaylist>),
    /// Saves the open external playlist to the library (it is opened when the core publishes it).
    SaveSource,
    /// Saves an album to the library or removes it.
    ToggleSaveAlbum(Box<SavedAlbum>),
    /// Follows an artist or stops.
    ToggleFollow(Box<FollowedArtist>),
    /// Opens the settings on the Account tab (the profile form).
    OpenAccountSettings,
    /// Opens the profile of a person (their username).
    OpenProfile(String),
    /// Opens the public playlist `id` of a person.
    OpenFriendPlaylist {
        handle: String,
        id: uuid::Uuid,
    },
    /// Searches for this text (puts it in the field and opens Cerca).
    Search(gpui::SharedString),
    /// A notice (only `Shell` shows notices).
    Notice(Notice),
}

/// The equaliser spots of each view that shows it (see `components::playing_spot`).
struct Spots {
    home: PlayingSpot,
    search: PlayingSpot,
    downloads: PlayingSpot,
    collection: PlayingSpot,
    album: PlayingSpot,
    artist: PlayingSpot,
    profile: PlayingSpot,
    queue: PlayingSpot,
}

/// The external pages' tabs that the history can still reopen: beyond that, the ones that
/// no page in the history names are dropped.
const SOURCE_PAGES: usize = 64;

/// The open «Altro» menu.
struct OpenMenu {
    menu: TrackMenu,
    level: MenuLevel,
    playlists: Vec<(uuid::Uuid, gpui::SharedString)>,
    /// Already follows the track's artist (when the menu opens).
    following: bool,
    /// The download entries (when the menu opens).
    downloads: DownloadItems,
}

/// The playlist list in the menu scrolls beyond ten entries.
const MENU_LIST_MAX_H: f32 = 10.0 * psytify_design::tokens::size::MENU_ITEM_H;

/// How to open the app (from the command line): a search already launched and the queue panel open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    pub search: Option<String>,
    pub queue: bool,
    /// The side panel open on the lyrics (`--lyrics`).
    pub lyrics: bool,
    /// The stage open (`--focus`), if a track is queued.
    pub focus: bool,
    /// The side panel open on *In riproduzione* (`--now`).
    pub now: bool,
    /// Plays the first result of the official catalogue as soon as it arrives (to test the import without
    /// touching anything: the row shows «Apro…» while the server works).
    pub play_first_official: bool,
    /// The page to open (`--page`): for screenshots and measurements, where nothing is touched.
    pub page: Option<PageArg>,
    /// The social test data (`--fixture`, only with the `fixtures` feature).
    #[cfg(feature = "fixtures")]
    pub fixture: Option<String>,
    /// The clip on in the media slot (`--video`): the panel's, or with `--focus` the stage's.
    #[cfg(feature = "video")]
    pub video: bool,
}

pub struct Shell {
    core: Entity<CoreModel>,
    focus: FocusHandle,
    search: Entity<SearchField>,
    bar: Entity<PlayerBar>,
    /// The update banner under the title bar, and whether there is one (only then it enters the window).
    banner: Entity<UpdateBanner>,
    banner_shown: bool,
    queue: Entity<QueuePanel>,
    /// *Testo* and *In riproduzione* in the side panel.
    lyrics: Entity<LyricsView>,
    now_panel: Entity<NowPanel>,
    /// The stage: in place of page, panel and bar when open; over them while it moves.
    stage_view: Entity<StageView>,
    stage_open: bool,
    /// The reveal (0 bar, 1 stage) eased in `MOTION_PANEL` and shared with the stage view.
    stage_t: Tween,
    stage_reveal: stage_view::Reveal,
    /// The stage was moving at the last draw (its end notifies the cached view once).
    stage_moving: bool,
    /// The window's single clip stage.
    #[cfg(feature = "video")]
    video: Entity<VideoStage>,
    /// The clip is on in the media slot (panel or stage).
    video_on: bool,
    /// The core knows that someone shows the lyrics (`PlaybackCmd::Lyrics`).
    lyrics_wanted: bool,
    search_page: Entity<SearchPage>,
    home_page: Entity<HomePage>,
    news_page: Entity<NewsPage>,
    downloads_page: Entity<DownloadsPage>,
    settings_page: Entity<SettingsPage>,
    sign_in_page: Entity<SignInPage>,
    friends_page: Entity<FriendsPage>,
    profile_page: Entity<ProfilePage>,
    jam_page: Entity<JamPage>,
    library_page: Entity<LibraryPage>,
    /// «Brani che ti piacciono» and the playlists: one view that changes the collection.
    collection_page: Entity<CollectionPage>,
    /// One view per kind of object page: changes the album, the artist or the tab it shows.
    album_page: Entity<AlbumPage>,
    artist_page: Entity<ArtistPage>,
    source_page: Entity<SourcePage>,
    /// The external pages' tabs of the history (`Page::Source` keeps only their fingerprint).
    sources: HashMap<SourceKey, SourcePlaylist>,
    /// The usernames of the people's pages in the history (`Page::Profile` keeps only the fingerprint).
    people: HashMap<PersonKey, String>,
    /// The account in the title bar, and one's own username (to open one's own profile).
    account: AccountChip,
    own_handle: Option<String>,
    /// Incoming friend requests: the number on the «Amici» entry.
    incoming: usize,
    /// Incoming jam invitations: the number on the «Jam» entry.
    invites: usize,
    /// The equaliser spots of the views with track rows: `Shell` paints it over the view
    /// (BACKLOG 23), so the sprite's frame does not redo the cached page.
    spots: Spots,
    /// The title bar and the sidebar in cache (`chrome.rs`), with what they showed at the last
    /// comparison: they redraw only when it changes.
    title_view: Entity<Chrome>,
    sidebar_view: Entity<Chrome>,
    chrome_keys: Option<(ChromeKey, ChromeKey)>,
    /// The playlist dialog, above everything, if it is open; with whoever had the focus before.
    dialog: Option<(Entity<PlaylistDialog>, Option<FocusHandle>, Subscription)>,
    /// The playlist just created: it opens when the core publishes it.
    pending_playlist: Option<uuid::Uuid>,
    /// The open «Altro» menu: the track, the panel and the playlists (id and name) taken on opening.
    menu: Option<OpenMenu>,
    /// Downloads queued or in progress: the number on the «Scaricati» entry of the sidebar.
    busy_downloads: usize,
    /// Releases from your artists not yet seen: the number on the «Novità» entry.
    unread_news: usize,
    /// The last page shown: leaving «Novità» counts what was there as seen.
    shown: Page,
    /// The current page and the ones behind and in front of it (Indietro and Avanti).
    history: History,
    /// The side panel open, and the tab it showed last (drawn while it slides shut).
    panel: Option<SidePanel>,
    panel_last: SidePanel,
    /// The mark in the title bar is lime only while it plays.
    playing: bool,
    /// The notice on the display and the ones waiting; the timer runs only while a notice is shown.
    toasts: Toasts,
    /// The last event of each domain already shown (by sequence number).
    seen: Seen,
    /// Closing has started: it waits for the core to save.
    closing: bool,
    /// The UI zoom preference (`Auto` or pinned); the zoom in effect lives in `theme::zoom`.
    zoom_pref: ZoomPref,
    /// The sidebar: mode, width, library tabs and filter, the jam card.
    sidebar: Sidebar,
    /// The sidebar showed the rail at the last draw (for `ChromeKey::Sidebar`).
    sidebar_rail: bool,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(core: Entity<CoreModel>, launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.entity().downgrade();
        let initial = launch.search.clone().unwrap_or_default();
        let search = cx.new(|cx| {
            SearchField::new("title-search", "Cerca brani, album, artisti", cx)
                .with_text(initial)
                .on_change({
                    let core = core.clone();
                    let shell = this.clone();
                    move |text, _, cx| {
                        // Every key goes to the core, which waits 300 ms after the last one before searching.
                        core.read(cx).catalog(CatalogCmd::SearchInput(text.to_owned()));
                        shell.update(cx, |shell, cx| shell.show(Page::Search, cx)).ok();
                    }
                })
                .on_submit({
                    let core = core.clone();
                    move |text, _, cx| core.read(cx).catalog(CatalogCmd::Search(text.trim().to_owned()))
                })
        });
        let filter = cx.new(|cx| {
            SearchField::new("sidebar-filter", "Filtra", cx).without_hint().on_change({
                let shell = this.clone();
                move |text, _, cx| {
                    shell
                        .update(cx, |shell, cx| {
                            if shell.sidebar.set_filter(text) {
                                cx.notify();
                            }
                        })
                        .ok();
                }
            })
        });
        let bar = cx.new(|cx| PlayerBar::new(core.clone(), cx));
        let banner_line = update_line(core.read(cx).update_view(), Place::Banner);
        let banner = cx.new(|_| UpdateBanner::new(core.clone()));
        let banner_shown = banner.update(cx, |banner, cx| banner.set(banner_line, cx));
        let queue = cx.new(|cx| QueuePanel::new(core.clone(), cx));
        let lyrics = cx.new(|cx| LyricsView::new(core.clone(), Look::Panel, cx));
        #[cfg(feature = "video")]
        let video = cx.new(|cx| VideoStage::new(core.clone(), cx));
        let now_panel = cx.new(|cx| {
            NowPanel::new(
                core.clone(),
                #[cfg(feature = "video")]
                video.clone(),
                cx,
            )
        });
        let stage_reveal: stage_view::Reveal = Rc::new(Cell::new(0.0));
        let stage_view = cx.new(|cx| {
            StageView::new(
                core.clone(),
                Rc::clone(&stage_reveal),
                #[cfg(feature = "video")]
                video.clone(),
                cx,
            )
        });
        let play_first_official = launch.play_first_official;
        let search_page = cx.new(|cx| SearchPage::new(core.clone(), play_first_official, cx));
        let home_page = cx.new(|cx| HomePage::new(core.clone(), cx));
        let news_page = cx.new(|cx| NewsPage::new(core.clone(), cx));
        let downloads_page = cx.new(|cx| DownloadsPage::new(core.clone(), cx));
        let settings_page = cx.new(|cx| SettingsPage::new(core.clone(), cx));
        let sign_in_page = cx.new(|cx| SignInPage::new(core.clone(), cx));
        let friends_page = cx.new(|cx| FriendsPage::new(core.clone(), cx));
        let profile_page = cx.new(|cx| ProfilePage::new(core.clone(), cx));
        let jam_page = cx.new(|cx| JamPage::new(core.clone(), cx));
        let library_page = cx.new(|cx| LibraryPage::new(core.clone(), cx));
        let collection_page = cx.new(|cx| CollectionPage::new(core.clone(), cx));
        let album_page = cx.new(|cx| AlbumPage::new(core.clone(), cx));
        let artist_page = cx.new(|cx| ArtistPage::new(core.clone(), cx));
        let source_page = cx.new(|cx| SourcePage::new(core.clone(), cx));
        let spots = Spots {
            home: home_page.read(cx).spot(),
            search: search_page.read(cx).spot(),
            downloads: downloads_page.read(cx).spot(),
            collection: collection_page.read(cx).spot(),
            album: album_page.read(cx).spot(),
            artist: artist_page.read(cx).spot(),
            profile: profile_page.read(cx).spot(),
            queue: queue.read(cx).spot(),
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let snapshot = core.read(cx).snapshot();
        let device_error = snapshot.playback.device_error.as_ref().map(ToString::to_string);
        let social = core.read(cx).social_view();
        let weak = cx.weak_entity();
        let title_view = cx.new(|_| Chrome::new(weak.clone(), Part::TitleBar));
        let sidebar_view = cx.new(|_| Chrome::new(weak, Part::Sidebar));
        let mut shell = Self {
            focus,
            search,
            bar,
            banner,
            banner_shown,
            queue,
            lyrics,
            now_panel,
            stage_view,
            stage_open: false,
            stage_t: Tween::at(0.0),
            stage_reveal,
            stage_moving: false,
            #[cfg(feature = "video")]
            video,
            video_on: false,
            lyrics_wanted: false,
            search_page,
            home_page,
            news_page,
            downloads_page,
            settings_page,
            sign_in_page,
            friends_page,
            profile_page,
            jam_page,
            library_page,
            collection_page,
            album_page,
            artist_page,
            source_page,
            sources: HashMap::new(),
            people: HashMap::new(),
            account: account_chip(&social.account),
            own_handle: own_handle(&social),
            incoming: incoming(&social),
            invites: social.jam_invites.len(),
            spots,
            title_view,
            sidebar_view,
            chrome_keys: None,
            dialog: None,
            pending_playlist: None,
            menu: None,
            busy_downloads: snapshot.downloads.busy_count(),
            unread_news: snapshot.library.unread_news,
            shown: Page::Home,
            history: History::new(Page::Home),
            panel: if launch.lyrics {
                Some(SidePanel::Lyrics)
            } else if launch.queue {
                Some(SidePanel::Queue)
            } else if launch.now {
                Some(SidePanel::Now)
            } else {
                None
            },
            panel_last: SidePanel::Now,
            playing: snapshot.playback.is_playing(),
            toasts: Toasts::default(),
            seen: Seen::at_start(&snapshot),
            closing: false,
            zoom_pref: ZoomPref::read(&snapshot.settings.ui),
            sidebar: Sidebar::new(
                SidebarPrefs::read(&snapshot.settings.ui),
                filter,
                Arc::clone(&snapshot.library),
                &core.read(cx).jam_view(),
            ),
            sidebar_rail: false,
            _subscriptions: Vec::new(),
            core: core.clone(),
        };
        // Every time `Shell` notifies: the title bar and the sidebar redraw only if what they show
        // changes, compared with what they show now.
        shell.chrome_keys = Some((shell.chrome_key(Part::TitleBar), shell.chrome_key(Part::Sidebar)));
        shell._subscriptions.push(cx.observe_self(|this, cx| this.sync_chrome(cx)));
        // `Auto` zoom follows the window width.
        shell
            ._subscriptions
            .push(cx.observe_window_bounds(window, |this, window, cx| this.apply_zoom(window, cx)));
        shell.apply_zoom(window, cx);
        shell._subscriptions.push(cx.subscribe_in(
            &core,
            window,
            |this, core, changed: &Changed, window, cx| {
                this.core_changed(core, *changed, window, cx);
            },
        ));
        shell._subscriptions.push(cx.subscribe(&shell.bar, |this, _, event: &BarEvent, cx| match event {
            BarEvent::ToggleQueue => this.toggle_panel(SidePanel::Queue, cx),
            BarEvent::ToggleLyrics => this.toggle_panel(SidePanel::Lyrics, cx),
            BarEvent::ToggleNow => this.toggle_panel(SidePanel::Now, cx),
            BarEvent::ToggleFocus => this.toggle_focus(cx),
            BarEvent::OpenArtist(id) => this.show(Page::Artist(*id), cx),
            BarEvent::OpenJam => this.show(Page::Jam, cx),
            #[cfg(feature = "video")]
            BarEvent::ToggleVideo => this.toggle_video(cx),
        }));
        shell._subscriptions.push(cx.subscribe(&shell.stage_view, |this, _, event: &StageViewEvent, cx| {
            match event {
                StageViewEvent::Close => this.set_focus(false, cx),
                StageViewEvent::Video(on) => this.set_video_on(*on, cx),
            }
        }));
        shell._subscriptions.push(cx.subscribe(
            &shell.now_panel,
            |this, _, event: &NowEvent, cx| match event {
                NowEvent::OpenLyrics => this.toggle_panel(SidePanel::Lyrics, cx),
                NowEvent::OpenArtist(id) => this.show(Page::Artist(*id), cx),
                NowEvent::Video(on) => this.set_video_on(*on, cx),
            },
        ));
        #[cfg(feature = "video")]
        {
            // A clip that stops is a notice like any other.
            shell._subscriptions.push(cx.subscribe(&shell.video, |this, _, event: &StageEvent, cx| {
                if let StageEvent::Notice(notice) = event {
                    this.notify(notice.clone(), cx);
                }
            }));
        }
        shell._subscriptions.push(cx.subscribe_in(
            &shell.search_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.home_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.downloads_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.news_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.library_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.collection_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.album_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.artist_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.settings_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.friends_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.profile_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.jam_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.sign_in_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        shell._subscriptions.push(cx.subscribe_in(
            &shell.source_page,
            window,
            |this, _, event: &PageEvent, window, cx| this.page_event(event, window, cx),
        ));
        // Safety net for the quits that do not go through `request_close` (the system ending the app):
        // GPUI waits for this future only briefly (`SHUTDOWN_TIMEOUT`), usually enough for the save.
        shell._subscriptions.push(cx.on_app_quit(|this, cx| {
            let done = this.core.update(cx, |core, _| core.begin_shutdown());
            async move {
                if let Some(done) = done {
                    let _ = done.await;
                }
            }
        }));
        // The Home always loads: it is the start page and the feed comes from it (the
        // other artists of the artist page, BACKLOG 32), whatever page is open.
        core.read(cx).catalog(CatalogCmd::Ensure(psytify_core::command::Resource::Home));
        if let Some(text) = launch.search.as_deref() {
            core.read(cx).catalog(CatalogCmd::Search(text.trim().to_owned()));
            shell.history = History::new(Page::Search);
        }
        if launch.focus && snapshot.playback.queue.current().is_some() {
            shell.stage_open = true;
            shell.stage_t = Tween::at(1.0);
            shell.stage_reveal.set(1.0);
        }
        #[cfg(feature = "video")]
        if launch.video {
            shell.video_on = true;
            if !shell.stage_open && shell.panel.is_none() {
                shell.panel = Some(SidePanel::Now);
            }
        }
        if let Some(tab) = shell.panel {
            shell.panel_last = tab;
        }
        shell.sync_panels(cx);
        match launch.page {
            Some(PageArg::Page(page)) => shell.history = History::new(page),
            Some(PageArg::FirstPlaylist) => {
                if let Some(first) = snapshot.library.playlists.first() {
                    shell.history = History::new(Page::Playlist(first.id));
                }
            }
            Some(PageArg::FirstAlbum) => {
                if let Some(first) = snapshot.library.saved_albums.first() {
                    shell.history = History::new(Page::Album(first.id));
                }
            }
            Some(PageArg::FirstArtist) => {
                if let Some(first) = snapshot.library.followed.first() {
                    shell.history = History::new(Page::Artist(first.id));
                }
            }
            Some(PageArg::Profile) => {
                // One's own profile, or (with the test data) the one the view has opened.
                let handle =
                    social.open.as_ref().map(|(handle, _)| handle.clone()).or(shell.own_handle.clone());
                if let Some(handle) = handle {
                    let key = PersonKey::of(&handle);
                    shell.people.insert(key, handle);
                    shell.history = History::new(Page::Profile(key));
                }
            }
            None => {}
        }
        shell.page_shown(true, cx);
        // The test data of a failed update carry the notice the core would give.
        #[cfg(feature = "fixtures")]
        if let Some(notice) = launch.fixture.as_deref().and_then(crate::fixtures::notice) {
            shell.notify(notice, cx);
        }
        if let Some(message) = device_error {
            shell.notify(Notice::error(message), cx);
        } else if let Some(message) = core.read(cx).start_error() {
            let message = format!("Il core non è partito: {message}");
            shell.notify(Notice::error(message), cx);
        }
        shell
    }

    /// What to do when the core publishes: notices and the mark. The views update by themselves.
    fn core_changed(
        &mut self,
        core: &Entity<CoreModel>,
        changed: Changed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = core.read(cx).snapshot();
        if changed.settings {
            let pref = ZoomPref::read(&snapshot.settings.ui);
            if pref != self.zoom_pref {
                self.zoom_pref = pref;
                self.apply_zoom(window, cx);
            }
        }
        if changed.playback {
            let playing = snapshot.playback.is_playing();
            if playing != self.playing {
                self.playing = playing;
                cx.notify();
            }
            if self.stage_open && snapshot.playback.queue.current().is_none() {
                // Without a track the stage has nothing to show.
                self.set_focus(false, cx);
            }
            // A track with or without a clip moves the clip's place.
            #[cfg(feature = "video")]
            self.sync_panels(cx);
        }
        if changed.library
            && let Some(id) = self.pending_playlist
            && snapshot.library.playlists.iter().any(|playlist| playlist.id == id)
        {
            // The playlist just created is in the library: it is opened.
            self.pending_playlist = None;
            self.show(Page::Playlist(id), cx);
        }
        if changed.library && self.sidebar.take_library(Arc::clone(&snapshot.library)) {
            cx.notify();
        }
        if changed.jam && self.sidebar.take_jam(&core.read(cx).jam_view()) {
            cx.notify();
        }
        if changed.library && snapshot.library.unread_news != self.unread_news {
            self.unread_news = snapshot.library.unread_news;
            cx.notify();
        }
        if changed.downloads {
            let busy = snapshot.downloads.busy_count();
            if busy != self.busy_downloads {
                self.busy_downloads = busy;
                cx.notify();
            }
            // The banner redraws itself if its row changes; `Shell` only when it appears or disappears.
            let line = update_line(core.read(cx).update_view(), Place::Banner);
            let shown = self.banner.update(cx, |banner, cx| banner.set(line, cx));
            if shown != self.banner_shown {
                self.banner_shown = shown;
                cx.notify();
            }
        }
        if changed.social {
            self.social_changed(&core.read(cx).social_view(), cx);
        }
        if changed.catalog {
            let loading = snapshot.catalog.search.results.is_loading();
            self.search.update(cx, |field, cx| field.set_loading(loading, cx));
        }
        // The new events of each changed domain, in order: one system for all.
        let heard = self.seen.collect(&snapshot, changed);
        if heard.raise {
            window.activate_window();
        }
        for notice in heard.notices {
            self.notify(notice, cx);
        }
        if let Some(id) = heard.open_artist {
            // The official catalogue's artist asked for by the search is on the server: its page opens.
            self.show(Page::Artist(id), cx);
        }
    }

    /// The social view changed: the account in the bar, the number of requests and, once signed in,
    /// leaving the sign-in page.
    fn social_changed(&mut self, social: &SocialView, cx: &mut Context<Self>) {
        let chip = account_chip(&social.account);
        let handle = own_handle(social);
        let requests = incoming(social);
        let invites = social.jam_invites.len();
        let signed_in = matches!(chip, AccountChip::SignedIn { .. });
        if chip != self.account
            || handle != self.own_handle
            || requests != self.incoming
            || invites != self.invites
        {
            self.account = chip;
            self.own_handle = handle;
            self.incoming = requests;
            self.invites = invites;
            cx.notify();
        }
        if signed_in && self.history.current() == Page::SignIn {
            if self.history.can_back() {
                self.go_back(cx);
            } else {
                self.show(Page::Home, cx);
            }
            self.history.forget(|page| page == Page::SignIn);
        }
    }

    /// Opens the profile of `handle` (its name stays in `people` while the history names it).
    fn open_person(&mut self, handle: &str, playlist: Option<uuid::Uuid>, cx: &mut Context<Self>) {
        let key = PersonKey::of(handle);
        self.people.insert(key, handle.to_owned());
        let page = match playlist {
            Some(id) => Page::FriendPlaylist(key, id),
            None => Page::Profile(key),
        };
        self.show(page, cx);
        if self.people.len() > SOURCE_PAGES {
            let kept: std::collections::HashSet<PersonKey> = self
                .history
                .pages()
                .filter_map(|page| match page {
                    Page::Profile(key) | Page::FriendPlaylist(key, _) => Some(key),
                    _ => None,
                })
                .collect();
            self.people.retain(|key, _| kept.contains(key));
        }
    }

    /// A click on the account in the bar: sign-in, or one's own profile (without a username: Amici, which
    /// asks to choose one).
    fn account_clicked(&mut self, cx: &mut Context<Self>) {
        match (&self.account, self.own_handle.clone()) {
            (AccountChip::SignedOut, _) => self.show(Page::SignIn, cx),
            (AccountChip::SignedIn { .. }, Some(handle)) => self.open_person(&handle, None, cx),
            (AccountChip::SignedIn { .. }, None) => self.show(Page::Friends, cx),
            (AccountChip::Hidden | AccountChip::Working, _) => {}
        }
    }

    /// Queues a notice; if the display is empty it shows it at once and arms the timer.
    fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        if self.toasts.push(notice) == Pushed::Shown {
            self.hold_toast(cx);
            cx.notify();
        }
    }

    /// The timer of the notice on the display: `TOAST_HOLD_MS`, then it moves to the next or leaves the
    /// display empty. There is only one at a time, and only while a notice is shown.
    fn hold_toast(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(u64::from(TOAST_HOLD_MS))).await;
            let _ = this.update(cx, |this, cx| {
                if this.toasts.expire() {
                    this.hold_toast(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a page (and puts it in the history). Nothing if it is already that one.
    /// A page that opens closes the immersive view, which would cover it.
    fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.sidebar.overlay {
            // Choosing a page closes the sidebar laid over the canvas.
            self.sidebar.overlay = false;
            cx.notify();
        }
        if self.history.go(page) {
            self.set_focus(false, cx);
            self.page_shown(true, cx);
        }
    }

    /// Ctrl+B and the title bar button: expanded or rail; under `BP_RAIL` the expanded sidebar over the canvas.
    fn toggle_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) {
        if to_dp(window.viewport_size().width) < BP_RAIL {
            self.sidebar.overlay = !self.sidebar.overlay;
        } else {
            self.sidebar.prefs.mode = match self.sidebar.prefs.mode {
                SidebarMode::Expanded => SidebarMode::Rail,
                SidebarMode::Rail => SidebarMode::Expanded,
            };
            self.core
                .read(cx)
                .settings(psytify_core::command::SettingsCmd::SaveUiPrefs(self.sidebar.prefs.prefs()));
        }
        cx.notify();
    }

    /// The right edge is being dragged to `x` (window dp).
    fn drag_sidebar(&mut self, x: f32, cx: &mut Context<Self>) {
        if let Some((start_x, start_w)) = self.sidebar.drag {
            let width = sidebar::clamp_width(start_w + x - start_x);
            if width != self.sidebar.prefs.width {
                self.sidebar.prefs.width = width;
                cx.notify();
            }
        }
    }

    /// The drag ends: the width is saved.
    fn end_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        if self.sidebar.drag.take().is_some() {
            self.core
                .read(cx)
                .settings(psytify_core::command::SettingsCmd::SaveUiPrefs(self.sidebar.prefs.prefs()));
            cx.notify();
        }
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.history.back().is_some() {
            self.set_focus(false, cx);
            self.page_shown(false, cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.history.forward().is_some() {
            self.set_focus(false, cx);
            self.page_shown(false, cx);
        }
    }

    /// The current page changed: the view of that kind takes its object (and asks the core for it
    /// if there is none), and the root redraws.
    /// `rise`: the page enter rises (a new page), not on Back and Forward.
    fn page_shown(&mut self, rise: bool, cx: &mut Context<Self>) {
        let current = self.history.current();
        if self.shown == Page::News && current != Page::News {
            // Leaving «Novità» counts what was there as seen.
            self.core.read(cx).library(LibraryCmd::MarkNewsSeen);
        }
        self.shown = current;
        match current {
            Page::Liked => self.collection_page.update(cx, |page, cx| page.show(Collection::Liked, cx)),
            Page::Playlist(id) => {
                self.collection_page.update(cx, |page, cx| page.show(Collection::Playlist(id), cx));
            }
            Page::Album(id) => self.album_page.update(cx, |page, cx| page.show(id, cx)),
            Page::Home => self.home_page.update(cx, |page, cx| {
                page.show(cx);
                page.enter(rise, cx);
            }),
            Page::News => self.news_page.update(cx, |page, cx| page.show(cx)),
            Page::Artist(id) => self.artist_page.update(cx, |page, cx| page.show(id, cx)),
            Page::Source(key) => {
                if let Some(playlist) = self.sources.get(&key).cloned() {
                    self.source_page.update(cx, |page, cx| page.show(playlist, cx));
                }
            }
            Page::SignIn => self.sign_in_page.update(cx, |page, cx| page.show(cx)),
            Page::Friends => self.friends_page.update(cx, |page, cx| page.show(cx)),
            Page::Profile(key) => {
                if let Some(handle) = self.people.get(&key).cloned() {
                    self.profile_page.update(cx, |page, cx| page.show(&handle, cx));
                }
            }
            Page::FriendPlaylist(key, id) => {
                if let Some(handle) = self.people.get(&key).cloned() {
                    self.collection_page.update(cx, |page, cx| page.show_public(&handle, id, cx));
                }
            }
            Page::Search | Page::Library | Page::Downloads | Page::Settings | Page::Jam => {}
        }
        self.enter_page(current, rise, cx);
        cx.notify();
    }

    /// The page enter (desktop v2 §3) of the view that shows `page` (the Home starts its own above).
    fn enter_page(&mut self, page: Page, rise: bool, cx: &mut Context<Self>) {
        match page {
            Page::Album(_) => self.album_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Liked | Page::Playlist(_) | Page::FriendPlaylist(..) => {
                self.collection_page.update(cx, |page, cx| page.enter(rise, cx));
            }
            Page::Artist(_) => self.artist_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Source(_) => self.source_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::News => self.news_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Search => self.search_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Library => self.library_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Downloads => self.downloads_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Friends => self.friends_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Profile(_) => self.profile_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Jam => self.jam_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Settings => self.settings_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::SignIn => self.sign_in_page.update(cx, |page, cx| page.enter(rise, cx)),
            Page::Home => {}
        }
    }

    /// Opens the page of an external tab; the tabs the history no longer names are dropped.
    fn open_source(&mut self, playlist: SourcePlaylist, cx: &mut Context<Self>) {
        let key = SourceKey::of(playlist.source, &playlist.id);
        self.sources.insert(key, playlist);
        self.show(Page::Source(key), cx);
        if self.sources.len() > SOURCE_PAGES {
            let kept: std::collections::HashSet<SourceKey> = self
                .history
                .pages()
                .filter_map(|page| if let Page::Source(key) = page { Some(key) } else { None })
                .collect();
            self.sources.retain(|key, _| kept.contains(key));
        }
    }

    /// A page asks to open another one or a dialog.
    fn page_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PageEvent::Open(Page::Search) => self.focus_search(window, cx),
            PageEvent::Open(page) => self.show(*page, cx),
            PageEvent::Dialog(request) => self.open_dialog(request.clone(), window, cx),
            PageEvent::TrackMenu(menu) => {
                let library = Arc::clone(&self.core.read(cx).snapshot().library);
                let playlists = playlist_items(&library.playlists);
                let following = library.followed.iter().any(|artist| artist.id == menu.track.artist.id);
                let downloads = DownloadItems::of(&menu.track, &self.core.read(cx).snapshot().downloads);
                self.menu = Some(OpenMenu {
                    menu: (**menu).clone(),
                    level: MenuLevel::Main,
                    playlists,
                    following,
                    downloads,
                });
                cx.notify();
            }
            PageEvent::OpenSource(playlist) => self.open_source((**playlist).clone(), cx),
            PageEvent::SaveSource => {
                // The playlist opens when the core publishes it; if it does not create it (empty, library full)
                // the identifier never appears.
                let id = uuid::Uuid::new_v4();
                self.pending_playlist = Some(id);
                self.core.read(cx).external(ExternalCmd::SavePlaylist { id });
            }
            PageEvent::ToggleSaveAlbum(album) => self.toggle_save_album((**album).clone(), cx),
            PageEvent::ToggleFollow(artist) => self.toggle_follow((**artist).clone(), cx),
            PageEvent::Search(text) => {
                self.search.update(cx, |field, cx| field.set_text(text.clone(), cx));
                self.core.read(cx).catalog(CatalogCmd::Search(text.to_string()));
                self.focus_search(window, cx);
            }
            PageEvent::Notice(notice) => self.notify(notice.clone(), cx),
            PageEvent::OpenProfile(handle) => self.open_person(handle, None, cx),
            PageEvent::OpenAccountSettings => {
                self.show(Page::Settings, cx);
                self.settings_page.update(cx, |page, cx| page.show_account(cx));
            }
            PageEvent::OpenFriendPlaylist { handle, id } => self.open_person(handle, Some(*id), cx),
        }
    }

    /// Saves an album or removes it, with its notice.
    fn toggle_save_album(&mut self, album: SavedAlbum, cx: &mut Context<Self>) {
        let saved =
            self.core.read(cx).snapshot().library.saved_albums.iter().any(|known| known.id == album.id);
        let notice = if saved {
            format!("«{}» tolto dalla libreria", album.title)
        } else {
            format!("«{}» è nella tua libreria", album.title)
        };
        self.core.read(cx).library(LibraryCmd::ToggleSaveAlbum(album));
        self.notify(Notice::info(notice), cx);
    }

    /// Follows an artist or stops, with its notice.
    fn toggle_follow(&mut self, artist: FollowedArtist, cx: &mut Context<Self>) {
        let following =
            self.core.read(cx).snapshot().library.followed.iter().any(|known| known.id == artist.id);
        let notice = if following {
            format!("Non segui più {}", artist.name)
        } else {
            format!("Segui {}: le sue nuove uscite compaiono in Novità", artist.name)
        };
        self.core.read(cx).library(LibraryCmd::ToggleFollow(artist));
        self.notify(Notice::info(notice), cx);
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// An entry of the «Altro» menu: the command to the core, the notice where it is given.
    fn menu_choice(&mut self, choice: MenuChoice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.menu.as_mut() else { return };
        match choice {
            MenuChoice::ShowPlaylists | MenuChoice::ShowQualities | MenuChoice::Back => {
                open.level = match choice {
                    MenuChoice::ShowPlaylists => MenuLevel::Playlists,
                    MenuChoice::ShowQualities => MenuLevel::Qualities,
                    _ => MenuLevel::Main,
                };
                cx.notify();
                return;
            }
            _ => {}
        }
        let Some(OpenMenu { menu, .. }) = self.menu.take() else { return };
        cx.notify();
        let snapshot = self.core.read(cx).snapshot();
        if let Some(notice) = choice_notice(choice, &menu.track, &snapshot.library.playlists) {
            self.notify(notice, cx);
        }
        let core = self.core.read(cx);
        match choice {
            MenuChoice::PlayNext => core.playback(PlaybackCmd::EnqueueNext(Box::new(menu.track))),
            MenuChoice::Enqueue => core.playback(PlaybackCmd::Enqueue(Box::new(menu.track))),
            MenuChoice::AddTo(id) => core.library(LibraryCmd::AddToPlaylist { id, track: menu.track }),
            MenuChoice::RemoveFromPlaylist => {
                if let Some((id, index)) = menu.from_playlist {
                    core.library(LibraryCmd::RemoveFromPlaylist { id, index });
                }
            }
            MenuChoice::NewPlaylist => {
                self.open_dialog(DialogRequest::NewPlaylist { seed: vec![menu.track] }, window, cx)
            }
            MenuChoice::Radio => core.playback(PlaybackCmd::StartRadio(Box::new(menu.track))),
            MenuChoice::GoToArtist => self.show(Page::Artist(menu.track.artist.id), cx),
            MenuChoice::GoToAlbum => {
                if let Some(album) = &menu.track.album {
                    self.show(Page::Album(album.id), cx);
                }
            }
            MenuChoice::ToggleFollow => {
                let artist = FollowedArtist {
                    id: menu.track.artist.id,
                    name: menu.track.artist.name.clone(),
                    cover_url: menu.track.cover_url.clone(),
                };
                self.toggle_follow(artist, cx);
            }
            MenuChoice::Copy => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_text(&menu.track)))
            }
            MenuChoice::Download => core.downloads(DownloadsCmd::Download(Box::new(menu.track))),
            MenuChoice::DownloadAs(quality) => {
                core.downloads(DownloadsCmd::DownloadAs(Box::new(menu.track), quality))
            }
            MenuChoice::RemoveDownload => core.downloads(DownloadsCmd::Remove(menu.track.id)),
            MenuChoice::ShowPlaylists | MenuChoice::ShowQualities | MenuChoice::Back => {}
        }
    }

    /// The panel of the open menu.
    fn menu_panel(&self, open: &OpenMenu, cx: &mut Context<Self>) -> MenuPanel {
        let choose = |id: &'static str, label: &'static str, icon: &'static str, choice: MenuChoice| {
            MenuItem::new(id, label).icon(icon).on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.menu_choice(choice, window, cx)),
            )
        };
        match open.level {
            MenuLevel::Main => {
                let mut panel = MenuPanel::new()
                    .item(choose(
                        "menu-next",
                        "Riproduci subito dopo",
                        sprite::ICON_NEXT,
                        MenuChoice::PlayNext,
                    ))
                    .item(choose("menu-queue", "Aggiungi alla coda", sprite::ICON_QUEUE, MenuChoice::Enqueue))
                    .item(choose("menu-radio", "Avvia radio", sprite::ICON_RADIO, MenuChoice::Radio))
                    .separator()
                    .item(
                        choose(
                            "menu-lists",
                            "Aggiungi a una playlist",
                            sprite::ICON_LIST,
                            MenuChoice::ShowPlaylists,
                        )
                        .submenu(true),
                    );
                if open.menu.from_playlist.is_some() {
                    panel = panel.item(
                        choose(
                            "menu-remove",
                            "Togli da questa playlist",
                            sprite::ICON_TRASH,
                            MenuChoice::RemoveFromPlaylist,
                        )
                        .destructive(true),
                    );
                }
                panel = panel.separator().item(choose(
                    "menu-artist",
                    "Vai all'artista",
                    sprite::ICON_ACCOUNT,
                    MenuChoice::GoToArtist,
                ));
                if open.menu.track.album.is_some() {
                    panel = panel.item(choose(
                        "menu-album",
                        "Vai all'album",
                        sprite::ICON_LIBRARY,
                        MenuChoice::GoToAlbum,
                    ));
                }
                let (follow, follow_icon) = follow_item(open.following);
                panel = panel
                    .item(choose("menu-follow", follow, follow_icon, MenuChoice::ToggleFollow))
                    .item(choose("menu-copy", "Copia titolo e artista", sprite::ICON_COPY, MenuChoice::Copy));
                // Downloads after «Copia».
                if open.downloads.offer {
                    let label = download_label(open.downloads.quality);
                    panel = panel
                        .item(MenuItem::new("menu-download", label).icon(sprite::ICON_DOWNLOAD).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.menu_choice(MenuChoice::Download, window, cx)
                            }),
                        ))
                        .item(
                            choose(
                                "menu-download-as",
                                "Scarica in…",
                                sprite::ICON_DOWNLOAD,
                                MenuChoice::ShowQualities,
                            )
                            .submenu(true),
                        );
                }
                if open.downloads.remove {
                    panel = panel.item(choose(
                        "menu-download-remove",
                        "Rimuovi dai download",
                        sprite::ICON_TRASH,
                        MenuChoice::RemoveDownload,
                    ));
                }
                panel
            }
            MenuLevel::Qualities => {
                let current = open.downloads.quality;
                quality_items().into_iter().enumerate().fold(
                    MenuPanel::new()
                        .item(choose("menu-back", "Indietro", sprite::ICON_CHEVRON_LEFT, MenuChoice::Back))
                        .separator(),
                    |panel, (ix, (quality, label))| {
                        panel.item(
                            MenuItem::new(("menu-quality", ix), label).checked(quality == current).on_click(
                                cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.menu_choice(MenuChoice::DownloadAs(quality), window, cx)
                                }),
                            ),
                        )
                    },
                )
            }
            MenuLevel::Playlists => {
                let lists = open.playlists.iter().enumerate().map(|(ix, (id, name))| {
                    let id = *id;
                    MenuItem::new(("menu-playlist", ix), name.clone()).icon(sprite::ICON_LIST).on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.menu_choice(MenuChoice::AddTo(id), window, cx)
                        }),
                    )
                });
                MenuPanel::new()
                    .item(choose("menu-back", "Indietro", sprite::ICON_CHEVRON_LEFT, MenuChoice::Back))
                    .separator()
                    .item(choose("menu-new", "Nuova playlist…", sprite::ICON_PLUS, MenuChoice::NewPlaylist))
                    .child(
                        div()
                            .id("menu-playlists")
                            .max_h(dp(MENU_LIST_MAX_H))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(lists),
                    )
            }
        }
    }

    fn open_dialog(&mut self, request: DialogRequest, window: &mut Window, cx: &mut Context<Self>) {
        let before = window.focused(cx);
        let core = self.core.clone();
        let dialog = cx.new(|cx| PlaylistDialog::new(core, request, window, cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event: &DialogEvent, window, cx| {
            this.dialog_done(event.clone(), window, cx);
        });
        self.dialog = Some((dialog, before, subscription));
        cx.notify();
    }

    /// The dialog is over: it closes, focus returns to where it was and what the outcome says is done.
    fn dialog_done(&mut self, event: DialogEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, before, _)) = self.dialog.take() {
            let target = before.unwrap_or_else(|| self.focus.clone());
            window.focus(&target, cx);
        }
        cx.notify();
        let DialogEvent::Done { notice, open, deleted } = event else { return };
        if let Some(notice) = notice {
            self.notify(notice, cx);
        }
        if open.is_some() {
            self.pending_playlist = open;
        }
        if let Some(id) = deleted {
            // Leaves the deleted playlist, then removes it from the whole history.
            if self.history.current() == Page::Playlist(id) {
                self.show(Page::Library, cx);
            }
            self.history.forget(|page| page == Page::Playlist(id));
        }
    }

    /// The history keys for the title bar, lit only when there is a step to take.
    fn history_buttons(&self, cx: &mut Context<Self>) -> HistoryButtons {
        let shell = cx.entity().downgrade();
        let back = self.history.can_back().then(|| {
            let shell = shell.clone();
            std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                shell.update(cx, |shell, cx| shell.go_back(cx)).ok();
            }) as OnClose
        });
        let forward = self.history.can_forward().then(|| {
            std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                shell.update(cx, |shell, cx| shell.go_forward(cx)).ok();
            }) as OnClose
        });
        HistoryButtons { on_back: back, on_forward: forward }
    }

    /// The queue (Q), lyrics (T) or now-playing (the bar cover) button: the panel on that tab, or shut.
    fn toggle_panel(&mut self, kind: SidePanel, cx: &mut Context<Self>) {
        self.panel = toggled(self.panel, kind);
        if self.panel.is_some() && self.stage_open {
            // A panel button under the stage leaves it for the panel.
            self.set_focus(false, cx);
        }
        self.sync_panels(cx);
        cx.notify();
    }

    /// A tab of the open panel.
    fn select_tab(&mut self, tab: SidePanel, cx: &mut Context<Self>) {
        if self.panel != Some(tab) {
            self.panel = Some(tab);
            self.sync_panels(cx);
            cx.notify();
        }
    }

    /// F: opens the stage if a track is queued, closes it if open.
    fn toggle_focus(&mut self, cx: &mut Context<Self>) {
        let open = !self.stage_open && self.core.read(cx).snapshot().playback.queue.current().is_some();
        self.set_focus(open, cx);
    }

    fn set_focus(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.stage_open != open {
            self.stage_open = open;
            let ms = crate::motion::duration(MOTION_PANEL, cx);
            self.stage_t.set(if open { 1.0 } else { 0.0 }, ms, std::time::Instant::now());
            self.sync_panels(cx);
            cx.notify();
        }
    }

    /// The playing track has a clip.
    fn has_video(&self, cx: &App) -> bool {
        self.core.read(cx).snapshot().playback.queue.current().is_some_and(|track| track.has_video)
    }

    /// The bar's «Videoclip» and V: off if the clip is on screen; else on, in the stage's slot when the stage
    /// is open, or in the panel's (opened on *In riproduzione* if it is not there). A clip that failed retries.
    #[cfg(feature = "video")]
    fn toggle_video(&mut self, cx: &mut Context<Self>) {
        if !self.has_video(cx) {
            return;
        }
        let shown = video_surface(self.panel, self.stage_open, self.video_on).is_some();
        if shown {
            self.set_video_on(false, cx);
            return;
        }
        if !self.stage_open && self.panel != Some(SidePanel::Now) {
            self.panel = Some(SidePanel::Now);
        }
        self.set_video_on(true, cx);
        cx.notify();
    }

    /// The Cover / Video switch of a media slot.
    fn set_video_on(&mut self, on: bool, cx: &mut Context<Self>) {
        #[cfg(feature = "video")]
        if on && matches!(self.video.read(cx).status(), crate::video::stage::Status::Failed) {
            self.video.read(cx).retry(cx);
        }
        if self.video_on != on {
            self.video_on = on;
            self.sync_panels(cx);
        }
    }

    /// Tells every view whether it shows (only then it has its timers) and the core whether anyone shows the
    /// lyrics.
    fn sync_panels(&mut self, cx: &mut Context<Self>) {
        let (panel, stage) = (self.panel, self.stage_open);
        if let Some(tab) = panel {
            self.panel_last = tab;
        }
        self.bar.update(cx, |bar, cx| {
            bar.set_panel(panel, cx);
            bar.set_hidden(stage, cx);
        });
        let shown = |tab| panel == Some(tab) && !stage;
        self.lyrics.update(cx, |lyrics, cx| lyrics.set_visible(shown(SidePanel::Lyrics), cx));
        let now_shown = shown(SidePanel::Now);
        let video_on = self.video_on && self.has_video(cx);
        self.now_panel.update(cx, |view, cx| {
            view.set_visible(now_shown, cx);
            view.set_video(video_on, cx);
        });
        self.stage_view.update(cx, |view, cx| {
            view.set_open(stage, cx);
            view.set_video(video_on, cx);
        });
        #[cfg(feature = "video")]
        {
            let surface = video_surface(panel, stage, video_on);
            self.video.update(cx, |video, cx| video.set_surface(surface, cx));
            self.bar.update(cx, |bar, cx| bar.set_video_shown(surface.is_some(), cx));
        }
        let wanted = shown(SidePanel::Lyrics) || now_shown || stage;
        if wanted != self.lyrics_wanted {
            self.lyrics_wanted = wanted;
            self.core.read(cx).playback(PlaybackCmd::Lyrics(wanted));
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_focus(false, cx);
        self.show(Page::Search, cx);
        let handle = self.search.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Closes the window after the core has written `config.json`. It waits for the save up to
    /// [`SHUTDOWN_GRACE`]: beyond that, it quits anyway (the configuration stays that of the last save).
    fn request_close(&mut self, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        let done = self.core.update(cx, |core, _| core.begin_shutdown());
        cx.spawn(async move |_, cx| {
            if let Some(done) = done {
                // The outcome changes nothing: if the save failed, the core has already said so in the log.
                let _ = done.await;
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
        // Safety net: a core that does not answer must not keep the window open.
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SHUTDOWN_GRACE).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    /// Sets the zoom the preference asks for this window width; a change lays everything out again (the
    /// cached views too, through `Window::refresh`).
    fn apply_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let width = f32::from(window.viewport_size().width);
        if crate::theme::set_zoom(self.zoom_pref.effective(width, crate::theme::zoom())) {
            window.refresh();
            cx.notify();
        }
    }

    /// Ctrl+= / Ctrl+- (`Some(true)` / `Some(false)`) pin the next preset, Ctrl+0 (`None`) goes back to `Auto`.
    fn step_zoom(&mut self, up: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let pref = match up {
            Some(up) => ZoomPref::stepped(crate::theme::zoom(), up),
            None => ZoomPref::Auto,
        };
        if pref != self.zoom_pref {
            self.zoom_pref = pref;
            self.core.read(cx).settings(psytify_core::command::SettingsCmd::SaveUiPrefs(pref.prefs()));
            self.apply_zoom(window, cx);
        }
    }

    /// The handler of the click on the account in the title bar.
    fn account_handler(&self, cx: &mut Context<Self>) -> OnClose {
        let shell = cx.entity().downgrade();
        std::rc::Rc::new(move |_, cx| {
            shell.update(cx, |shell, cx| shell.account_clicked(cx)).ok();
        })
    }

    /// What the title bar or the sidebar shows (`chrome.rs`).
    fn chrome_key(&self, part: Part) -> ChromeKey {
        match part {
            Part::TitleBar => ChromeKey::TitleBar {
                playing: self.playing,
                back: self.history.can_back(),
                forward: self.history.can_forward(),
                account: self.account.clone(),
            },
            Part::Sidebar => ChromeKey::Sidebar {
                current: self.history.current(),
                counts: [self.busy_downloads, self.unread_news, self.incoming, self.invites],
                rail: self.sidebar_rail,
                rev: self.sidebar.rev,
                tab: self.sidebar.tab,
            },
        }
    }

    /// `Shell` notified itself: the title bar and the sidebar redraw if what they show changes
    /// (the frame of a sprite painted by `Shell` does not redo them).
    fn sync_chrome(&mut self, cx: &mut Context<Self>) {
        let keys = (self.chrome_key(Part::TitleBar), self.chrome_key(Part::Sidebar));
        let Some(before) = self.chrome_keys.replace(keys.clone()) else { return };
        if before.0 != keys.0 {
            self.title_view.update(cx, |_, cx| cx.notify());
        }
        if before.1 != keys.1 {
            self.sidebar_view.update(cx, |_, cx| cx.notify());
        }
    }

    /// The elements of the title bar or the sidebar, for their cached view (`Chrome`).
    fn chrome(&mut self, part: Part, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        match part {
            Part::TitleBar => {
                let state = TitleBarState {
                    search: self.search.clone(),
                    playing: self.playing,
                    history: self.history_buttons(cx),
                    account: self.account.clone(),
                    on_account: Some(self.account_handler(cx)),
                    on_close: Self::close_handler(&cx.entity()),
                    on_sidebar: Some({
                        let shell = cx.entity().downgrade();
                        std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                            shell.update(cx, |shell, cx| shell.toggle_sidebar(window, cx)).ok();
                        })
                    }),
                };
                title_row(state, window, cx).into_any_element()
            }
            Part::Sidebar => {
                let rail = self.sidebar_rail;
                self.sidebar(rail, window, cx)
            }
        }
    }

    /// The notice on the display, above everything (also the video).
    fn toast(&self) -> Option<gpui::Div> {
        let notice = self.toasts.current()?;
        Some(
            div()
                .absolute()
                .bottom(dp(PLAYER_BAR_H + SPACE_5))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(Toast::new(notice.text.clone()).error(notice.error)),
        )
    }

    /// The handler to give to the title bar and to the system to close the window.
    pub fn close_handler(this: &Entity<Self>) -> OnClose {
        let shell = this.downgrade();
        std::rc::Rc::new(move |_, cx| {
            shell.update(cx, |shell, cx| shell.request_close(cx)).ok();
        })
    }

    /// The sidebar, the page and the open side panel (beside the page if there is room,
    /// otherwise over it).
    fn body(
        &self,
        layout: SidebarLayout,
        width: f32,
        docked: bool,
        panel_w: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // Under the overlay the row keeps a plain rail-wide strip: the view draws once, over the canvas.
        let slot = if layout.overlay.is_some() {
            div().flex_none().h_full().w(dp(width)).bg(col(SURFACE_CHROME)).into_any_element()
        } else {
            self.sidebar_view.clone().cached(sidebar_style(width)).into_any_element()
        };
        let mut body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .child(slot)
            .child(Separator::vertical())
            // The canvas fills the width between the sidebar and the side panel (no column cap).
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.page_content())
                    .children(self.page_spot().map(spot_overlay)),
            );
        if panel_w > 0.0 {
            body = if docked {
                body.child(self.side_panel(panel_w, cx))
            } else {
                body.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .occlude()
                        .child(self.side_panel(panel_w, cx)),
                )
            };
        }
        if layout.resizable {
            // The right edge drags the width (`SIDEBAR_MIN_W` to `SIDEBAR_MAX_W`), saved on release.
            body = body.child(
                div()
                    .id("sidebar-grip")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(dp(width - SIDEBAR_GRIP_W / 2.0))
                    .w(dp(SIDEBAR_GRIP_W))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.sidebar.drag = Some((to_dp(event.position.x), this.sidebar.prefs.width));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        if let Some(over) = layout.overlay {
            // A click beside the laid-over sidebar closes it.
            body = body
                .child(
                    div()
                        .id("sidebar-veil")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .left(dp(over))
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.sidebar.overlay = false;
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(dp(over))
                        .occlude()
                        .child(hard_shadow(0, 2, col(SHADOW_HARD)))
                        .child(self.sidebar_view.clone().cached(fill())),
                );
        }
        body
    }

    /// The side panel `width` dp wide (less than `SIDE_PANEL_W` while it slides): the tabs, then the tab's
    /// view at full width, clipped by the sliding edge so the cached view keeps its size.
    fn side_panel(&self, width: f32, cx: &mut Context<Self>) -> gpui::Div {
        let tab = self.panel.unwrap_or(self.panel_last);
        let content = match tab {
            SidePanel::Queue => div()
                .relative()
                .size_full()
                .child(self.queue.clone().cached(fill()))
                .child(spot_overlay(self.spots.queue.clone())),
            SidePanel::Lyrics => div().size_full().child(self.lyrics.clone().cached(fill())),
            SidePanel::Now => div().size_full().child(self.now_panel.clone().cached(fill())),
        };
        let shell = cx.entity().downgrade();
        let tabs = Tabs::new(
            SidePanel::ALL
                .iter()
                .map(|tab| crate::components::navigation::Tab::new(("panel-tab", tab.ix()), tab.label()))
                .collect(),
            tab.ix(),
        )
        .on_select(move |ix, _, cx| {
            let tab = SidePanel::ALL.get(ix).copied().unwrap_or(SidePanel::Now);
            shell.update(cx, |this, cx| this.select_tab(tab, cx)).ok();
        });
        let inner = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .w(dp(SIDE_PANEL_W))
            .flex()
            .bg(col(SURFACE_CHROME))
            .child(Separator::vertical().strong())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_none().px(dp(SPACE_3)).pt(dp(SPACE_3)).child(tabs))
                    .child(div().flex_1().min_h_0().child(content)),
            );
        div().relative().flex_none().h_full().w(dp(width)).overflow_hidden().child(inner)
    }

    /// The equaliser spot of the current page, if it is a page with track rows.
    fn page_spot(&self) -> Option<PlayingSpot> {
        let spot = match self.history.current() {
            Page::Home => &self.spots.home,
            Page::Search => &self.spots.search,
            Page::Downloads => &self.spots.downloads,
            Page::Liked | Page::Playlist(_) | Page::FriendPlaylist(..) => &self.spots.collection,
            Page::Profile(_) => &self.spots.profile,
            Page::Album(_) => &self.spots.album,
            Page::Artist(_) => &self.spots.artist,
            Page::Library
            | Page::News
            | Page::Source(_)
            | Page::Settings
            | Page::SignIn
            | Page::Friends
            | Page::Jam => {
                return None;
            }
        };
        Some(spot.clone())
    }

    fn page_content(&self) -> gpui::AnyElement {
        match self.history.current() {
            Page::Home => self.home_page.clone().cached(fill()).into_any_element(),
            Page::News => self.news_page.clone().cached(fill()).into_any_element(),
            Page::Search => self.search_page.clone().cached(fill()).into_any_element(),
            Page::Downloads => self.downloads_page.clone().cached(fill()).into_any_element(),
            Page::Library => self.library_page.clone().cached(fill()).into_any_element(),
            Page::Liked | Page::Playlist(_) | Page::FriendPlaylist(..) => {
                self.collection_page.clone().cached(fill()).into_any_element()
            }
            Page::Album(_) => self.album_page.clone().cached(fill()).into_any_element(),
            Page::Artist(_) => self.artist_page.clone().cached(fill()).into_any_element(),
            Page::Source(_) => self.source_page.clone().cached(fill()).into_any_element(),
            Page::Settings => self.settings_page.clone().cached(fill()).into_any_element(),
            Page::SignIn => self.sign_in_page.clone().cached(fill()).into_any_element(),
            Page::Friends => self.friends_page.clone().cached(fill()).into_any_element(),
            Page::Profile(_) => self.profile_page.clone().cached(fill()).into_any_element(),
            Page::Jam => self.jam_page.clone().cached(fill()).into_any_element(),
        }
    }
}

/// The account as the title bar shows it.
fn account_chip(account: &Account) -> AccountChip {
    match account {
        Account::Unavailable => AccountChip::Hidden,
        Account::Working => AccountChip::Working,
        Account::SignedOut => AccountChip::SignedOut,
        Account::SignedIn(me) => {
            let profile = me.profile.as_ref();
            AccountChip::SignedIn {
                avatar: profile.and_then(|p| p.avatar_url.clone()).map(Into::into),
                name: profile.map_or_else(|| "Il tuo account".into(), |p| p.display_name.clone().into()),
            }
        }
    }
}

/// One's own username, if one has been chosen.
fn own_handle(social: &SocialView) -> Option<String> {
    social.account.profile().map(|profile| profile.handle.clone())
}

/// Incoming friend requests.
fn incoming(social: &SocialView) -> usize {
    social.friends.ready().map_or(0, |friends| friends.incoming.len())
}

/// The style with which a full-area view enters `.cached()`: GPUI's cache does not measure the children,
/// it gives the view the container's size. The view redraws only when it notifies itself, when
/// its size changes or when the window redraws entirely (`Window::refresh`: resize,
/// arrival of a cover, mouse over the edges); sprite frames invalidate only the view
/// that painted them (`pixel::anim`).
fn fill() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

/// The «in riproduzione» bar in `.cached()`: as wide as the window, `PLAYER_BAR_H` high.
fn bar_style() -> StyleRefinement {
    StyleRefinement::default().w_full().h(dp(PLAYER_BAR_H)).flex_none()
}

/// The title bar row in `.cached()`: as wide as the window, `TITLE_BAR_H` high.
fn title_style() -> StyleRefinement {
    StyleRefinement::default().w_full().h(dp(TITLE_BAR_H)).flex_none()
}

/// The sidebar in `.cached()`, `width` dp wide (it eases between the rail and the expanded width).
fn sidebar_style(width: f32) -> StyleRefinement {
    StyleRefinement::default().flex_none().h_full().w(dp(width))
}

/// The update banner in `.cached()`: as wide as the window, `BANNER_H` high.
fn banner_style() -> StyleRefinement {
    StyleRefinement::default().w_full().h(dp(BANNER_H)).flex_none()
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::theme::note_scale(window.scale_factor());
        let width = to_dp(window.viewport_size().width);
        let layout = self.sidebar.layout(width);
        if layout.rail != self.sidebar_rail {
            self.sidebar_rail = layout.rail;
            self.sidebar_view.update(cx, |_, cx| cx.notify());
        }
        let (side_w, moving) = self.sidebar.width(layout.flow, crate::motion::duration(MOTION_PANEL, cx));
        crate::motion::keep_running(window, moving);
        // The side panel slides its width in `MOTION_PANEL`, on whole APs.
        let panel_target = if self.panel.is_some() { SIDE_PANEL_W } else { 0.0 };
        let panel_w = crate::motion::follow("side-panel-width", panel_target, MOTION_PANEL, window, cx);
        let ap_dp = to_dp(Ap::of(window).one());
        let panel_w = if ap_dp > 0.0 { ap_round(panel_w / ap_dp) as f32 * ap_dp } else { panel_w };
        let panel_w = panel_w.min(SIDE_PANEL_W);
        // The stage reveal: while it moves the stage lies over page and bar, uncached.
        let now = std::time::Instant::now();
        let stage_moving = self.stage_t.running(now);
        let reveal = self.stage_t.value(now);
        self.stage_reveal.set(reveal);
        crate::motion::keep_running(window, stage_moving);
        if self.stage_moving && !stage_moving {
            // Landed: the cached stage view redraws once with the final reveal.
            self.stage_view.update(cx, |_, cx| cx.notify());
        }
        self.stage_moving = stage_moving;
        let stage_shown = self.stage_open && !stage_moving;
        // The panel sits beside the page if there is room, otherwise it floats over it.
        let docked = width >= BP_PANEL_DOCK;
        // In cache like the pages: they redraw only when what they show changes (`sync_chrome`), not at every
        // frame of the equaliser that `Shell` paints over the pages (BACKLOG 23).
        let bar = self.title_view.clone().cached(title_style());

        let mut root = div()
            .id("shell")
            .key_context(SHELL_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrev, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &Quit, _, cx| this.request_close(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleQueueAction, _, cx| this.toggle_panel(SidePanel::Queue, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleLyricsAction, _, cx| this.toggle_panel(SidePanel::Lyrics, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFocusAction, _, cx| this.toggle_focus(cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.show(Page::Settings, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.step_zoom(Some(true), window, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.step_zoom(Some(false), window, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| this.step_zoom(None, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.menu.is_some() {
                    this.close_menu(cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.stage_open && this.dialog.is_none() {
                    this.set_focus(false, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            // The mouse side buttons, as in a browser.
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.go_back(cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, _, cx| this.go_forward(cx)),
            )
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| {
                this.core.read(cx).playback(psytify_core::command::PlaybackCmd::Toggle);
            }))
            .on_action(cx.listener(|this, _: &NextTrack, _, cx| {
                this.core.read(cx).playback(psytify_core::command::PlaybackCmd::Next);
            }))
            .on_action(cx.listener(|this, _: &PreviousTrack, _, cx| {
                this.core.read(cx).playback(psytify_core::command::PlaybackCmd::Previous);
            }))
            .on_action(cx.listener(|this, _: &ToggleShuffle, _, cx| {
                this.core.read(cx).playback(psytify_core::command::PlaybackCmd::ToggleShuffle);
            }))
            .on_action(cx.listener(|this, _: &CycleRepeat, _, cx| {
                this.core.read(cx).playback(psytify_core::command::PlaybackCmd::CycleRepeat);
            }))
            .child(bar)
            .child(Separator::horizontal());
        #[cfg(feature = "video")]
        {
            root = root.on_action(cx.listener(|this, _: &ToggleVideoFocus, _, cx| this.toggle_video(cx)));
        }
        if self.banner_shown && !stage_shown {
            // Cached at its height: the download loader redraws only the banner.
            root = root.child(self.banner.clone().cached(banner_style())).child(Separator::horizontal());
        }
        root = if stage_shown {
            // The stage takes everything under the title bar: no page, panel or bar.
            root.child(div().flex_1().min_h_0().child(self.stage_view.clone().cached(fill())))
        } else {
            let mut below = div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.body(layout, side_w, docked, panel_w, cx))
                .child(Separator::horizontal())
                // Cached like the pages: redrawn when it notifies itself (its 250 ms tick while playing, a track
                // or state change), not at every `Shell` draw (BACKLOG 23).
                .child(self.bar.clone().cached(bar_style()));
            if stage_moving {
                below = below.child(
                    div().absolute().top_0().left_0().size_full().occlude().child(self.stage_view.clone()),
                );
            }
            root.child(below)
        };
        // The video stage after the page, panels and immersive view: it paints the frame in the well that the
        // panel or the immersive view marked in this same draw (`VideoStage::well`). A
        // frame redoes `Shell` (the root: GPUI redoes it on every draw) and the stage; the cached views, the
        // panel and the immersive view included, are reused (BACKLOG 41).
        if self.sidebar.drag.is_some() {
            // Window-wide listeners only while the edge is dragged.
            let shell = cx.entity().downgrade();
            root = root.child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        let moved = shell.clone();
                        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, _, _, cx| {
                            moved.update(cx, |this, cx| this.drag_sidebar(to_dp(event.position.x), cx)).ok();
                        });
                        let released = shell.clone();
                        window.on_mouse_event(move |_: &gpui::MouseUpEvent, _, _, cx| {
                            released.update(cx, |this, cx| this.end_sidebar_drag(cx)).ok();
                        });
                    },
                )
                .absolute()
                .size_0(),
            );
        }
        #[cfg(feature = "video")]
        {
            root = root.child(self.video.clone());
        }

        if let Some(open) = &self.menu {
            // A click outside the menu closes it (and does not reach what is underneath).
            let close = |this: &mut Self,
                         _: &gpui::MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<Self>| { this.close_menu(cx) };
            root = root.child(deferred(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(close))
                    .on_mouse_down(MouseButton::Right, cx.listener(close)),
            ));
            // The panel covers the scrim: a click on an entry does not reach the scrim (which closes).
            let panel = div().occlude().child(self.menu_panel(open, cx));
            root = root.child(popup(open.menu.at, panel));
        }
        if let Some((dialog, ..)) = &self.dialog {
            // The scrim covers the whole window; a click on the scrim closes it, like «Annulla».
            let veil = scrim().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.dialog_done(DialogEvent::Cancelled, window, cx)),
            );
            root = root.child(deferred(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(veil)
                    .child(dialog.clone()),
            ));
        }
        window_frame(root.children(self.toast()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_bar_shows_the_account_as_the_core_says() {
        use psytify_core::api::Me;
        use psytify_core::api::social::MyProfile;
        assert_eq!(account_chip(&Account::Unavailable), AccountChip::Hidden);
        assert_eq!(account_chip(&Account::Working), AccountChip::Working);
        assert_eq!(account_chip(&Account::SignedOut), AccountChip::SignedOut);
        let mut me = Me { user_id: "u".to_owned(), admin: false, profile: None };
        assert_eq!(
            account_chip(&Account::SignedIn(me.clone())),
            AccountChip::SignedIn { avatar: None, name: "Il tuo account".into() }
        );
        me.profile = Some(MyProfile {
            handle: "psy".to_owned(),
            display_name: "Psy".to_owned(),
            avatar_url: None,
            share_activity: true,
        });
        let social = SocialView { account: Account::SignedIn(me.clone()), ..SocialView::default() };
        assert_eq!(own_handle(&social).as_deref(), Some("psy"));
        assert_eq!(account_chip(&social.account), AccountChip::SignedIn { avatar: None, name: "Psy".into() });
        assert_eq!(incoming(&social), 0);
    }

    #[cfg(feature = "video")]
    #[test]
    fn the_clip_shows_in_one_media_slot_at_a_time() {
        let now = Some(SidePanel::Now);
        assert_eq!(video_surface(now, false, true), Some(Surface::Panel));
        assert_eq!(video_surface(now, false, false), None, "the clip is off: the cover");
        assert_eq!(video_surface(Some(SidePanel::Queue), false, true), None, "another tab hides the slot");
        assert_eq!(video_surface(None, false, true), None);
        // The stage covers the panel: the clip moves to the stage's slot.
        assert_eq!(video_surface(now, true, true), Some(Surface::Stage));
        assert_eq!(video_surface(None, true, true), Some(Surface::Stage));
        assert_eq!(video_surface(None, true, false), None);
    }

    #[test]
    fn the_panel_tabs_are_in_order_and_toggle() {
        assert_eq!(SidePanel::ALL.map(SidePanel::label), ["In riproduzione", "Testo", "Coda"]);
        assert_eq!(SidePanel::Queue.ix(), 2);
        assert_eq!(toggled(Some(SidePanel::Now), SidePanel::Now), None);
        assert_eq!(toggled(Some(SidePanel::Now), SidePanel::Lyrics), Some(SidePanel::Lyrics));
    }

    #[test]
    fn the_queue_and_the_lyrics_share_the_panel() {
        // A button opens its tab, or closes the panel if that tab was open.
        assert_eq!(toggled(None, SidePanel::Lyrics), Some(SidePanel::Lyrics));
        assert_eq!(toggled(Some(SidePanel::Queue), SidePanel::Lyrics), Some(SidePanel::Lyrics));
        assert_eq!(toggled(Some(SidePanel::Lyrics), SidePanel::Lyrics), None);
        assert_eq!(toggled(Some(SidePanel::Lyrics), SidePanel::Queue), Some(SidePanel::Queue));
    }
}
