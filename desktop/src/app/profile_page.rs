//! A person's profile (`Page::Profile`), your own too (`PSYTIFY_ART_DIRECTION.md` «Profile»): the head with the avatar in the pixel circle, the name in
//! `TYPE_DISPLAY`, «@name · N amici» and the relation button (Modifica profilo, Amici, Accetta la richiesta,
//! Richiesta inviata, Aggiungi agli amici); the «Sta ascoltando» card (a `Card` with the cover and
//! «Ascolta»); the most played artists as a ranking (numbered rows); the recent listens as track rows
//! (`TrackRow`); the public playlists, which open their page; the mutual friends. A stranger's activity is
//! private and the page says so. Everything sits at the frame margins and enters with the page-enter move.
//!
//! The social view keeps one open (`SocialView::open`): `Shell` asks for `OpenProfile` when the page opens,
//! and the page shows it only if it is its handle's (else the skeleton). Texts are rebuilt when the open
//! profile or a request for it changes. `render` does not read the snapshot.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::Track;
use psytify_core::api::social::{NowPlaying, Relation, UserPage};
use psytify_core::catalog::Load;
use psytify_core::command::{LibraryCmd, PlaybackCmd, SocialCmd};
use psytify_core::snapshot::{DownloadsView, OpenProfile, SocialView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_PRIMARY, TEXT_SECONDARY};
use psytify_design::tokens::size::cover::ART_SHORTCUT;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_9};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_BODY, TYPE_HEADING, TYPE_TRACK_TITLE};
use uuid::Uuid;

use super::Page;
use super::PageEvent;
use super::album_page::failure;
use super::page_frame::PageFrame;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::people::{at, friends, plays};
use super::playlist_dialog::DialogRequest;
use super::track_list::{
    RowText, download_command, download_state, now_playing, plays as row_plays, with_links,
};
use super::track_menu::TrackMenu;
use crate::components::badge::{Badge, BadgeKind};
use crate::components::button::Button;
use crate::components::card::Card;
use crate::components::frame::{Frame, measure};
use crate::components::indicators::PlayState;
use crate::components::list_row::{ListRow, RowArt};
use crate::components::overlay::EmptyState;
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::person_row::{PersonRow, Status};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{note_at, section_heading_at};
use crate::components::tile::Cover;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::theme::{TypeStyle, col, dp};

/// The album column of the recent listens shows from this inner width (dp) up.
const ALBUM_COLUMN_MIN: f32 = 640.0;
/// The «Sta ascoltando» card keeps a readable width on a wide canvas.
const PRESENCE_MAX_W: f32 = 640.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    TopArtists,
    Recent,
    Playlists,
    Mutual,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::TopArtists => "Artisti più ascoltati",
            Self::Recent => "Ascolti recenti",
            Self::Playlists => "Playlist pubbliche",
            Self::Mutual => "Amici in comune",
        }
    }

    fn meta(self) -> &'static str {
        match self {
            Self::TopArtists => "ultimi 30 giorni",
            _ => "",
        }
    }
}

/// One row after the head, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Presence,
    Heading(Section),
    Artist(usize),
    Recent(usize),
    NoListens,
    /// The activity is for friends only.
    Private,
    Playlist(usize),
    Mutual(usize),
}

/// The profile's rows. Pure.
fn plan(page: &UserPage, recent: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    match &page.activity {
        Some(activity) => {
            if activity.now_playing.is_some() {
                rows.push(Row::Presence);
            }
            if !activity.top_artists.is_empty() {
                rows.push(Row::Heading(Section::TopArtists));
                rows.extend((0..activity.top_artists.len()).map(Row::Artist));
            }
            rows.push(Row::Heading(Section::Recent));
            if recent == 0 {
                rows.push(Row::NoListens);
            }
            rows.extend((0..recent).map(Row::Recent));
        }
        None if page.relation != Relation::Me => rows.push(Row::Private),
        None => {}
    }
    if !page.playlists.is_empty() {
        rows.push(Row::Heading(Section::Playlists));
        rows.extend((0..page.playlists.len()).map(Row::Playlist));
    }
    if !page.mutual_friends.is_empty() {
        rows.push(Row::Heading(Section::Mutual));
        rows.extend((0..page.mutual_friends.len()).map(Row::Mutual));
    }
    rows
}

/// What the relation button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelationClick {
    /// Asks for friendship (`true`) or withdraws the request (`false`) at once.
    Send(bool),
    /// Removes a friend, but asks for confirmation first (the owner's decision: one click was too little).
    ConfirmUnfriend,
}

/// The relation button in the head: the text, the icon and what it does. `None` for one's own profile
/// (there is «Modifica profilo»).
fn relation_action(relation: Relation) -> Option<(&'static str, Option<&'static str>, RelationClick)> {
    match relation {
        Relation::Me => None,
        Relation::Friend => Some(("Amici", Some(sprite::ICON_CHECK), RelationClick::ConfirmUnfriend)),
        Relation::Incoming => Some(("Accetta la richiesta", None, RelationClick::Send(true))),
        Relation::Outgoing => Some(("Richiesta inviata", None, RelationClick::Send(false))),
        Relation::None => Some(("Aggiungi agli amici", Some(sprite::ICON_PLUS), RelationClick::Send(true))),
    }
}

/// What the profile page of `handle` shows, from the social view.
#[derive(Clone, Debug)]
enum Shown {
    Loading,
    Missing,
    Failed(SharedString),
    Ready(Arc<OpenProfile>),
}

fn shown(view: &SocialView, handle: &str) -> Shown {
    match &view.open {
        Some((open, Load::Ready(profile))) if open == handle => Shown::Ready(Arc::clone(profile)),
        Some((open, Load::Failed(error))) if open == handle => {
            if error.is_not_found() {
                Shown::Missing
            } else {
                Shown::Failed(error.to_string().into())
            }
        }
        _ => Shown::Loading,
    }
}

/// The same state (so the texts are not redone): the same published profile, or the same load.
fn same(a: &Shown, b: &Shown) -> bool {
    match (a, b) {
        (Shown::Ready(a), Shown::Ready(b)) => Arc::ptr_eq(a, b),
        (Shown::Loading, Shown::Loading) | (Shown::Missing, Shown::Missing) => true,
        (Shown::Failed(a), Shown::Failed(b)) => a == b,
        _ => false,
    }
}

/// A profile's ready texts.
#[derive(Clone, Debug, Default)]
struct Texts {
    meta: SharedString,
    avatar: Option<SharedString>,
    name: SharedString,
    presence: Option<(NowPlaying, RowText)>,
    artists: Vec<(Uuid, SharedString, SharedString)>,
    recent: Vec<RowText>,
    playlists: Vec<(Uuid, SharedString, SharedString)>,
    mutual: Vec<(String, SharedString, Option<SharedString>)>,
}

impl Texts {
    fn of(open: &OpenProfile) -> Self {
        let page = &open.page;
        let activity = page.activity.as_ref();
        Self {
            meta: format!("@{} · {}", page.profile.handle, friends(page.friend_count)).into(),
            avatar: page.profile.avatar_url.clone().map(Into::into),
            name: page.profile.display_name.clone().into(),
            presence: activity.and_then(|activity| activity.now_playing.clone()).map(|now| {
                let text = RowText::of(&now.track);
                (now, text)
            }),
            artists: activity
                .map(|activity| {
                    activity
                        .top_artists
                        .iter()
                        .map(|artist| (artist.id, artist.name.clone().into(), plays(artist.plays).into()))
                        .collect()
                })
                .unwrap_or_default(),
            recent: open.recent.iter().map(RowText::of).collect(),
            playlists: page
                .playlists
                .iter()
                .map(|playlist| {
                    let tracks = if playlist.track_count == 1 {
                        "1 brano".to_owned()
                    } else {
                        format!("{} brani", playlist.track_count)
                    };
                    (
                        playlist.id,
                        playlist.name.clone().into(),
                        format!("Playlist pubblica · {tracks}").into(),
                    )
                })
                .collect(),
            mutual: page
                .mutual_friends
                .iter()
                .map(|p| {
                    (p.handle.clone(), p.display_name.clone().into(), p.avatar_url.clone().map(Into::into))
                })
                .collect(),
        }
    }
}

pub struct ProfilePage {
    core: Entity<CoreModel>,
    handle: String,
    shown: Shown,
    /// A friendship request for this person is in flight.
    busy: bool,
    rows: Vec<Row>,
    texts: Texts,
    downloads: Arc<DownloadsView>,
    liked: Arc<HashSet<Uuid>>,
    now: Option<(Uuid, PlayState)>,
    list: ListState,
    frame: PageFrame,
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for ProfilePage {}

impl ProfilePage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        Self {
            handle: String::new(),
            shown: Shown::Loading,
            busy: false,
            rows: Vec::new(),
            texts: Texts::default(),
            downloads: Arc::clone(&snapshot.downloads),
            liked: core.read(cx).liked(),
            now: now_playing(&snapshot.playback),
            list: new_list(0),
            frame: PageFrame::default(),
            spot: PlayingSpot::new(),
            core,
            _changes: changes,
        }
    }

    /// This view's equalizer spot, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame != self.frame.frame {
            self.frame.frame = frame;
            cx.notify();
        }
    }

    /// Shows `handle`'s profile and asks the core for it (nothing happens if it is open and ready).
    pub fn show(&mut self, handle: &str, cx: &mut Context<Self>) {
        self.core.read(cx).social(SocialCmd::OpenProfile(handle.to_owned()));
        if handle == self.handle {
            return;
        }
        self.handle = handle.to_owned();
        to_top(&self.list);
        let view = self.core.read(cx).social_view();
        self.take(&view);
        cx.notify();
    }

    /// Takes the new view; `true` if what is shown has changed.
    fn take(&mut self, view: &SocialView) -> bool {
        let next = shown(view, &self.handle);
        let busy = view.pending.contains(&self.handle);
        if same(&next, &self.shown) && busy == self.busy {
            return false;
        }
        if !same(&next, &self.shown) {
            match &next {
                Shown::Ready(open) => {
                    self.texts = Texts::of(open);
                    self.rows = plan(&open.page, open.recent.len());
                }
                _ => {
                    self.texts = Texts::default();
                    self.rows.clear();
                }
            }
            set_rows(&self.list, self.rows.len());
        }
        self.shown = next;
        self.busy = busy;
        remeasure_header(&self.list);
        true
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let mut dirty = false;
        if changed.social {
            dirty |= self.take(&core.read(cx).social_view());
        }
        let snapshot = core.read(cx).snapshot();
        if changed.library {
            let liked = core.read(cx).liked();
            if !Arc::ptr_eq(&liked, &self.liked) {
                self.liked = liked;
                dirty = true;
            }
        }
        if changed.downloads && !Arc::ptr_eq(&snapshot.downloads, &self.downloads) {
            self.downloads = Arc::clone(&snapshot.downloads);
            dirty = true;
        }
        if changed.playback {
            let now = now_playing(&snapshot.playback);
            if now != self.now {
                self.now = now;
                dirty = true;
            }
        }
        if dirty {
            cx.notify();
        }
    }

    fn open(&self) -> Option<&Arc<OpenProfile>> {
        match &self.shown {
            Shown::Ready(open) => Some(open),
            _ => None,
        }
    }

    fn header(&self, open: &OpenProfile, cx: &mut Context<Self>) -> AnyElement {
        let mut header =
            PageHeader::new(HeaderArt::Avatar(self.texts.avatar.clone()), "Profilo", self.texts.name.clone())
                .margin(self.frame.margin())
                .meta(self.texts.meta.clone());
        let relation = open.page.relation;
        match relation_action(relation) {
            None => {
                header = header.action(
                    Button::new("profile-edit", "Modifica profilo")
                        .secondary()
                        .icon(sprite::ICON_EDIT)
                        .on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenAccountSettings)),
                        ),
                );
            }
            Some((label, icon, click)) => {
                let handle = open.page.profile.handle.clone();
                let name = open.page.profile.display_name.clone();
                let mut button = Button::new("profile-relation", label)
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match click {
                        RelationClick::Send(add) => {
                            this.core
                                .read(cx)
                                .social(SocialCmd::FriendAction { handle: handle.clone(), add });
                        }
                        RelationClick::ConfirmUnfriend => {
                            cx.emit(PageEvent::Dialog(DialogRequest::Unfriend {
                                handle: handle.clone(),
                                name: name.clone(),
                            }));
                        }
                    }));
                button = if relation == Relation::Outgoing { button.ghost() } else { button.secondary() };
                if let Some(icon) = icon {
                    button = button.icon(icon);
                }
                header = header.action(button);
            }
        }
        header.into_any_element()
    }

    fn presence(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (now, text) = self.texts.presence.clone()?;
        let badge = if now.paused {
            div().type_role(TYPE_ARTIST).text_color(col(TEXT_SECONDARY)).child("In pausa").into_any_element()
        } else {
            Badge::new("Sta ascoltando", BadgeKind::Live).into_any_element()
        };
        let track = now.track.clone();
        let card = Card::new().child(
            div()
                .flex()
                .items_center()
                .gap(dp(SPACE_5))
                .child(Cover::new(ART_SHORTCUT).url(text.cover_url.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_2))
                        // The badge keeps its width, on the left: in a column it would stretch.
                        .child(div().flex().child(badge))
                        .child(
                            div()
                                .type_role(TYPE_TRACK_TITLE)
                                .text_color(col(TEXT_PRIMARY))
                                .truncate()
                                .child(text.title.clone()),
                        )
                        .child(
                            div()
                                .type_role(TYPE_ARTIST)
                                .text_color(col(TEXT_SECONDARY))
                                .truncate()
                                .child(text.artist.clone()),
                        ),
                )
                .child(
                    Button::new("profile-listen", "Ascolta").secondary().icon(sprite::ICON_PLAY).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let tracks: Arc<[Track]> = Arc::from(vec![track.clone()]);
                            this.core.read(cx).playback(PlaybackCmd::Play { tracks, start: 0 });
                        }),
                    ),
                ),
        );
        Some(
            div()
                .px(dp(self.frame.margin()))
                .py(dp(SPACE_3))
                .child(div().max_w(dp(PRESENCE_MAX_W)).child(card))
                .into_any_element(),
        )
    }

    fn recent(&self, ix: usize, wide: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.open()?;
        let track = open.recent.get(ix)?;
        let text = self.texts.recent.get(ix)?;
        let data = TrackData {
            index: u32::try_from(ix + 1).unwrap_or(u32::MAX),
            title: text.title.clone(),
            artist: text.artist.clone(),
            album: text.album.clone(),
            duration: text.duration.clone(),
            liked: self.liked.contains(&track.id),
            download: download_state(&self.downloads, track),
            available: true,
            cover_url: text.cover_url.clone(),
        };
        let (liked, download, menu) = (track.clone(), track.clone(), track.clone());
        let mut row = TrackRow::new(("profile-recent", ix), data)
            .show_album(wide)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if row_plays(event) {
                    this.play_recent(ix, cx);
                }
            }))
            .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).library(LibraryCmd::ToggleLike(liked.clone()));
            }))
            .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).downloads(download_command(&this.downloads, &download));
            }))
            .on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                    track: menu.clone(),
                    at: event.position(),
                    from_playlist: None,
                })));
            }));
        row = with_links(row, track, cx);
        if let Some((id, state)) = self.now
            && id == track.id
        {
            row = row.playing_at(state, &self.spot);
        }
        Some(self.frame.row(row))
    }

    /// Double click on a listen: the queue becomes the recent listens from there; on the playing track,
    /// play or pause.
    fn play_recent(&self, start: usize, cx: &gpui::App) {
        let Some(open) = self.open() else { return };
        let core = self.core.read(cx);
        match (open.recent.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(&open.recent), start }),
        }
    }

    fn row(&self, row: Row, wide: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match row {
            Row::Presence => return self.presence(cx),
            Row::Heading(section) => {
                section_heading_at(section.title().into(), section.meta().into(), margin).into_any_element()
            }
            Row::Artist(ix) => {
                let (id, name, detail) = self.texts.artists.get(ix)?.clone();
                let row = ListRow::new(("profile-artist", ix), RowArt::None, name)
                    .index(u32::try_from(ix + 1).unwrap_or(u32::MAX))
                    .detail(detail)
                    .on_click(
                        cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::Open(Page::Artist(id)))
                        }),
                    );
                self.frame.row(row)
            }
            Row::Recent(ix) => return self.recent(ix, wide, cx),
            Row::NoListens => note_at(
                div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child("Ancora nessun ascolto."),
                margin,
            )
            .into_any_element(),
            Row::Private => div()
                .px(dp(margin))
                .py(dp(SPACE_5))
                .flex()
                .flex_col()
                .gap(dp(SPACE_2))
                .child(
                    div()
                        .type_role(TYPE_HEADING)
                        .text_color(col(TEXT_PRIMARY))
                        .child("L'attività è per gli amici"),
                )
                .child(div().max_w(measure(TYPE_BODY)).type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(
                    "Quando siete amici vedi cosa ascolta, gli ascolti recenti e i suoi artisti preferiti.",
                ))
                .into_any_element(),
            Row::Playlist(ix) => {
                let (id, name, detail) = self.texts.playlists.get(ix)?.clone();
                let handle = self.handle.clone();
                self.frame.row(
                    ListRow::new(("profile-playlist", ix), RowArt::Sprite(sprite::ICON_LIST), name)
                        .inset(SPACE_3)
                        .detail(detail)
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::OpenFriendPlaylist { handle: handle.clone(), id })
                        })),
                )
            }
            Row::Mutual(ix) => {
                let (handle, name, avatar) = self.texts.mutual.get(ix)?.clone();
                self.frame.row(
                    PersonRow::new(("profile-mutual", ix), name, Status::Plain(at(&handle)))
                        .avatar(avatar)
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::OpenProfile(handle.clone()))
                        })),
                )
            }
        })
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.open().cloned().map(|open| self.header(&open, cx)),
            Some(row) => {
                let wide = self.frame.frame.inner >= ALBUM_COLUMN_MIN;
                self.rows.get(row).copied().and_then(|row| self.row(row, wide, cx))
            }
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        // The head, then one step per section.
        let section = ix.checked_sub(1).map_or(0, |row| {
            1 + self.rows.iter().take(row + 1).filter(|row| matches!(row, Row::Heading(_))).count()
        });
        self.frame.wrap(element, section, window)
    }
}

impl Render for ProfilePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.spot.clear();
        self.frame.begin(window, Some(&self.list));
        let page = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame));
        match &self.shown {
            Shown::Loading => page.child(PageHeader::loading_person(self.frame.margin())),
            Shown::Missing => page.child(
                div().flex_1().flex().justify_center().pt(dp(SPACE_9)).child(
                    EmptyState::crate_(
                        "Questa persona non esiste più",
                        "Forse ha chiuso l'account o cambiato nome utente.",
                    )
                    .action(Button::new("profile-friends", "Vai agli amici").on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Friends))),
                    )),
                ),
            ),
            Shown::Failed(error) => page.child(failure(
                "profile-retry",
                error.clone(),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).social(SocialCmd::ReloadProfile)
                }),
            )),
            Shown::Ready(_) => {
                let list = list(
                    self.list.clone(),
                    cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
                )
                .size_full();
                page.child(div().flex_1().min_h_0().child(list))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;
    use psytify_core::api::social::{Activity, PlaylistSummary, Profile, TopArtist};
    use psytify_core::error::AppError;

    use super::*;

    fn person(handle: &str) -> Profile {
        Profile { handle: handle.to_owned(), display_name: handle.to_owned(), avatar_url: None }
    }

    fn user(relation: Relation, activity: Option<Activity>) -> UserPage {
        UserPage {
            profile: person("luca"),
            relation,
            friend_count: 1,
            mutual_friends: vec![person("sara")],
            playlists: vec![PlaylistSummary { id: Uuid::nil(), name: "Sera".to_owned(), track_count: 1 }],
            activity,
        }
    }

    #[test]
    fn a_friend_shows_what_they_play_their_artists_and_listens() {
        let activity = Activity {
            now_playing: Some(NowPlaying { track: fixtures::track(1), position_ms: 0, paused: false }),
            recent: Vec::new(),
            top_artists: vec![TopArtist { id: Uuid::nil(), name: "Marea".to_owned(), plays: 3 }],
        };
        assert_eq!(
            plan(&user(Relation::Friend, Some(activity)), 2),
            vec![
                Row::Presence,
                Row::Heading(Section::TopArtists),
                Row::Artist(0),
                Row::Heading(Section::Recent),
                Row::Recent(0),
                Row::Recent(1),
                Row::Heading(Section::Playlists),
                Row::Playlist(0),
                Row::Heading(Section::Mutual),
                Row::Mutual(0),
            ]
        );
    }

    #[test]
    fn a_stranger_keeps_the_activity_private_and_an_empty_history_says_so() {
        let rows = plan(&user(Relation::None, None), 0);
        assert_eq!(rows[0], Row::Private);
        let quiet = Activity { now_playing: None, recent: Vec::new(), top_artists: Vec::new() };
        assert_eq!(
            plan(&user(Relation::Me, Some(quiet)), 0)[..2],
            [Row::Heading(Section::Recent), Row::NoListens]
        );
        assert!(
            !plan(&user(Relation::Me, None), 0).contains(&Row::Private),
            "il proprio profilo non è privato"
        );
    }

    #[test]
    fn the_relation_button_matches_the_iced_client() {
        assert_eq!(relation_action(Relation::Me), None);
        assert_eq!(relation_action(Relation::Friend).map(|a| a.0), Some("Amici"));
        assert_eq!(relation_action(Relation::Incoming).map(|a| a.0), Some("Accetta la richiesta"));
        assert_eq!(relation_action(Relation::Outgoing).map(|a| a.0), Some("Richiesta inviata"));
        assert_eq!(relation_action(Relation::None).map(|a| a.0), Some("Aggiungi agli amici"));
        assert_eq!(friends(12), "12 amici");
    }

    #[test]
    fn removing_a_friend_asks_first_and_the_rest_acts_at_once() {
        let click = |relation| relation_action(relation).map(|a| a.2);
        assert_eq!(click(Relation::Friend), Some(RelationClick::ConfirmUnfriend));
        assert_eq!(click(Relation::Incoming), Some(RelationClick::Send(true)));
        assert_eq!(click(Relation::Outgoing), Some(RelationClick::Send(false)));
        assert_eq!(click(Relation::None), Some(RelationClick::Send(true)));
    }

    #[test]
    fn only_the_profile_of_this_handle_is_shown() {
        let open =
            Arc::new(OpenProfile { page: user(Relation::Friend, None), recent: Arc::from(Vec::new()) });
        let mut view = SocialView {
            open: Some(("luca".to_owned(), Load::Ready(Arc::clone(&open)))),
            ..SocialView::default()
        };
        assert!(matches!(shown(&view, "luca"), Shown::Ready(_)));
        assert!(matches!(shown(&view, "sara"), Shown::Loading), "un altro profilo aperto: lo scheletro");
        assert!(same(&shown(&view, "luca"), &Shown::Ready(open)));
        view.open = Some((
            "luca".to_owned(),
            Load::Failed(AppError::Api { status: 404, code: "not_found".into(), message: "x".into() }),
        ));
        assert!(matches!(shown(&view, "luca"), Shown::Missing));
        view.open = Some(("luca".to_owned(), Load::Failed(AppError::Network)));
        assert!(matches!(shown(&view, "luca"), Shown::Failed(_)));
    }
}
