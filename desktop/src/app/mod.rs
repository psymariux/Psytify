//! L'app desktop su GPUI: la finestra con barra del titolo lato client, barra laterale, pagina,
//! pannello laterale (la coda o il testo) e barra «in riproduzione», o al loro posto la vista immersiva
//! (`focus_view`). Ogni pezzo è una vista (entità) che si abbona al
//! `CoreModel` e si ridisegna solo quando cambia il suo dominio; la radice tiene la pagina corrente,
//! il pannello aperto, gli avvisi (un solo sistema per gli eventi di tutti i domini, vedi `notices`) e la
//! chiusura.
//!
//! Pagine (`nav::Page`, con la cronologia indietro/avanti di `nav::History`): Cerca, Libreria, «Brani che
//! ti piacciono» e una playlist (`collection_page`), Scaricati, un album (`album_page`), un artista
//! (`artist_page`), un album o una playlist del catalogo ufficiale o di un'altra fonte (`source_page`),
//! la Home (`home_page`, la pagina d'avvio come nel client Iced), l'accesso e gli Amici con i profili, e la
//! Jam (`jam_page`, l'ascolto insieme). Sopra tutto `Shell` tiene il dialogo delle playlist
//! (`playlist_dialog`, anche la conferma per togliere un amico) e il menu «Altro» di un brano
//! (`track_menu`), che le pagine chiedono con un `PageEvent`. Tutto ciò che si può fare passa da un
//! comando al core: la finestra non tiene copie dello stato, solo gli `Arc` delle viste e i testi
//! già pronti.

mod album_page;
mod artist_page;
mod chrome;
mod collection_page;
mod downloads_page;
mod focus_view;
mod friends_page;
mod home_page;
mod jam_page;
mod library_page;
mod lyrics_view;
mod nav;
mod news_page;
mod page_list;
mod people;
mod player_bar;
mod playlist_dialog;
mod profile_page;
mod queue_panel;
mod search_page;
pub mod settings_page;
mod sign_in_page;
mod source_page;
mod track_list;
mod track_menu;
mod update_banner;
#[cfg(feature = "video")]
mod video_panel;

pub use nav::{Page, PageArg};

use std::collections::HashMap;
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
use psytify_design::tokens::color::SURFACE_CANVAS;
use psytify_design::tokens::color::SURFACE_CHROME;
use psytify_design::tokens::motion::TOAST_HOLD_MS;
use psytify_design::tokens::size::{
    BP_PANEL_DOCK, BP_RAIL, CONTENT_MAX_W, PLAYER_BAR_H, SIDE_PANEL_W, SIDEBAR_RAIL_W, SIDEBAR_W, TITLE_BAR_H,
};
use psytify_design::tokens::space::{SPACE_3, SPACE_5};

use crate::actions::{
    CycleRepeat, FocusNext, FocusPrev, FocusSearch, GoBack, GoForward, NextTrack, OpenSettings,
    PreviousTrack, Quit, SHELL_CONTEXT, ToggleFocus as ToggleFocusAction, ToggleLyrics as ToggleLyricsAction,
    TogglePlay, ToggleQueue as ToggleQueueAction, ToggleShuffle,
};
use crate::components::indicators::PlayState;
use crate::components::menu::{MenuItem, MenuPanel, popup};
use crate::components::navigation::NavigationItem;
use crate::components::overlay::{Toast, scrim};
use crate::components::playing_spot::{PlayingSpot, overlay as spot_overlay};
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::notices::{Notice, Pushed, Seen, Toasts};
use crate::theme::{col, dp};
use crate::titlebar::{AccountChip, HistoryButtons, OnClose, TitleBarState, title_row, window_frame};
use album_page::AlbumPage;
use artist_page::ArtistPage;
use chrome::{Chrome, Key as ChromeKey, Part};
use collection_page::{Collection, CollectionPage};
use downloads_page::DownloadsPage;
use focus_view::{FocusEvent, FocusView};
use friends_page::FriendsPage;
use home_page::HomePage;
use jam_page::JamPage;
use library_page::LibraryPage;
use lyrics_view::{Look, LyricsView};
use nav::{History, Nav, PersonKey, SourceKey};
use news_page::NewsPage;
use player_bar::{BarEvent, PlayerBar};
use playlist_dialog::{DialogEvent, DialogRequest, PlaylistDialog};
use profile_page::ProfilePage;
use queue_panel::QueuePanel;
use search_page::SearchPage;
use settings_page::SettingsPage;
use sign_in_page::SignInPage;
use source_page::SourcePage;
use track_menu::{
    DownloadItems, MenuChoice, MenuLevel, TrackMenu, choice_notice, copy_text, download_label, follow_item,
    playlist_items, quality_items,
};
use update_banner::{BANNER_H, Place, UpdateBanner, update_line};
#[cfg(feature = "video")]
use video_panel::{PanelEvent, VideoPanel};

#[cfg(feature = "video")]
use crate::actions::ToggleVideoFocus;
#[cfg(feature = "video")]
use crate::video::stage::{StageEvent, Surface, VideoStage};

/// Quanto si aspetta il salvataggio del core prima di uscire comunque.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Il pannello laterale: la coda, il testo o (con la feature `video`) il videoclip, mai insieme (come le
/// schede del pannello del client Iced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    Queue,
    Lyrics,
    #[cfg(feature = "video")]
    Video,
}

/// Dove si vede il videoclip: nella vista immersiva aperta sul videoclip, o nel pannello sul videoclip
/// quando la vista immersiva non lo copre. Puro.
#[cfg(feature = "video")]
pub fn video_surface(panel: Option<SidePanel>, focus_open: bool, focus_video: bool) -> Option<Surface> {
    if focus_open {
        focus_video.then_some(Surface::Focus)
    } else {
        (panel == Some(SidePanel::Video)).then_some(Surface::Panel)
    }
}

/// Il pannello dopo il tasto di `kind`: si apre su quella scheda, o si chiude se era già lei (come
/// `Message::OpenPanel` del client Iced).
pub fn toggled(open: Option<SidePanel>, kind: SidePanel) -> Option<SidePanel> {
    if open == Some(kind) { None } else { Some(kind) }
}

/// Come mostrare il brano in corso nelle liste: lime e animato solo se suona.
pub fn play_state(view: &PlaybackView) -> Option<PlayState> {
    match view.state {
        PlaybackState::Playing => Some(PlayState::Playing),
        PlaybackState::Paused => Some(PlayState::Paused),
        PlaybackState::Loading { .. } => Some(PlayState::Buffering),
        PlaybackState::Stopped | PlaybackState::Error(_) => None,
    }
}

/// Ciò che una pagina chiede a `Shell`: aprire un'altra pagina o un dialogo (che sta sopra tutta la
/// finestra, non dentro la pagina), e i gesti che danno un avviso (gli avvisi li fa solo `Shell`).
#[derive(Clone, Debug, PartialEq)]
pub enum PageEvent {
    Open(Page),
    Dialog(DialogRequest),
    /// Il menu «Altro» di un brano.
    TrackMenu(Box<TrackMenu>),
    /// Apre la pagina di un album o di una playlist di una fonte esterna.
    OpenSource(Box<SourcePlaylist>),
    /// Salva nella libreria la playlist esterna aperta (la si apre quando il core la pubblica).
    SaveSource,
    /// Salva un album nella libreria o lo toglie.
    ToggleSaveAlbum(Box<SavedAlbum>),
    /// Segue un artista o smette.
    ToggleFollow(Box<FollowedArtist>),
    /// Apre le impostazioni sulla scheda Account (il modulo del profilo).
    OpenAccountSettings,
    /// Apre il profilo di una persona (il suo nome utente).
    OpenProfile(String),
    /// Apre la playlist pubblica `id` di una persona.
    OpenFriendPlaylist {
        handle: String,
        id: uuid::Uuid,
    },
    /// Cerca questo testo (lo mette nel campo e apre Cerca).
    Search(gpui::SharedString),
    /// Un avviso (gli avvisi li mostra solo `Shell`).
    Notice(Notice),
}

/// I posti dell'equalizzatore di ogni vista che lo mostra (vedi `components::playing_spot`).
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

/// Le schede delle pagine esterne che la cronologia può ancora riaprire: oltre, si scartano quelle che
/// nessuna pagina della cronologia nomina.
const SOURCE_PAGES: usize = 64;

/// Il menu «Altro» aperto.
struct OpenMenu {
    menu: TrackMenu,
    level: MenuLevel,
    playlists: Vec<(uuid::Uuid, gpui::SharedString)>,
    /// Segui già l'artista del brano (all'apertura del menu).
    following: bool,
    /// Le voci di download (all'apertura del menu).
    downloads: DownloadItems,
}

/// L'elenco delle playlist nel menu scorre oltre dieci voci.
const MENU_LIST_MAX_H: f32 = 10.0 * psytify_design::tokens::size::MENU_ITEM_H;

/// Come aprire l'app (dalla riga di comando): una ricerca già lanciata e il pannello della coda aperto.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    pub search: Option<String>,
    pub queue: bool,
    /// Il pannello laterale aperto sul testo (`--lyrics`).
    pub lyrics: bool,
    /// La vista immersiva aperta (`--focus`), se c'è un brano in coda.
    pub focus: bool,
    /// Suona il primo risultato del catalogo ufficiale appena arriva (per provare l'importazione senza
    /// toccare niente: la riga mostra «Apro…» finché il server lavora).
    pub play_first_official: bool,
    /// La pagina da aprire (`--page`): per le schermate e le misure, dove non si tocca niente.
    pub page: Option<PageArg>,
    /// I dati sociali di prova (`--fixture`, solo con la feature `fixtures`).
    #[cfg(feature = "fixtures")]
    pub fixture: Option<String>,
    /// Il pannello sul videoclip (`--video`); con `--focus`, la vista immersiva sul videoclip.
    #[cfg(feature = "video")]
    pub video: bool,
}

pub struct Shell {
    core: Entity<CoreModel>,
    focus: FocusHandle,
    search: Entity<SearchField>,
    bar: Entity<PlayerBar>,
    /// La fascia dell'aggiornamento sotto la barra del titolo, e se c'è (solo allora entra nella finestra).
    banner: Entity<UpdateBanner>,
    banner_shown: bool,
    queue: Entity<QueuePanel>,
    /// Il testo nel pannello laterale.
    lyrics: Entity<LyricsView>,
    /// La vista immersiva (con il suo testo): al posto di pagina, pannello e barra quando è aperta.
    focus_view: Entity<FocusView>,
    focus_open: bool,
    /// Il palco del videoclip (uno per finestra), il pannello che lo mostra e la faccia della vista immersiva
    /// (videoclip o testo).
    #[cfg(feature = "video")]
    video: Entity<VideoStage>,
    #[cfg(feature = "video")]
    video_panel: Entity<VideoPanel>,
    #[cfg(feature = "video")]
    focus_video: bool,
    /// Il core sa che qualcuno mostra il testo (`PlaybackCmd::Lyrics`).
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
    /// «Brani che ti piacciono» e le playlist: una vista sola che cambia raccolta.
    collection_page: Entity<CollectionPage>,
    /// Una vista per specie di pagina d'oggetto: cambia l'album, l'artista o la scheda che mostra.
    album_page: Entity<AlbumPage>,
    artist_page: Entity<ArtistPage>,
    source_page: Entity<SourcePage>,
    /// Le schede delle pagine esterne della cronologia (`Page::Source` ne tiene solo l'impronta).
    sources: HashMap<SourceKey, SourcePlaylist>,
    /// I nomi utente delle pagine di persone della cronologia (`Page::Profile` ne tiene l'impronta).
    people: HashMap<PersonKey, String>,
    /// L'account nella barra del titolo, e il proprio nome utente (per aprire il proprio profilo).
    account: AccountChip,
    own_handle: Option<String>,
    /// Le richieste di amicizia arrivate: il numero sulla voce «Amici».
    incoming: usize,
    /// Gli inviti alle jam arrivati: il numero sulla voce «Jam».
    invites: usize,
    /// I posti dell'equalizzatore delle viste con righe di brani: `Shell` lo dipinge sopra la vista
    /// (BACKLOG 23), così lo scatto dello sprite non rifà la pagina in cache.
    spots: Spots,
    /// La barra del titolo e la barra laterale in cache (`chrome.rs`), con ciò che mostravano all'ultimo
    /// confronto: si ridisegnano solo quando cambia.
    title_view: Entity<Chrome>,
    sidebar_view: Entity<Chrome>,
    chrome_keys: Option<(ChromeKey, ChromeKey)>,
    /// Il dialogo delle playlist, sopra tutto, se è aperto; con chi aveva il focus prima.
    dialog: Option<(Entity<PlaylistDialog>, Option<FocusHandle>, Subscription)>,
    /// La playlist appena creata: si apre quando il core la pubblica.
    pending_playlist: Option<uuid::Uuid>,
    /// Il menu «Altro» aperto: il brano, il pannello e le playlist (id e nome) prese all'apertura.
    menu: Option<OpenMenu>,
    /// I download in coda o in corso: il numero sulla voce «Scaricati» della barra laterale.
    busy_downloads: usize,
    /// Le uscite dei tuoi artisti non ancora viste: il numero sulla voce «Novità».
    unread_news: usize,
    /// La pagina mostrata per ultima: uscendo da «Novità» ciò che c'era si considera visto.
    shown: Page,
    /// La pagina corrente e quelle dietro e davanti (Indietro e Avanti).
    history: History,
    /// Il pannello laterale aperto.
    panel: Option<SidePanel>,
    /// Il marchio nella barra del titolo è lime solo mentre suona.
    playing: bool,
    /// L'avviso sul display e quelli che aspettano; il timer c'è solo mentre un avviso è mostrato.
    toasts: Toasts,
    /// L'ultimo evento di ogni dominio già mostrato (per numero d'ordine).
    seen: Seen,
    /// La chiusura è partita: si aspetta che il core salvi.
    closing: bool,
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
                        // Ogni tasto va al core, che aspetta 300 ms dall'ultimo prima di cercare.
                        core.read(cx).catalog(CatalogCmd::SearchInput(text.to_owned()));
                        shell.update(cx, |shell, cx| shell.show(Page::Search, cx)).ok();
                    }
                })
                .on_submit({
                    let core = core.clone();
                    move |text, _, cx| core.read(cx).catalog(CatalogCmd::Search(text.trim().to_owned()))
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
        #[cfg(feature = "video")]
        let video_panel = cx.new(|cx| VideoPanel::new(core.clone(), video.clone(), cx));
        let focus_view = cx.new(|cx| {
            FocusView::new(
                core.clone(),
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
            focus_view,
            focus_open: false,
            #[cfg(feature = "video")]
            video,
            #[cfg(feature = "video")]
            video_panel,
            #[cfg(feature = "video")]
            focus_video: false,
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
            } else {
                None
            },
            playing: snapshot.playback.is_playing(),
            toasts: Toasts::default(),
            seen: Seen::at_start(&snapshot),
            closing: false,
            _subscriptions: Vec::new(),
            core: core.clone(),
        };
        // Ogni volta che `Shell` si notifica: la barra del titolo e la barra laterale si rifanno solo se cambia
        // ciò che mostrano, rispetto a ciò che mostrano adesso.
        shell.chrome_keys = Some((shell.chrome_key(Part::TitleBar), shell.chrome_key(Part::Sidebar)));
        shell._subscriptions.push(cx.observe_self(|this, cx| this.sync_chrome(cx)));
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
            BarEvent::ToggleFocus => this.toggle_focus(cx),
            BarEvent::OpenArtist(id) => this.show(Page::Artist(*id), cx),
            BarEvent::OpenJam => this.show(Page::Jam, cx),
            #[cfg(feature = "video")]
            BarEvent::ToggleVideo => this.toggle_panel(SidePanel::Video, cx),
        }));
        shell._subscriptions.push(cx.subscribe(&shell.focus_view, |this, _, event: &FocusEvent, cx| {
            match event {
                FocusEvent::Close => this.set_focus(false, cx),
                #[cfg(feature = "video")]
                FocusEvent::Video(on) => this.set_focus_video(*on, cx),
            }
        }));
        #[cfg(feature = "video")]
        {
            shell._subscriptions.push(cx.subscribe(&shell.video_panel, |this, _, event: &PanelEvent, cx| {
                match event {
                    PanelEvent::Fullscreen => this.toggle_video_focus(cx),
                }
            }));
            // Un videoclip che si ferma è un avviso come gli altri.
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
        // Rete di sicurezza per le uscite che non passano da `request_close` (il sistema che termina l'app):
        // GPUI aspetta questo futuro per poco (`SHUTDOWN_TIMEOUT`), di solito basta per il salvataggio.
        shell._subscriptions.push(cx.on_app_quit(|this, cx| {
            let done = this.core.update(cx, |core, _| core.begin_shutdown());
            async move {
                if let Some(done) = done {
                    let _ = done.await;
                }
            }
        }));
        // La Home si carica sempre, come nel client Iced: è la pagina d'avvio e da lei nasce il feed (gli altri
        // artisti della pagina artista, BACKLOG 32), qualunque sia la pagina aperta.
        core.read(cx).catalog(CatalogCmd::Ensure(psytify_core::command::Resource::Home));
        if let Some(text) = launch.search.as_deref() {
            core.read(cx).catalog(CatalogCmd::Search(text.trim().to_owned()));
            shell.history = History::new(Page::Search);
        }
        if launch.focus && snapshot.playback.queue.current().is_some() {
            shell.focus_open = true;
        }
        #[cfg(feature = "video")]
        if launch.video {
            if shell.focus_open {
                shell.focus_video = true;
                shell.focus_view.update(cx, |view, cx| view.set_video(true, cx));
            } else {
                shell.panel = Some(SidePanel::Video);
            }
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
                // Il proprio profilo, o (con i dati di prova) quello che la vista ha aperto.
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
        shell.page_shown(cx);
        // I dati di prova di un aggiornamento non riuscito portano l'avviso che il core avrebbe dato.
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

    /// Cosa fare quando il core pubblica: avvisi e marchio. Le viste si aggiornano da sole.
    fn core_changed(
        &mut self,
        core: &Entity<CoreModel>,
        changed: Changed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = core.read(cx).snapshot();
        if changed.playback {
            let playing = snapshot.playback.is_playing();
            if playing != self.playing {
                self.playing = playing;
                cx.notify();
            }
            if self.focus_open && snapshot.playback.queue.current().is_none() {
                // Senza brano la vista immersiva non ha niente da mostrare (come nel client Iced).
                self.set_focus(false, cx);
            }
        }
        if changed.library
            && let Some(id) = self.pending_playlist
            && snapshot.library.playlists.iter().any(|playlist| playlist.id == id)
        {
            // La playlist appena creata è nella libreria: la si apre.
            self.pending_playlist = None;
            self.show(Page::Playlist(id), cx);
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
            // La fascia si ridisegna da sé se la sua riga cambia; `Shell` solo quando compare o sparisce.
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
        // Gli eventi nuovi di ogni dominio cambiato, in ordine: un solo sistema per tutti.
        let heard = self.seen.collect(&snapshot, changed);
        if heard.raise {
            window.activate_window();
        }
        for notice in heard.notices {
            self.notify(notice, cx);
        }
        if let Some(id) = heard.open_artist {
            // L'artista del catalogo ufficiale chiesto dalla ricerca è sul server: si apre la sua pagina.
            self.show(Page::Artist(id), cx);
        }
    }

    /// La vista sociale è cambiata: l'account nella barra, il numero delle richieste e, ad accesso fatto,
    /// l'uscita dalla pagina di accesso (come nel client Iced).
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

    /// Apre il profilo di `handle` (il suo nome resta in `people` finché la cronologia lo nomina).
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

    /// Un clic sull'account nella barra: l'accesso, o il proprio profilo (senza nome utente: gli Amici, che
    /// chiedono di sceglierne uno).
    fn account_clicked(&mut self, cx: &mut Context<Self>) {
        match (&self.account, self.own_handle.clone()) {
            (AccountChip::SignedOut, _) => self.show(Page::SignIn, cx),
            (AccountChip::SignedIn { .. }, Some(handle)) => self.open_person(&handle, None, cx),
            (AccountChip::SignedIn { .. }, None) => self.show(Page::Friends, cx),
            (AccountChip::Hidden | AccountChip::Working, _) => {}
        }
    }

    /// Mette un avviso in coda; se il display è vuoto lo mostra subito e arma il timer.
    fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        if self.toasts.push(notice) == Pushed::Shown {
            self.hold_toast(cx);
            cx.notify();
        }
    }

    /// Il timer dell'avviso sul display: `TOAST_HOLD_MS`, poi passa al prossimo o lascia il display
    /// vuoto. Ce n'è uno solo per volta e solo mentre un avviso è mostrato.
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

    /// Apre una pagina (e la mette nella cronologia). Niente se è già quella.
    /// Una pagina che si apre chiude la vista immersiva, che la coprirebbe.
    fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.history.go(page) {
            self.set_focus(false, cx);
            self.page_shown(cx);
        }
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.history.back().is_some() {
            self.set_focus(false, cx);
            self.page_shown(cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.history.forward().is_some() {
            self.set_focus(false, cx);
            self.page_shown(cx);
        }
    }

    /// La pagina corrente è cambiata: la vista di quella specie prende il suo oggetto (e lo chiede al core
    /// se non c'è), e si ridisegna la radice.
    fn page_shown(&mut self, cx: &mut Context<Self>) {
        let current = self.history.current();
        if self.shown == Page::News && current != Page::News {
            // Come il client Iced: uscendo da «Novità» ciò che c'era si considera visto.
            self.core.read(cx).library(LibraryCmd::MarkNewsSeen);
        }
        self.shown = current;
        match current {
            Page::Liked => self.collection_page.update(cx, |page, cx| page.show(Collection::Liked, cx)),
            Page::Playlist(id) => {
                self.collection_page.update(cx, |page, cx| page.show(Collection::Playlist(id), cx));
            }
            Page::Album(id) => self.album_page.update(cx, |page, cx| page.show(id, cx)),
            Page::Home => self.home_page.update(cx, |page, cx| page.show(cx)),
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
        cx.notify();
    }

    /// Apre la pagina di una scheda esterna; le schede che la cronologia non nomina più si scartano.
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

    /// Una pagina chiede di aprirne un'altra o un dialogo.
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
                // La playlist si apre quando il core la pubblica; se non la crea (vuota, libreria piena)
                // l'identificatore non compare mai.
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

    /// Salva un album o lo toglie, con l'avviso del client Iced.
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

    /// Segue un artista o smette, con l'avviso del client Iced.
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

    /// Una voce del menu «Altro»: il comando al core, l'avviso dove il client Iced lo dà.
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

    /// Il pannello del menu aperto.
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
                // I download dopo «Copia», come nel client Iced.
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

    /// Il dialogo è finito: si chiude, il focus torna dov'era e si fa ciò che dice l'esito.
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
            // Si lascia la playlist eliminata, poi la si toglie da tutta la cronologia.
            if self.history.current() == Page::Playlist(id) {
                self.show(Page::Library, cx);
            }
            self.history.forget(|page| page == Page::Playlist(id));
        }
    }

    /// I tasti della cronologia per la barra del titolo, accesi solo se c'è un passo da fare.
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

    /// Il tasto della coda (Q) o del testo (T): apre il pannello su quella scheda o lo chiude.
    fn toggle_panel(&mut self, kind: SidePanel, cx: &mut Context<Self>) {
        self.panel = toggled(self.panel, kind);
        self.sync_panels(cx);
        cx.notify();
    }

    /// Il tasto della vista immersiva (F): la apre se c'è un brano, la chiude se è aperta.
    fn toggle_focus(&mut self, cx: &mut Context<Self>) {
        let open = !self.focus_open && self.core.read(cx).snapshot().playback.queue.current().is_some();
        // F apre la faccia del testo (V quella del videoclip).
        #[cfg(feature = "video")]
        if open {
            self.set_focus_video(false, cx);
        }
        self.set_focus(open, cx);
    }

    fn set_focus(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.focus_open != open {
            self.focus_open = open;
            self.sync_panels(cx);
            cx.notify();
        }
    }

    /// V (o «Videoclip a tutto schermo» del pannello): apre la vista immersiva sul videoclip se il brano ne
    /// ha uno, la chiude se è già aperta lì (come `ToggleVideoFocus` del client Iced). Un videoclip che non
    /// si apriva riprova.
    #[cfg(feature = "video")]
    fn toggle_video_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_open && self.focus_video {
            self.set_focus(false, cx);
            return;
        }
        let has_video =
            self.core.read(cx).snapshot().playback.queue.current().is_some_and(|track| track.has_video);
        if !has_video {
            return;
        }
        if matches!(self.video.read(cx).status(), crate::video::stage::Status::Failed) {
            self.video.read(cx).retry(cx);
        }
        self.set_focus_video(true, cx);
        self.set_focus(true, cx);
    }

    /// La faccia della vista immersiva: il videoclip o il testo.
    #[cfg(feature = "video")]
    fn set_focus_video(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.focus_video != on {
            self.focus_video = on;
            self.focus_view.update(cx, |view, cx| view.set_video(on, cx));
            self.sync_panels(cx);
            cx.notify();
        }
    }

    /// Dice a ogni vista se si vede (solo allora ha i suoi timer) e al core se qualcuno mostra il testo.
    fn sync_panels(&mut self, cx: &mut Context<Self>) {
        let (panel, focus) = (self.panel, self.focus_open);
        self.bar.update(cx, |bar, cx| {
            bar.set_panel(panel, cx);
            bar.set_hidden(focus, cx);
        });
        let panel_lyrics = panel == Some(SidePanel::Lyrics) && !focus;
        self.lyrics.update(cx, |lyrics, cx| lyrics.set_visible(panel_lyrics, cx));
        self.focus_view.update(cx, |view, cx| view.set_open(focus, cx));
        #[cfg(feature = "video")]
        {
            let surface = video_surface(panel, focus, self.focus_video);
            self.video.update(cx, |stage, cx| stage.set_surface(surface, cx));
        }
        let wanted = panel == Some(SidePanel::Lyrics) || focus;
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

    /// Chiude la finestra dopo che il core ha scritto `config.json`. Si aspetta il salvataggio fino a
    /// [`SHUTDOWN_GRACE`]: oltre, si esce comunque (la configurazione resta quella dell'ultimo salvataggio).
    fn request_close(&mut self, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        let done = self.core.update(cx, |core, _| core.begin_shutdown());
        cx.spawn(async move |_, cx| {
            if let Some(done) = done {
                // L'esito non cambia niente: se il salvataggio è fallito il core l'ha già detto nel log.
                let _ = done.await;
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
        // Rete di sicurezza: un core che non risponde non deve tenere aperta la finestra.
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SHUTDOWN_GRACE).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    /// Il gestore del clic sull'account nella barra del titolo.
    fn account_handler(&self, cx: &mut Context<Self>) -> OnClose {
        let shell = cx.entity().downgrade();
        std::rc::Rc::new(move |_, cx| {
            shell.update(cx, |shell, cx| shell.account_clicked(cx)).ok();
        })
    }

    /// Ciò che la barra del titolo o la barra laterale mostra (`chrome.rs`).
    fn chrome_key(&self, part: Part) -> ChromeKey {
        match part {
            Part::TitleBar => ChromeKey::TitleBar {
                playing: self.playing,
                back: self.history.can_back(),
                forward: self.history.can_forward(),
                account: self.account.clone(),
            },
            Part::Sidebar => ChromeKey::Sidebar {
                current: self.history.current().nav(),
                counts: [self.busy_downloads, self.unread_news, self.incoming, self.invites],
            },
        }
    }

    /// `Shell` si è notificata: la barra del titolo e la barra laterale si ridisegnano se cambia ciò che
    /// mostrano (lo scatto di uno sprite che dipinge `Shell` non le rifà).
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

    /// Gli elementi della barra del titolo o della barra laterale, per la loro vista in cache (`Chrome`).
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
                };
                title_row(state, window, cx).into_any_element()
            }
            Part::Sidebar => {
                let rail = f32::from(window.viewport_size().width) < BP_RAIL;
                self.sidebar(rail, cx).into_any_element()
            }
        }
    }

    /// L'avviso sul display, sopra a tutto (anche al videoclip).
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

    /// Il gestore da dare alla barra del titolo e al sistema per chiudere la finestra.
    pub fn close_handler(this: &Entity<Self>) -> OnClose {
        let shell = this.downgrade();
        std::rc::Rc::new(move |_, cx| {
            shell.update(cx, |shell, cx| shell.request_close(cx)).ok();
        })
    }

    fn sidebar(&self, rail: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.history.current().nav();
        let items = Nav::ALL.iter().copied().enumerate().map(|(ix, nav)| {
            let (idle, filled) = nav.icons();
            // Animated icons say "something is going on" without a badge: unread news, a running download, a jam invite.
            let (idle, filled) = match nav {
                Nav::News if self.unread_news > 0 => (sprite::ICON_BELL_RING, sprite::ICON_BELL_RING),
                Nav::Downloads if self.busy_downloads > 0 => {
                    (sprite::ICON_DOWNLOAD_BUSY, sprite::ICON_DOWNLOAD_BUSY)
                }
                Nav::Jam if self.invites > 0 => (sprite::ICON_PEOPLE_LIVE, sprite::ICON_PEOPLE_LIVE),
                _ => (idle, filled),
            };
            let mut item = NavigationItem::new(("nav", ix), nav.label(), idle).icon_selected(filled);
            if nav == Nav::Downloads && self.busy_downloads > 0 {
                item = item.count(u32::try_from(self.busy_downloads).unwrap_or(u32::MAX));
            }
            if nav == Nav::News && self.unread_news > 0 {
                item = item.count(u32::try_from(self.unread_news).unwrap_or(u32::MAX));
            }
            if nav == Nav::Friends && self.incoming > 0 {
                item = item.count(u32::try_from(self.incoming).unwrap_or(u32::MAX));
            }
            if nav == Nav::Jam && self.invites > 0 {
                item = item.count(u32::try_from(self.invites).unwrap_or(u32::MAX));
            }
            item.selected(current == nav).rail(rail).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    if nav == Nav::Search {
                        this.focus_search(window, cx);
                    } else {
                        this.show(nav.page(), cx);
                    }
                },
            ))
        });
        div()
            .flex_none()
            .h_full()
            .w(dp(if rail { SIDEBAR_RAIL_W } else { SIDEBAR_W }))
            .flex()
            .flex_col()
            .gap(dp(2.0))
            .py(dp(SPACE_3))
            .bg(col(SURFACE_CHROME))
            .children(items)
    }

    /// La barra laterale, la pagina e il pannello laterale aperto (accanto alla pagina se c'è posto,
    /// altrimenti sopra).
    fn body(&self, rail: bool, docked: bool) -> gpui::Div {
        let mut body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .child(self.sidebar_view.clone().cached(sidebar_style(rail)))
            .child(Separator::vertical())
            // On very wide windows the page column stops at `CONTENT_MAX_W` and is centred.
            .child(
                div().flex_1().min_w_0().h_full().flex().justify_center().child(
                    div()
                        .relative()
                        .w_full()
                        .max_w(dp(CONTENT_MAX_W))
                        .h_full()
                        .child(self.page_content())
                        .children(self.page_spot().map(spot_overlay)),
                ),
            );
        if let Some(kind) = self.panel {
            let content = match kind {
                SidePanel::Queue => div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.queue.clone().cached(fill()))
                    .child(spot_overlay(self.spots.queue.clone())),
                SidePanel::Lyrics => {
                    div().flex_1().min_w_0().h_full().child(self.lyrics.clone().cached(fill()))
                }
                #[cfg(feature = "video")]
                SidePanel::Video => {
                    div().flex_1().min_w_0().h_full().child(self.video_panel.clone().cached(fill()))
                }
            };
            let panel = div()
                .flex_none()
                .h_full()
                .w(dp(SIDE_PANEL_W))
                .flex()
                .bg(col(SURFACE_CHROME))
                .child(Separator::vertical().strong())
                .child(content);
            body = if docked {
                body.child(panel)
            } else {
                body.child(div().absolute().top_0().bottom_0().right_0().occlude().child(panel))
            };
        }
        body
    }

    /// Il posto dell'equalizzatore della pagina corrente, se è una pagina con righe di brani.
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

/// L'account come lo mostra la barra del titolo.
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

/// Il proprio nome utente, se lo si è scelto.
fn own_handle(social: &SocialView) -> Option<String> {
    social.account.profile().map(|profile| profile.handle.clone())
}

/// Le richieste di amicizia arrivate.
fn incoming(social: &SocialView) -> usize {
    social.friends.ready().map_or(0, |friends| friends.incoming.len())
}

/// Lo stile con cui una vista a pieno riquadro entra in `.cached()`: la cache di GPUI non misura i figli,
/// dà alla vista la misura del contenitore. La vista si ridisegna solo quando si notifica da sé, quando
/// cambia la sua misura o quando la finestra si ridisegna per intero (`Window::refresh`: ridimensionamento,
/// arrivo di una copertina, passaggio del mouse sui bordi); gli scatti degli sprite invalidano solo la vista
/// che li ha dipinti (`pixel::anim`).
fn fill() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

/// La barra «in riproduzione» in `.cached()`: larga quanto la finestra, alta `PLAYER_BAR_H`.
fn bar_style() -> StyleRefinement {
    StyleRefinement::default().w_full().h(dp(PLAYER_BAR_H)).flex_none()
}

/// La riga della barra del titolo in `.cached()`: larga quanto la finestra, alta `TITLE_BAR_H`.
fn title_style() -> StyleRefinement {
    StyleRefinement::default().w_full().h(dp(TITLE_BAR_H)).flex_none()
}

/// La barra laterale in `.cached()`, larga quanto la voce o la sola icona (`rail`).
fn sidebar_style(rail: bool) -> StyleRefinement {
    StyleRefinement::default().flex_none().h_full().w(dp(if rail { SIDEBAR_RAIL_W } else { SIDEBAR_W }))
}

/// La fascia dell'aggiornamento in `.cached()`: larga quanto la finestra, alta `BANNER_H`.
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
        let width = f32::from(window.viewport_size().width);
        let rail = width < BP_RAIL;
        // Il pannello sta accanto alla pagina se c'è posto, altrimenti ci galleggia sopra.
        let docked = width >= BP_PANEL_DOCK;
        // In cache come le pagine: si rifanno solo quando cambia ciò che mostrano (`sync_chrome`), non a ogni
        // scatto dell'equalizzatore che `Shell` dipinge sopra le pagine (BACKLOG 23).
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
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.menu.is_some() {
                    this.close_menu(cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.focus_open && this.dialog.is_none() {
                    this.set_focus(false, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            // I tasti laterali del mouse, come in un browser.
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
            root =
                root.on_action(cx.listener(|this, _: &ToggleVideoFocus, _, cx| this.toggle_video_focus(cx)));
        }
        if self.banner_shown && !self.focus_open {
            // In cache alla sua altezza: il loader del download ridisegna solo la fascia.
            root = root.child(self.banner.clone().cached(banner_style())).child(Separator::horizontal());
        }
        root = if self.focus_open {
            // La vista immersiva prende tutto sotto la barra del titolo: niente pagina, pannello o barra.
            root.child(div().flex_1().min_h_0().child(self.focus_view.clone().cached(fill())))
        } else {
            root.child(self.body(rail, docked))
                .child(Separator::horizontal())
                // In cache come le pagine: si rifà quando si notifica da sé (il suo battito da 250 ms mentre
                // suona, un cambio di brano o di stato), non a ogni ridisegno di `Shell` (BACKLOG 23).
                .child(self.bar.clone().cached(bar_style()))
        };
        // Il palco del videoclip dopo pagina, pannelli e vista immersiva: dipinge il fotogramma nel pozzo che il
        // pannello o la vista immersiva hanno segnato in questo stesso disegno (`VideoStage::well`). Un
        // fotogramma rifà `Shell` (la radice: GPUI la rifà a ogni disegno) e il palco; le viste in cache, il
        // pannello e la vista immersiva compresi, si riusano (BACKLOG 41).
        #[cfg(feature = "video")]
        {
            root = root.child(self.video.clone());
        }

        if let Some(open) = &self.menu {
            // Un clic fuori dal menu lo chiude (e non arriva a ciò che c'è sotto).
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
            // Il pannello copre il velo: un clic su una voce non arriva al velo (che chiude).
            let panel = div().occlude().child(self.menu_panel(open, cx));
            root = root.child(popup(open.menu.at, panel));
        }
        if let Some((dialog, ..)) = &self.dialog {
            // Il velo copre tutta la finestra; un clic sul velo chiude come «Annulla».
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
    fn the_video_shows_in_one_place_at_a_time() {
        let video = Some(SidePanel::Video);
        assert_eq!(video_surface(video, false, false), Some(Surface::Panel));
        assert_eq!(video_surface(Some(SidePanel::Queue), false, true), None);
        assert_eq!(video_surface(None, false, false), None);
        // La vista immersiva copre il pannello: si vede lì, se è sul videoclip, o da nessuna parte.
        assert_eq!(video_surface(video, true, true), Some(Surface::Focus));
        assert_eq!(video_surface(video, true, false), None);
        assert_eq!(video_surface(None, true, true), Some(Surface::Focus));
        assert_eq!(toggled(video, SidePanel::Video), None);
    }

    #[test]
    fn the_queue_and_the_lyrics_share_the_panel() {
        // Come le schede del client Iced: il tasto apre la sua scheda, o chiude se era già aperta.
        assert_eq!(toggled(None, SidePanel::Lyrics), Some(SidePanel::Lyrics));
        assert_eq!(toggled(Some(SidePanel::Queue), SidePanel::Lyrics), Some(SidePanel::Lyrics));
        assert_eq!(toggled(Some(SidePanel::Lyrics), SidePanel::Lyrics), None);
        assert_eq!(toggled(Some(SidePanel::Lyrics), SidePanel::Queue), Some(SidePanel::Queue));
    }
}
