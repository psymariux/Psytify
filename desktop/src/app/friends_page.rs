//! «Amici» (`Page::Friends`, `PSYTIFY_ART_DIRECTION.md` «Friends»): the title with your username, the search by username (Enter searches,
//! an empty field clears the results), then one list of 48 dp rows at the frame margins: the people found,
//! the requests received (Accetta secondary, Rifiuta ghost), those pending (Ritira) and the friends with what
//! they listen to («Sta ascoltando …» with the live dot). A click on a person opens their profile. While a
//! request for a person is in flight (`SocialView::pending`) their buttons are off.
//!
//! Signed out: Nastro and «Accedi»; without a username: the invitation to choose one in Settings; nobody yet:
//! the empty crate. At most one Nastro per screen. On opening the page asks for `RefreshFriends`. The page
//! enters with the page-enter move.
//!
//! The rows and their texts are a pure function of the social view (`model`), rebuilt when the core
//! publishes; the page redraws only if the result changes. `render` does not read the snapshot.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::api::social::{Friends, Profile};
use psytify_core::catalog::Load;
use psytify_core::command::SocialCmd;
use psytify_core::snapshot::{Account, SocialView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR, TEXT_SECONDARY};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::{SPACE_3, SPACE_5, SPACE_9};
use psytify_design::tokens::type_::TYPE_BODY;

use super::page_frame::PageFrame;
use super::people::{at, status};
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::Frame;
use crate::components::overlay::EmptyState;
use crate::components::person_row::{PersonRow, Status};
use crate::components::search_field::SearchField;
use crate::components::section::{PageTitle, note_at, section_heading_at};
use crate::core_model::{Changed, CoreModel};
use crate::theme::{TypeStyle, col, dp};

/// What the page can show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gate {
    #[default]
    SignedOut,
    /// Signed in but without a username yet.
    NoProfile,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    People,
    Requests,
    Waiting,
    Friends,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::People => "Persone",
            Self::Requests => "Richieste",
            Self::Waiting => "In attesa",
            Self::Friends => "I tuoi amici",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Searching,
    NoneFound,
    SearchFailed,
    Loading,
    NoFriends,
}

impl Line {
    fn text(self) -> &'static str {
        match self {
            Self::Searching => "Cerco…",
            Self::NoneFound => "Nessuno con questo nome.",
            Self::SearchFailed => "La ricerca non è riuscita. Riprova.",
            Self::Loading => "Carico gli amici…",
            Self::NoFriends => "Ancora nessun amico: cerca qualcuno qui sopra e mandagli una richiesta.",
        }
    }
}

/// A row of the list, in the order it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section, usize),
    Person(usize),
    Note(Line),
    /// The friends did not load: the error and «Riprova».
    Retry,
}

/// The buttons of a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Actions {
    None,
    /// Found, not yet a friend: «Aggiungi».
    Add,
    /// Found, already a friend: «Amici», off.
    Friends,
    /// Found, request already sent: «Richiesta inviata», off.
    Sent,
    /// Found, they sent you a request: «Accetta».
    Accept,
    /// An incoming request: «Accetta» and «Rifiuta».
    AcceptReject,
    /// An outgoing request: «Ritira».
    Withdraw,
}

/// A person of a row, with the texts ready.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Person {
    handle: String,
    name: SharedString,
    avatar: Option<SharedString>,
    status: Status,
    actions: Actions,
    /// A request for them is in flight: the buttons are off.
    busy: bool,
}

/// Everything the page shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Model {
    gate: Gate,
    /// «Sei @nome».
    me: SharedString,
    rows: Vec<Row>,
    people: Vec<Person>,
    /// The error of loading the friends.
    error: Option<SharedString>,
}

/// The relation of a person found, from the friends lists.
fn found_actions(handle: &str, own: &str, friends: Option<&Friends>) -> Actions {
    if handle == own {
        return Actions::None;
    }
    let Some(friends) = friends else { return Actions::Add };
    let has = |list: &[Profile]| list.iter().any(|p| p.handle == handle);
    if friends.friends.iter().any(|f| f.profile.handle == handle) {
        Actions::Friends
    } else if has(&friends.outgoing) {
        Actions::Sent
    } else if has(&friends.incoming) {
        Actions::Accept
    } else {
        Actions::Add
    }
}

/// The rows and the texts of the page. Pure.
fn model(view: &SocialView) -> Model {
    let own = match &view.account {
        Account::SignedIn(me) => match &me.profile {
            Some(profile) => profile.handle.clone(),
            None => return Model { gate: Gate::NoProfile, ..Model::default() },
        },
        _ => return Model::default(),
    };
    let mut model = Model { gate: Gate::Ready, me: format!("Sei @{own}").into(), ..Model::default() };
    let friends = view.friends.ready().map(Arc::as_ref);
    let add = |model: &mut Model, profile: &Profile, status: Status, actions: Actions| {
        model.rows.push(Row::Person(model.people.len()));
        model.people.push(Person {
            handle: profile.handle.clone(),
            name: profile.display_name.clone().into(),
            avatar: profile.avatar_url.clone().map(Into::into),
            status,
            actions,
            busy: view.pending.contains(&profile.handle),
        });
    };
    match &view.found {
        Load::Idle => {}
        Load::Loading => model.rows.extend([Row::Heading(Section::People, 0), Row::Note(Line::Searching)]),
        Load::Failed(_) => {
            model.rows.extend([Row::Heading(Section::People, 0), Row::Note(Line::SearchFailed)])
        }
        Load::Ready(found) => {
            model.rows.push(Row::Heading(Section::People, found.len()));
            if found.is_empty() {
                model.rows.push(Row::Note(Line::NoneFound));
            }
            for profile in found.iter() {
                let actions = found_actions(&profile.handle, &own, friends);
                add(&mut model, profile, Status::Plain(at(&profile.handle)), actions);
            }
        }
    }
    match &view.friends {
        Load::Ready(friends) => {
            if !friends.incoming.is_empty() {
                model.rows.push(Row::Heading(Section::Requests, friends.incoming.len()));
                for profile in &friends.incoming {
                    add(&mut model, profile, Status::Plain(at(&profile.handle)), Actions::AcceptReject);
                }
            }
            if !friends.outgoing.is_empty() {
                model.rows.push(Row::Heading(Section::Waiting, friends.outgoing.len()));
                for profile in &friends.outgoing {
                    add(&mut model, profile, Status::Plain("Richiesta inviata".into()), Actions::Withdraw);
                }
            }
            let nobody =
                friends.friends.is_empty() && friends.incoming.is_empty() && friends.outgoing.is_empty();
            if nobody && matches!(view.found, Load::Idle) {
                // No one and no search: the page's empty state, not a list.
                return Model { rows: Vec::new(), people: Vec::new(), ..model };
            }
            model.rows.push(Row::Heading(Section::Friends, friends.friends.len()));
            if friends.friends.is_empty() {
                model.rows.push(Row::Note(Line::NoFriends));
            }
            for friend in &friends.friends {
                let line = status(&friend.profile.handle, friend.now_playing.as_ref());
                add(&mut model, &friend.profile, line, Actions::None);
            }
        }
        Load::Failed(error) => {
            model.error = Some(error.to_string().into());
            model.rows.push(Row::Retry);
        }
        Load::Idle | Load::Loading => model.rows.push(Row::Note(Line::Loading)),
    }
    model
}

pub struct FriendsPage {
    core: Entity<CoreModel>,
    model: Model,
    search: Entity<SearchField>,
    scroll: UniformListScrollHandle,
    frame: PageFrame,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for FriendsPage {}

impl FriendsPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.social {
                let model = model(&core.read(cx).social_view());
                if model != this.model {
                    this.model = model;
                    cx.notify();
                }
            }
        });
        let search = cx.new(|cx| {
            SearchField::new("friends-search", "Cerca per nome utente", cx)
                .without_hint()
                .sanitize(super::people::line_chars)
                .max_chars(super::people::TEXT_MAX)
                .on_change({
                    let core = core.clone();
                    move |text, _, cx| {
                        // You search with Enter; an empty field clears the results.
                        if text.trim().trim_start_matches('@').is_empty() {
                            core.read(cx).social(SocialCmd::SearchUsers(String::new()));
                        }
                    }
                })
                .on_submit({
                    let core = core.clone();
                    move |text, _, cx| {
                        let query = text.trim().trim_start_matches('@').to_owned();
                        core.read(cx).social(SocialCmd::SearchUsers(query));
                    }
                })
        });
        let model = model(&core.read(cx).social_view());
        Self {
            core,
            model,
            search,
            scroll: UniformListScrollHandle::new(),
            frame: PageFrame::default(),
            _changes: changes,
        }
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

    /// The page opens: friends and requests are read again.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        if self.model.gate == Gate::Ready {
            self.core.read(cx).social(SocialCmd::RefreshFriends);
        }
    }

    fn friend_action(&self, handle: &str, add: bool, cx: &gpui::App) {
        self.core.read(cx).social(SocialCmd::FriendAction { handle: handle.to_owned(), add });
    }

    fn person(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let person = self.model.people.get(ix)?;
        let open = person.handle.clone();
        let mut row = PersonRow::new(("friends-person", ix), person.name.clone(), person.status.clone())
            .avatar(person.avatar.clone())
            .on_click(
                cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenProfile(open.clone()))),
            );
        let action = |id: &'static str, label: &'static str, add: bool, cx: &mut Context<Self>| {
            let handle = person.handle.clone();
            Button::new((id, ix), label).disabled(person.busy).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.friend_action(&handle, add, cx)),
            )
        };
        row = match person.actions {
            Actions::None => row,
            Actions::Add => {
                row.action(action("friends-add", "Aggiungi", true, cx).secondary().icon(sprite::ICON_PLUS))
            }
            Actions::Friends => row.action(Button::new(("friends-are", ix), "Amici").ghost().disabled(true)),
            Actions::Sent => {
                row.action(Button::new(("friends-sent", ix), "Richiesta inviata").ghost().disabled(true))
            }
            Actions::Accept => row.action(action("friends-accept", "Accetta", true, cx).secondary()),
            Actions::AcceptReject => row
                .action(action("friends-accept", "Accetta", true, cx).secondary())
                .action(action("friends-reject", "Rifiuta", false, cx).ghost()),
            Actions::Withdraw => row.action(action("friends-withdraw", "Ritira", false, cx).ghost()),
        };
        Some(self.frame.row(row))
    }

    /// List row `ix`, in its enter move (one step per section).
    fn line(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let element = self.row(ix, cx)?;
        let section =
            2 + self.model.rows.iter().take(ix + 1).filter(|row| matches!(row, Row::Heading(..))).count();
        let element = div().h(dp(TRACK_ROW_H)).w_full().child(element).into_any_element();
        Some(self.frame.wrap(element, section, window))
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match *self.model.rows.get(ix)? {
            Row::Heading(section, count) => {
                let meta = if count > 0 { count.to_string() } else { String::new() };
                section_heading_at(section.title().into(), meta.into(), margin).into_any_element()
            }
            Row::Person(person) => return self.person(person, cx),
            Row::Note(line) => {
                note_at(div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(line.text()), margin)
                    .into_any_element()
            }
            Row::Retry => note_at(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_5))
                    .child(
                        div()
                            .type_role(TYPE_BODY)
                            .text_color(col(TEXT_ERROR))
                            .child(self.model.error.clone().unwrap_or_default()),
                    )
                    .child(Button::new("friends-retry", "Riprova").secondary().on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.core.read(cx).social(SocialCmd::RefreshFriends),
                    ))),
                margin,
            )
            .into_any_element(),
        })
    }
}

/// An empty state in the middle, a third from the top.
fn centered(content: impl IntoElement) -> gpui::Div {
    div().flex_1().flex().justify_center().pt(dp(SPACE_9)).child(content)
}

impl Render for FriendsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, None);
        let margin = self.frame.margin();
        let page = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame));
        match self.model.gate {
            Gate::SignedOut => {
                return page.child(PageTitle::screen("Amici").margin(margin)).child(centered(
                    EmptyState::new(
                        sprite::NASTRO_IDLE,
                        "Accedi per trovare i tuoi amici",
                        "Vedi cosa ascoltano, le loro playlist e gli artisti che amano.",
                    )
                    .action(Button::new("friends-signin", "Accedi").on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::SignIn))),
                    )),
                ));
            }
            Gate::NoProfile => {
                return page.child(PageTitle::screen("Amici").margin(margin)).child(centered(
                    EmptyState::crate_(
                        "Scegli un nome utente",
                        "Gli amici ti trovano così: sceglilo nelle impostazioni, in Account.",
                    )
                    .action(
                        Button::new("friends-settings", "Apri le impostazioni").on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenAccountSettings)),
                        ),
                    ),
                ));
            }
            Gate::Ready => {}
        }
        let title = PageTitle::screen("Amici").margin(margin).meta(self.model.me.clone());
        let search =
            div().flex().items_center().gap(dp(SPACE_3)).px(dp(margin)).child(self.search.clone()).child(
                Button::new("friends-search-go", "Cerca").secondary().on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let query = this.search.read(cx).text().trim().trim_start_matches('@').to_owned();
                        this.core.read(cx).social(SocialCmd::SearchUsers(query));
                    },
                )),
            );
        let page = page.child(self.frame.wrap(title.into_any_element(), 0, window)).child(self.frame.wrap(
            search.into_any_element(),
            1,
            window,
        ));
        if self.model.rows.is_empty() {
            return page.child(centered(EmptyState::crate_(
                "Ancora nessun amico",
                "Cerca qualcuno per nome utente qui sopra e mandagli una richiesta.",
            )));
        }
        let list = uniform_list(
            "friends-list",
            self.model.rows.len(),
            cx.processor(|this: &mut Self, range: std::ops::Range<usize>, window, cx| {
                range.filter_map(|ix| this.line(ix, window, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        page.child(div().flex_1().min_h_0().pt(dp(SPACE_3)).child(list))
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::Me;
    use psytify_core::api::models::fixtures;
    use psytify_core::api::social::{Friend, MyProfile, NowPlaying};
    use psytify_core::error::AppError;

    use super::*;

    fn person(handle: &str) -> Profile {
        Profile { handle: handle.to_owned(), display_name: handle.to_uppercase(), avatar_url: None }
    }

    fn signed_in(friends: Load<Arc<Friends>>, found: Load<Arc<[Profile]>>) -> SocialView {
        let me = Me {
            user_id: "u".to_owned(),
            admin: false,
            profile: Some(MyProfile {
                handle: "psy".to_owned(),
                display_name: "Psy".to_owned(),
                avatar_url: None,
                share_activity: true,
            }),
        };
        SocialView { account: Account::SignedIn(me), friends, found, ..SocialView::default() }
    }

    #[test]
    fn signed_out_or_without_a_name_there_is_no_list() {
        assert_eq!(model(&SocialView::default()).gate, Gate::SignedOut);
        let mut view = signed_in(Load::Idle, Load::Idle);
        if let Account::SignedIn(me) = &mut view.account {
            me.profile = None;
        }
        assert_eq!(model(&view).gate, Gate::NoProfile);
    }

    #[test]
    fn found_people_then_requests_then_friends_with_what_they_play() {
        let playing = NowPlaying { track: fixtures::track(1), position_ms: 0, paused: false };
        let friends = Friends {
            friends: vec![Friend { profile: person("luca"), now_playing: Some(playing) }],
            incoming: vec![person("giulia")],
            outgoing: vec![person("paolo")],
        };
        let found: Arc<[Profile]> =
            Arc::from(vec![person("luca"), person("lucia"), person("psy"), person("giulia")]);
        let mut view = signed_in(Load::Ready(Arc::new(friends)), Load::Ready(found));
        view.pending = vec!["giulia".to_owned()];
        let model = model(&view);
        assert_eq!(model.me, "Sei @psy");
        assert_eq!(
            model.rows,
            vec![
                Row::Heading(Section::People, 4),
                Row::Person(0),
                Row::Person(1),
                Row::Person(2),
                Row::Person(3),
                Row::Heading(Section::Requests, 1),
                Row::Person(4),
                Row::Heading(Section::Waiting, 1),
                Row::Person(5),
                Row::Heading(Section::Friends, 1),
                Row::Person(6),
            ]
        );
        let actions: Vec<Actions> = model.people.iter().map(|p| p.actions).collect();
        assert_eq!(
            actions,
            vec![
                Actions::Friends,
                Actions::Add,
                Actions::None,
                Actions::Accept,
                Actions::AcceptReject,
                Actions::Withdraw,
                Actions::None
            ]
        );
        assert!(model.people[3].busy && model.people[4].busy, "una richiesta in volo spegne i pulsanti");
        assert!(
            matches!(model.people[6].status, Status::Live(_)),
            "l'amico che ascolta ha il punto dal vivo"
        );
    }

    #[test]
    fn nobody_at_all_is_the_empty_state_and_a_search_shows_its_state() {
        let empty = signed_in(Load::Ready(Arc::new(Friends::default())), Load::Idle);
        assert!(model(&empty).rows.is_empty());
        let searching = signed_in(Load::Ready(Arc::new(Friends::default())), Load::Loading);
        assert_eq!(
            model(&searching).rows,
            vec![
                Row::Heading(Section::People, 0),
                Row::Note(Line::Searching),
                Row::Heading(Section::Friends, 0),
                Row::Note(Line::NoFriends)
            ]
        );
        let none = signed_in(Load::Ready(Arc::new(Friends::default())), Load::Ready(Arc::from(Vec::new())));
        assert_eq!(model(&none).rows[1], Row::Note(Line::NoneFound));
        let failed = signed_in(Load::Failed(AppError::Network), Load::Idle);
        let failed = model(&failed);
        assert_eq!(failed.rows, vec![Row::Retry]);
        assert!(failed.error.is_some());
        assert_eq!(model(&signed_in(Load::Loading, Load::Idle)).rows, vec![Row::Note(Line::Loading)]);
    }
}
