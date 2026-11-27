//! «Jam» (`Page::Jam`): listening together, as in the Iced client (`client/src/ui/pages/jam.rs`) and the
//! art direction («Jam: the code in `TYPE_NUMERIC_PIXEL` with a copy button, members as avatars in a row,
//! live badge `LIVE` while connected»). A sidebar page between Scaricati and Amici.
//!
//! Without a jam: the friends' invitations (Entra, Ignora), then the «Ascolta insieme» card with the name to
//! show in the room, «Crea una jam» and joining with a code (six letters or digits, filtered and upper-cased
//! while typing; «Entra» lights up only with a valid code, `jam::valid_code`). If the jam closed on its own
//! the card says so in `TEXT_ERROR` under the buttons, with «Riprova». The core has no «reconnecting» state:
//! a lost link closes the jam (`JamEvent::Closed`) and the page comes back here.
//!
//! Connecting: the loader and «Connessione in corso…», with «Esci dalla jam». In the room: the code in the
//! pixel face with the copy button and the `LIVE` badge, who is there as avatars in a row, then the people
//! (`PersonRow`, «Tu» marks you) and the friends to invite (Scegli / Scelto, «Invia l'invito»). Everything
//! sits at the frame margins and enters with the page-enter move.
//!
//! Rows and texts are a pure function of the jam and social views (`model`, `rows`), rebuilt when the core
//! publishes either; the page redraws only if the result changes. No timer: an idle jam wakes nothing.

use gpui::{
    AnyElement, AppContext, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement,
    IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString, Styled, Subscription,
    WeakEntity, Window, div, list, px,
};
use psytify_core::command::{JamCmd, SocialCmd};
use psytify_core::jam::{self, CODE_LEN, MAX_MEMBERS, MAX_NAME_CHARS};
use psytify_core::snapshot::{Account, JamEvent, JamPhase, JamView, SocialEvent, SocialView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::{SEARCH_FIELD_MAX_W, TRACK_ROW_H, cover::ART_ROW};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_HEADING, TYPE_META};

use super::PageEvent;
use super::page_frame::PageFrame;
use super::people::{line_chars, status};
use crate::components::badge::{Badge, BadgeKind};
use crate::components::button::Button;
use crate::components::card::Card;
use crate::components::frame::Frame;
use crate::components::icon_button::IconButton;
use crate::components::indicators::LoadingIndicator;
use crate::components::person_row::{PersonRow, Status};
use crate::components::search_field::SearchField;
use crate::components::section::{PageTitle, note_at, section_heading_at};
use crate::components::tile::Cover;
use crate::components::tooltip::with_tooltip;
use crate::core_model::{Changed, CoreModel};
use crate::notices::Notice;
use crate::theme::{TypeStyle, col, dp};

/// Il filtro del codice mentre si scrive: lettere e cifre ASCII, in maiuscolo (come il client Iced). Il
/// campo ne tiene al più [`CODE_LEN`].
pub fn code_chars(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

/// Dove si è con la jam.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Idle,
    Connecting,
    Open,
}

/// Una persona in stanza.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MemberLine {
    name: SharedString,
    you: bool,
}

/// L'invito di un amico, con i testi pronti.
#[derive(Clone, Debug, PartialEq, Eq)]
struct InviteLine {
    code: String,
    name: SharedString,
    line: SharedString,
}

/// Un amico da invitare.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FriendLine {
    handle: String,
    name: SharedString,
    avatar: Option<SharedString>,
    status: Status,
}

/// Gli amici da invitare, come li può mostrare la pagina.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Invitable {
    /// Senza accesso o senza nome utente.
    #[default]
    SignedOut,
    Loading,
    Ready(Vec<FriendLine>),
}

/// L'errore che ha chiuso la jam, se non lo si è già lasciato.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failure {
    seq: u64,
    text: SharedString,
    /// «Riprova» ha senso: un codice sbagliato si corregge, non si riprova.
    retry: bool,
}

/// Tutto ciò che la pagina mostra.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Model {
    stage: Stage,
    /// Il codice della stanza (vuoto finché la stanza non lo dice).
    code: SharedString,
    members: Vec<MemberLine>,
    /// «3 di 8».
    count: SharedString,
    invites: Vec<InviteLine>,
    friends: Invitable,
    /// Gli inviti stanno partendo.
    inviting: bool,
    failure: Option<Failure>,
}

/// Le frasi di una riga di stato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    SignIn,
    Loading,
    NoFriends,
}

impl Line {
    fn text(self) -> &'static str {
        match self {
            Self::SignIn => "Accedi con un profilo per invitare i tuoi amici.",
            Self::Loading => "Carico i tuoi amici…",
            Self::NoFriends => "Non hai ancora amici da invitare: trovali dalla pagina Amici.",
        }
    }
}

/// Una riga della lista, nell'ordine in cui si disegna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// «Inviti» e quanti.
    InvitesHeading(usize),
    Invite(usize),
    /// La scheda «Ascolta insieme»: nome, crea, codice, entra.
    Start,
    /// La scheda della stanza: codice, `LIVE`, avatar (o il loader mentre si collega).
    Room,
    /// «Persone» e «N di 8».
    MembersHeading,
    Member(usize),
    /// «Invita amici» e quanti se ne sono scelti.
    InviteHeading,
    Friend(usize),
    Note(Line),
    /// «Invia l'invito».
    Send,
}

/// Le persone e i testi della pagina, dalle due viste. Pura. `dismissed` è l'ultimo evento della jam già
/// lasciato (scrivendo o ripartendo): un errore più vecchio non si mostra più.
fn model(jam: &JamView, social: &SocialView, dismissed: u64) -> Model {
    let stage = match jam.phase {
        JamPhase::Idle => Stage::Idle,
        JamPhase::Connecting => Stage::Connecting,
        JamPhase::Open => Stage::Open,
    };
    let members: Vec<MemberLine> = jam
        .members
        .iter()
        .map(|member| MemberLine { name: member.name.clone().into(), you: jam.you == Some(member.id) })
        .collect();
    let count = format!("{} di {MAX_MEMBERS}", members.len()).into();
    let invites = social
        .jam_invites
        .iter()
        .map(|invite| InviteLine {
            code: invite.code.clone(),
            name: invite.from_name.clone().into(),
            line: format!("@{} ti invita in una jam", invite.from_handle).into(),
        })
        .collect();
    // Gli amici da invitare servono solo in stanza: fuori, la loro presenza che cambia non ridisegna niente.
    let friends = match (&social.account, social.friends.ready()) {
        _ if stage != Stage::Open => Invitable::default(),
        (Account::SignedIn(me), Some(friends)) if me.profile.is_some() => Invitable::Ready(
            friends
                .friends
                .iter()
                .map(|friend| FriendLine {
                    handle: friend.profile.handle.clone(),
                    name: friend.profile.display_name.clone().into(),
                    avatar: friend.profile.avatar_url.clone().map(Into::into),
                    status: status(&friend.profile.handle, friend.now_playing.as_ref()),
                })
                .collect(),
        ),
        (Account::SignedIn(_), None) => Invitable::Loading,
        _ => Invitable::SignedOut,
    };
    let failure = (stage == Stage::Idle)
        .then(|| jam.events.back())
        .flatten()
        .filter(|raised| raised.seq > dismissed && ends_the_jam(&raised.event))
        .map(|raised| Failure {
            seq: raised.seq,
            text: raised.event.message().into(),
            retry: !matches!(raised.event, JamEvent::BadCode),
        });
    Model {
        stage,
        code: jam.code.clone().unwrap_or_default().into(),
        members,
        count,
        invites,
        friends,
        inviting: social.inviting,
        failure,
    }
}

/// Un evento che lascia la pagina senza jam e merita una riga sotto i pulsanti (oltre all'avviso).
fn ends_the_jam(event: &JamEvent) -> bool {
    matches!(event, JamEvent::BadCode | JamEvent::Rejected(_) | JamEvent::Closed(_))
}

/// Le righe della pagina. Pura.
fn rows(model: &Model) -> Vec<Row> {
    let mut rows = Vec::new();
    match model.stage {
        Stage::Idle => {
            if !model.invites.is_empty() {
                rows.push(Row::InvitesHeading(model.invites.len()));
                rows.extend((0..model.invites.len()).map(Row::Invite));
            }
            rows.push(Row::Start);
        }
        Stage::Connecting => rows.push(Row::Room),
        Stage::Open => {
            rows.extend([Row::Room, Row::MembersHeading]);
            rows.extend((0..model.members.len()).map(Row::Member));
            rows.push(Row::InviteHeading);
            match &model.friends {
                Invitable::SignedOut => rows.push(Row::Note(Line::SignIn)),
                Invitable::Loading => rows.push(Row::Note(Line::Loading)),
                Invitable::Ready(friends) if friends.is_empty() => rows.push(Row::Note(Line::NoFriends)),
                Invitable::Ready(friends) => {
                    rows.extend((0..friends.len()).map(Row::Friend));
                    rows.push(Row::Send);
                }
            }
        }
    }
    rows
}

/// L'ultimo modo in cui si è provato ad avviare una jam: «Riprova» rifà quello.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attempt {
    Create,
    Join,
}

/// Il comando di «Riprova», con i campi di adesso. Senza un tentativo noto (la jam l'ha avviata un altro
/// frontend) si entra se il codice è valido, altrimenti si crea.
fn retry(attempt: Option<Attempt>, name: &str, code: &str) -> JamCmd {
    let attempt = attempt.unwrap_or(if jam::valid_code(code) { Attempt::Join } else { Attempt::Create });
    match attempt {
        Attempt::Create => JamCmd::Create { name: name.to_owned() },
        Attempt::Join => JamCmd::Join { code: code.to_owned(), name: name.to_owned() },
    }
}

/// «Entra» su un invito: l'invito si toglie (anche dal server) e si entra nella stanza; se si è già in una
/// jam si resta lì, con l'avviso del client Iced.
fn accept(code: &str, active: bool, name: &str) -> Result<(SocialCmd, JamCmd), Notice> {
    if active {
        return Err(Notice::info("Sei già in una jam: esci prima da quella"));
    }
    Ok((
        SocialCmd::DismissInvite(code.to_owned()),
        JamCmd::Join { code: code.to_owned(), name: name.to_owned() },
    ))
}

/// Sceglie un amico da invitare o lo toglie dalla scelta; non più dei posti della stanza.
fn toggle(chosen: &mut Vec<String>, handle: &str) {
    if let Some(at) = chosen.iter().position(|h| h == handle) {
        chosen.remove(at);
    } else if chosen.len() < MAX_MEMBERS {
        chosen.push(handle.to_owned());
    }
}

/// L'invito agli amici scelti, se si può mandare.
fn invite(chosen: &[String], inviting: bool) -> Option<SocialCmd> {
    (!chosen.is_empty() && !inviting).then(|| SocialCmd::InviteToJam(chosen.to_vec()))
}

/// «N scelti» (vuoto se nessuno).
fn chosen_label(chosen: usize) -> SharedString {
    match chosen {
        0 => SharedString::default(),
        1 => "1 scelto".into(),
        n => format!("{n} scelti").into(),
    }
}

pub struct JamPage {
    core: Entity<CoreModel>,
    model: Model,
    rows: Vec<Row>,
    /// Gli amici scelti per l'invito (nomi utente); si dimenticano a jam finita o a inviti partiti.
    chosen: Vec<String>,
    name: Entity<SearchField>,
    code: Entity<SearchField>,
    /// Il codice scritto è valido: «Entra» è acceso.
    code_ok: bool,
    attempt: Option<Attempt>,
    /// L'ultimo evento della jam lasciato alle spalle (vedi `model`).
    dismissed: u64,
    /// L'ultimo evento sociale letto (per «inviti mandati», che svuota la scelta).
    social_seen: u64,
    list: ListState,
    frame: PageFrame,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for JamPage {}

/// Un campo del modulo: 40 dp, senza icone; ogni tasto e Invio tornano alla pagina dopo che il campo ha
/// finito il suo aggiornamento (`App::defer`: il campo è ancora in prestito mentre chiama).
fn field(
    page: &WeakEntity<JamPage>,
    id: &'static str,
    placeholder: &'static str,
    submit: Option<Attempt>,
    cx: &mut Context<JamPage>,
    shape: impl FnOnce(SearchField) -> SearchField,
) -> Entity<SearchField> {
    let (edited, submitted) = (page.clone(), page.clone());
    cx.new(|cx| {
        let field =
            shape(SearchField::new(id, placeholder, cx).plain().large()).on_change(move |_, _, cx| {
                let page = edited.clone();
                cx.defer(move |cx| {
                    page.update(cx, |page, cx| page.edited(cx)).ok();
                });
            });
        match submit {
            Some(attempt) => field.on_submit(move |_, _, cx| {
                let page = submitted.clone();
                cx.defer(move |cx| {
                    page.update(cx, |page, cx| page.start(attempt, cx)).ok();
                });
            }),
            None => field,
        }
    })
}

impl JamPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let page = cx.entity().downgrade();
        let name = field(&page, "jam-name", "Il tuo nome (facoltativo)", Some(Attempt::Create), cx, |f| {
            f.sanitize(line_chars).max_chars(MAX_NAME_CHARS)
        });
        let code = field(&page, "jam-code", "Codice di 6 caratteri", Some(Attempt::Join), cx, |f| {
            f.sanitize(code_chars).max_chars(CODE_LEN)
        });
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.jam || changed.social {
                this.core_changed(&core, cx);
            }
        });
        let (jam, social) = (core.read(cx).jam_view(), core.read(cx).social_view());
        let model = model(&jam, &social, 0);
        let rows = rows(&model);
        let list = ListState::new(rows.len(), ListAlignment::Top, px(4.0 * TRACK_ROW_H));
        Self {
            core,
            model,
            rows,
            chosen: Vec::new(),
            name,
            code,
            code_ok: false,
            attempt: None,
            dismissed: 0,
            social_seen: social.events.back().map_or(0, |raised| raised.seq),
            list,
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

    fn core_changed(&mut self, core: &Entity<CoreModel>, cx: &mut Context<Self>) {
        let (jam, social) = (core.read(cx).jam_view(), core.read(cx).social_view());
        let mut dirty = false;
        // Inviti partiti: la scelta si svuota (come nel client Iced).
        let seen = self.social_seen;
        if social
            .events
            .iter()
            .any(|raised| raised.seq > seen && matches!(raised.event, SocialEvent::InvitesSent(_)))
            && !self.chosen.is_empty()
        {
            self.chosen.clear();
            dirty = true;
        }
        self.social_seen = social.events.back().map_or(seen, |raised| raised.seq.max(seen));
        if !jam.is_active() && !self.chosen.is_empty() {
            self.chosen.clear();
            dirty = true;
        }
        dirty |= self.rebuild(&jam, &social);
        if dirty {
            cx.notify();
        }
    }

    /// Rifà il modello; `true` se è cambiato (e allora la lista si rimisura).
    fn rebuild(&mut self, jam: &JamView, social: &SocialView) -> bool {
        let model = model(jam, social, self.dismissed);
        if model == self.model {
            return false;
        }
        let same_stage = model.stage == self.model.stage;
        self.model = model;
        let old = self.rows.len();
        self.rows = rows(&self.model);
        if same_stage {
            // Lo stesso stato (un amico che cambia brano, una persona che entra): la lista si rimisura e
            // resta dove la si era scorsa.
            let top = self.list.logical_scroll_top();
            self.list.splice(0..old, self.rows.len());
            if top.item_ix < self.rows.len() {
                self.list.scroll_to(top);
            }
        } else {
            self.list.reset(self.rows.len());
        }
        true
    }

    fn rebuild_now(&mut self, cx: &mut Context<Self>) {
        let (jam, social) = (self.core.read(cx).jam_view(), self.core.read(cx).social_view());
        if self.rebuild(&jam, &social) {
            cx.notify();
        }
    }

    /// Si è scritto in un campo: «Entra» segue il codice e l'errore mostrato si lascia.
    fn edited(&mut self, cx: &mut Context<Self>) {
        let ok = jam::valid_code(self.code.read(cx).text());
        if ok != self.code_ok {
            self.code_ok = ok;
            cx.notify();
        }
        if let Some(failure) = &self.model.failure {
            self.dismissed = failure.seq;
            self.rebuild_now(cx);
        }
    }

    /// Il nome scritto (vuoto: il core usa quello del profilo).
    fn typed_name(&self, cx: &gpui::App) -> String {
        self.name.read(cx).text().trim().to_owned()
    }

    /// «Crea una jam», «Entra» o Invio in un campo.
    fn start(&mut self, attempt: Attempt, cx: &mut Context<Self>) {
        if self.model.stage != Stage::Idle {
            return;
        }
        // Un codice incompleto (Invio nel campo: il pulsante è spento) lo rifiuta il core, con il suo
        // avviso e la riga sotto i pulsanti, come nel client Iced.
        let code = self.code.read(cx).text().to_owned();
        self.attempt = Some(attempt);
        let command = retry(Some(attempt), &self.typed_name(cx), &code);
        self.send(command, cx);
    }

    /// Manda un comando che avvia la jam: l'errore di prima si lascia.
    fn send(&mut self, command: JamCmd, cx: &mut Context<Self>) {
        if let Some(failure) = &self.model.failure {
            self.dismissed = failure.seq;
        }
        self.core.read(cx).jam(command);
        self.rebuild_now(cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let code = self.code.read(cx).text().to_owned();
        let command = retry(self.attempt, &self.typed_name(cx), &code);
        self.send(command, cx);
    }

    fn accept_invite(&mut self, code: &str, cx: &mut Context<Self>) {
        let active = self.model.stage != Stage::Idle;
        match accept(code, active, &self.typed_name(cx)) {
            Ok((dismiss, join)) => {
                self.core.read(cx).social(dismiss);
                self.attempt = Some(Attempt::Join);
                self.send(join, cx);
            }
            Err(notice) => cx.emit(PageEvent::Notice(notice)),
        }
    }

    fn toggle_friend(&mut self, handle: &str, cx: &mut Context<Self>) {
        toggle(&mut self.chosen, handle);
        cx.notify();
    }

    fn send_invites(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = invite(&self.chosen, self.model.inviting) {
            self.core.read(cx).social(command);
        }
    }

    fn copy_code(&mut self, cx: &mut Context<Self>) {
        if self.model.code.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.model.code.to_string()));
        cx.emit(PageEvent::Notice(Notice::info("Codice copiato")));
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = self.row(ix, window, cx);
        // One step per block: a heading or a card starts one.
        let section = 1 + self
            .rows
            .iter()
            .take(ix + 1)
            .filter(|row| {
                matches!(
                    row,
                    Row::InvitesHeading(_)
                        | Row::Start
                        | Row::Room
                        | Row::MembersHeading
                        | Row::InviteHeading
                )
            })
            .count();
        self.frame.wrap(element, section, window)
    }

    fn row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(ix).copied() else { return div().into_any_element() };
        let margin = self.frame.margin();
        match row {
            Row::InvitesHeading(count) => {
                section_heading_at("Inviti".into(), count.to_string().into(), margin).into_any_element()
            }
            Row::Invite(ix) => self.invite_row(ix, cx),
            Row::Start => self.start_card(cx),
            Row::Room => self.room_card(window, cx),
            Row::MembersHeading => {
                section_heading_at("Persone".into(), self.model.count.clone(), margin).into_any_element()
            }
            Row::Member(ix) => match self.model.members.get(ix) {
                Some(member) => {
                    let line = if member.you { "Tu" } else { "Nella jam" };
                    self.frame.row(PersonRow::new(
                        ("jam-member", ix),
                        member.name.clone(),
                        Status::Plain(line.into()),
                    ))
                }
                None => div().into_any_element(),
            },
            Row::InviteHeading => {
                section_heading_at("Invita amici".into(), chosen_label(self.chosen.len()), margin)
                    .into_any_element()
            }
            Row::Friend(ix) => self.friend_row(ix, cx),
            Row::Note(line) => {
                note_at(div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(line.text()), margin)
                    .into_any_element()
            }
            Row::Send => note_at(
                Button::new("jam-invite-send", "Invia l'invito")
                    .primary()
                    .icon(sprite::ICON_USER_PLUS)
                    .loading(self.model.inviting)
                    .disabled(self.chosen.is_empty() || self.model.inviting)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send_invites(cx))),
                margin,
            )
            .into_any_element(),
        }
    }

    fn invite_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(invite) = self.model.invites.get(ix) else { return div().into_any_element() };
        let (join, dismiss) = (invite.code.clone(), invite.code.clone());
        let row = PersonRow::new(("jam-invite", ix), invite.name.clone(), Status::Plain(invite.line.clone()))
            .action(
                Button::new(("jam-invite-join", ix), "Entra")
                    .secondary()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.accept_invite(&join, cx))),
            )
            .action(Button::new(("jam-invite-dismiss", ix), "Ignora").ghost().on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).social(SocialCmd::DismissInvite(dismiss.clone()));
                },
            )));
        self.frame.row(row)
    }

    fn friend_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Invitable::Ready(friends) = &self.model.friends else { return div().into_any_element() };
        let Some(friend) = friends.get(ix) else { return div().into_any_element() };
        let chosen = self.chosen.contains(&friend.handle);
        let handle = friend.handle.clone();
        let button = if chosen {
            Button::new(("jam-friend", ix), "Scelto").primary().icon(sprite::ICON_CHECK)
        } else {
            Button::new(("jam-friend", ix), "Scegli").secondary()
        };
        self.frame.row(
            PersonRow::new(("jam-friend-row", ix), friend.name.clone(), friend.status.clone())
                .avatar(friend.avatar.clone())
                .action(button.on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_friend(&handle, cx)),
                )),
        )
    }

    /// The card to start a jam: name, «Crea una jam», code and «Entra», the error and «Riprova».
    fn start_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body =
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_4))
                .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child("Ascolta insieme"))
                .child(div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(format!(
                    "Fino a {MAX_MEMBERS} persone ascoltano la stessa coda. Ognuno suona dal proprio \
                 dispositivo: la jam condivide solo il brano e la posizione."
                )))
                .child(self.name.clone())
                .child(
                    div().child(Button::new("jam-create", "Crea una jam").primary().on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.start(Attempt::Create, cx)),
                    )),
                )
                .child(
                    div()
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_TERTIARY))
                        .child(format!("Per entrare serve il codice di {CODE_LEN} caratteri.")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(SPACE_3))
                        .child(div().flex_1().child(self.code.clone()))
                        .child(
                            Button::new("jam-join", "Entra")
                                .secondary()
                                .large()
                                .disabled(!self.code_ok)
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.start(Attempt::Join, cx)),
                                ),
                        ),
                );
        if let Some(failure) = &self.model.failure {
            let mut line = div().flex().items_center().gap(dp(SPACE_5)).child(
                div().flex_1().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(failure.text.clone()),
            );
            if failure.retry {
                line = line.child(
                    Button::new("jam-retry", "Riprova")
                        .secondary()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.retry(cx))),
                );
            }
            body = body.child(line);
        }
        padded(Card::new().child(body), self.frame.margin()).into_any_element()
    }

    /// The room card: «Codice» with the `LIVE` badge, the code in the pixel face and «Copia», the line, the
    /// avatars in a row. While connecting: the loader and «Connessione in corso…».
    fn room_card(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.model.stage == Stage::Connecting {
            return padded(
                Card::new().child(
                    div().flex().items_center().gap(dp(SPACE_4)).child(LoadingIndicator::new()).child(
                        div()
                            .type_role(TYPE_BODY)
                            .text_color(col(TEXT_SECONDARY))
                            .child("Connessione in corso…"),
                    ),
                ),
                self.frame.margin(),
            )
            .into_any_element();
        }
        let code =
            if self.model.code.is_empty() { SharedString::from("…") } else { self.model.code.clone() };
        let avatars = self.model.members.iter().enumerate().map(|(ix, member)| {
            let name = if member.you { format!("{} (tu)", member.name) } else { member.name.to_string() };
            with_tooltip(
                div().id(("jam-avatar", ix)).flex_none().child(Cover::new(ART_ROW).circle()),
                name,
                None,
            )
        });
        let body = div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(div().type_role(TYPE_META).text_color(col(TEXT_SECONDARY)).child("Codice"))
                    .child(Badge::new("LIVE", BadgeKind::Live)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(div().text_color(col(TEXT_PRIMARY)).pixel_type(true).child(code))
                    .child(
                        IconButton::new("jam-copy", sprite::ICON_COPY)
                            .tooltip("Copia il codice", None)
                            .disabled(self.model.code.is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy_code(cx))),
                    ),
            )
            .child(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Condividilo con chi vuoi far entrare. Vale finché la jam è aperta."),
            )
            .child(
                div().flex().items_center().gap(dp(SPACE_2)).children(avatars).child(
                    div()
                        .pl(dp(SPACE_2))
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_TERTIARY))
                        .child(self.model.count.clone()),
                ),
            );
        padded(Card::new().child(body), self.frame.margin()).into_any_element()
    }
}

/// A card of the page at the frame margin, at most as wide as a search field.
fn padded(card: Card, margin: f32) -> gpui::Div {
    div().w_full().px(dp(margin)).py(dp(SPACE_3)).child(div().max_w(dp(SEARCH_FIELD_MAX_W)).child(card))
}

impl Render for JamPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, Some(&self.list));
        let meta = match self.model.stage {
            Stage::Idle => "Ascolta insieme ad altri",
            Stage::Connecting => "Connessione in corso…",
            Stage::Open => "Collegato alla stanza",
        };
        let mut title = PageTitle::screen("Jam").margin(self.frame.margin()).meta(meta);
        if self.model.stage != Stage::Idle {
            title =
                title.action(Button::new("jam-leave", "Esci dalla jam").secondary().on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.core.read(cx).jam(JamCmd::Leave)),
                ));
        }
        let list = list(
            self.list.clone(),
            cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
        )
        .size_full();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame))
            .child(self.frame.wrap(title.into_any_element(), 0, window))
            .child(div().flex_1().min_h_0().child(list))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use psytify_core::api::Me;
    use psytify_core::api::social::{Friend, Friends, JamInvite, MyProfile, Profile};
    use psytify_core::catalog::Load;
    use psytify_core::jam::Member;
    use psytify_core::snapshot::JamRaised;
    use uuid::Uuid;

    use super::*;

    fn signed_in(friends: Load<Arc<Friends>>) -> SocialView {
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
        SocialView { account: Account::SignedIn(me), friends, ..SocialView::default() }
    }

    fn friend(handle: &str) -> Friend {
        Friend {
            profile: Profile {
                handle: handle.to_owned(),
                display_name: handle.to_uppercase(),
                avatar_url: None,
            },
            now_playing: None,
        }
    }

    fn open(members: &[&str], you: usize) -> JamView {
        let members: Vec<Member> = members
            .iter()
            .enumerate()
            .map(|(ix, name)| Member { id: Uuid::from_u128(ix as u128 + 1), name: (*name).to_owned() })
            .collect();
        JamView {
            phase: JamPhase::Open,
            code: Some("K7Q2MX".to_owned()),
            you: Some(Uuid::from_u128(you as u128 + 1)),
            members: Arc::from(members),
            events: Arc::default(),
        }
    }

    fn raised(seq: u64, event: JamEvent) -> JamView {
        JamView { events: Arc::new(VecDeque::from([JamRaised { seq, event }])), ..JamView::default() }
    }

    #[test]
    fn the_code_keeps_letters_and_digits_in_upper_case() {
        assert_eq!(code_chars("k7q-2 mx"), "K7Q2MX");
        assert_eq!(code_chars("àb!c\n"), "BC");
        assert!(jam::valid_code(&code_chars("k7q2mx")));
        assert!(!jam::valid_code(&code_chars("k7q2m")));
    }

    #[test]
    fn without_a_jam_the_invites_come_before_the_start_card() {
        let mut social = signed_in(Load::Idle);
        assert_eq!(rows(&model(&JamView::default(), &social, 0)), vec![Row::Start]);
        social.jam_invites = Arc::from(vec![JamInvite {
            code: "K7Q2MX".to_owned(),
            from_handle: "luca".to_owned(),
            from_name: "Luca".to_owned(),
        }]);
        let model = model(&JamView::default(), &social, 0);
        assert_eq!(rows(&model), vec![Row::InvitesHeading(1), Row::Invite(0), Row::Start]);
        assert_eq!(model.invites[0].line, "@luca ti invita in una jam");
        // Già in una jam gli inviti non si mostrano (come nel client Iced).
        let model = super::model(&open(&["Psy"], 0), &social, 0);
        assert!(!rows(&model).contains(&Row::Invite(0)));
    }

    #[test]
    fn connecting_shows_only_the_room_card() {
        let connecting = JamView { phase: JamPhase::Connecting, ..JamView::default() };
        let model = model(&connecting, &SocialView::default(), 0);
        assert_eq!(model.stage, Stage::Connecting);
        assert_eq!(rows(&model), vec![Row::Room]);
    }

    #[test]
    fn in_a_jam_the_members_mark_you_and_the_friends_can_be_invited() {
        let friends = Friends { friends: vec![friend("luca"), friend("sara")], ..Friends::default() };
        let social = signed_in(Load::Ready(Arc::new(friends)));
        let model = model(&open(&["Luca", "Psy", "Sara"], 1), &social, 0);
        assert_eq!(model.count, "3 di 8");
        assert_eq!(model.code, "K7Q2MX");
        let you: Vec<bool> = model.members.iter().map(|m| m.you).collect();
        assert_eq!(you, vec![false, true, false]);
        assert_eq!(
            rows(&model),
            vec![
                Row::Room,
                Row::MembersHeading,
                Row::Member(0),
                Row::Member(1),
                Row::Member(2),
                Row::InviteHeading,
                Row::Friend(0),
                Row::Friend(1),
                Row::Send
            ]
        );
        // Senza accesso, in caricamento o senza amici: una riga che lo dice, niente «Invia».
        let tail = |social: &SocialView| rows(&super::model(&open(&["Psy"], 0), social, 0)).pop();
        assert_eq!(tail(&SocialView::default()), Some(Row::Note(Line::SignIn)));
        assert_eq!(tail(&signed_in(Load::Loading)), Some(Row::Note(Line::Loading)));
        assert_eq!(tail(&signed_in(Load::Ready(Arc::default()))), Some(Row::Note(Line::NoFriends)));
    }

    #[test]
    fn an_error_that_ended_the_jam_shows_until_left_behind() {
        let social = SocialView::default();
        let closed = raised(4, JamEvent::Closed("Connessione persa".to_owned()));
        let failure = model(&closed, &social, 0).failure.expect("la jam chiusa si dice");
        assert_eq!((failure.seq, failure.text.as_ref(), failure.retry), (4, "Connessione persa", true));
        assert!(model(&closed, &social, 4).failure.is_none(), "lasciato scrivendo o ripartendo");
        let bad = model(&raised(2, JamEvent::BadCode), &social, 0).failure.expect("codice sbagliato");
        assert!(!bad.retry, "un codice sbagliato si corregge, non si riprova");
        assert!(model(&raised(3, JamEvent::Left), &social, 0).failure.is_none());
        assert!(model(&raised(3, JamEvent::TracksMissing), &social, 0).failure.is_none());
        // In stanza un rifiuto è solo un avviso.
        let mut busy = open(&["Psy"], 0);
        busy.events = raised(5, JamEvent::Rejected("Troppi cambiamenti".to_owned())).events;
        assert!(model(&busy, &social, 0).failure.is_none());
    }

    #[test]
    fn retry_repeats_the_last_attempt_with_the_fields_of_now() {
        let create = retry(Some(Attempt::Create), "Psy", "K7Q2MX");
        assert!(matches!(create, JamCmd::Create { name } if name == "Psy"));
        let join = retry(Some(Attempt::Join), "", "K7Q2MX");
        assert!(matches!(join, JamCmd::Join { code, name } if code == "K7Q2MX" && name.is_empty()));
        // Senza un tentativo noto: entra se il codice è valido, altrimenti crea.
        assert!(matches!(retry(None, "", "K7Q2MX"), JamCmd::Join { .. }));
        assert!(matches!(retry(None, "", "K7Q"), JamCmd::Create { .. }));
    }

    #[test]
    fn accepting_an_invite_dismisses_it_and_joins_unless_already_in_a_jam() {
        let (dismiss, join) = accept("K7Q2MX", false, "Psy").expect("si entra");
        assert!(matches!(dismiss, SocialCmd::DismissInvite(code) if code == "K7Q2MX"));
        assert!(matches!(join, JamCmd::Join { code, name } if code == "K7Q2MX" && name == "Psy"));
        let refused = accept("K7Q2MX", true, "").expect_err("già in una jam");
        assert_eq!(refused, Notice::info("Sei già in una jam: esci prima da quella"));
    }

    #[test]
    fn choosing_friends_has_a_cap_and_sending_needs_someone() {
        let mut chosen = Vec::new();
        assert!(invite(&chosen, false).is_none());
        toggle(&mut chosen, "luca");
        toggle(&mut chosen, "sara");
        toggle(&mut chosen, "luca");
        assert_eq!(chosen, ["sara"]);
        assert!(matches!(invite(&chosen, false), Some(SocialCmd::InviteToJam(h)) if h == ["sara"]));
        assert!(invite(&chosen, true).is_none(), "non mentre gli inviti partono");
        for n in 0..20 {
            toggle(&mut chosen, &format!("amico{n}"));
        }
        assert_eq!(chosen.len(), MAX_MEMBERS, "non più dei posti della stanza");
        assert_eq!(chosen_label(0), "");
        assert_eq!(chosen_label(1), "1 scelto");
        assert_eq!(chosen_label(3), "3 scelti");
    }
}
