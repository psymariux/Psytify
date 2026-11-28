//! «Amici», as the GPUI page (`desktop/src/app/friends_page.rs`) and the phone: the people found
//! by username (the field is the app's, `PsytifyCore::search_users`), the requests that arrived
//! (Accetta, Rifiuta), the ones sent (Ritira), then the friends with what they listen to («Sta
//! ascoltando …» with the live dot). A tap on a person opens the profile. While a request for someone
//! is on its way (`SocialView::pending`) their buttons are off.
//!
//! Signed out, without a username or with nobody at all, the page says so instead of a list. It asks
//! for `RefreshFriends` each time it opens.

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::social::{Friends, NowPlaying, Profile};
use psytify_core::catalog::Load;
use psytify_core::command::SocialCmd;
use psytify_core::snapshot::{Account, SocialView};

use super::{
    Built, ButtonKind, Mark, PageAction, PageStatus, Route, Row, RowAction, RowButton, Target, Tone,
};

/// What a person's row offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Actions {
    None,
    /// Found, not a friend yet: «Aggiungi».
    Add,
    /// Found, already a friend: «Amici», off.
    Friends,
    /// Found, request already sent: «Richiesta inviata», off.
    Sent,
    /// Found, they asked: «Accetta».
    Accept,
    /// A request that arrived: «Accetta» and «Rifiuta».
    AcceptReject,
    /// A request sent: «Ritira».
    Withdraw,
}

/// «Sta ascoltando Titolo · Artista» (live) or «In pausa: …».
pub(super) fn listening(now: &NowPlaying) -> (String, bool) {
    let track = format!("{} · {}", now.track.title, now.track.artist.name);
    if now.paused { (format!("In pausa: {track}"), false) } else { (format!("Sta ascoltando {track}"), true) }
}

/// A person's line: what they listen to, or `@handle`.
pub(super) fn status(handle: &str, now: Option<&NowPlaying>) -> (String, bool) {
    now.map_or_else(|| (format!("@{handle}"), false), listening)
}

pub(super) fn row_button(action: RowAction, label: &str, kind: ButtonKind, enabled: bool) -> RowButton {
    RowButton { action, label: label.to_owned(), kind, enabled, on: false }
}

/// A person's row, keyed within its section.
pub(super) fn person(
    section: &str,
    profile: &Profile,
    (status, live): (String, bool),
    buttons: Vec<RowButton>,
) -> Row {
    Row::Person {
        key: format!("{section}:{}", profile.handle),
        avatar_url: profile.avatar_url.clone(),
        name: profile.display_name.clone(),
        status,
        live,
        buttons,
    }
}

/// The relation of a person found, from the lists of friends.
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

/// Why there is no list, or `None` when there can be one. Shared with the Jam page's invite rows.
pub(super) fn gate(view: &SocialView) -> Result<&str, PageStatus> {
    match &view.account {
        Account::SignedIn(me) => match &me.profile {
            Some(profile) => Ok(profile.handle.as_str()),
            None => Err(PageStatus::Empty {
                message: "Scegli un nome utente nel Profilo: gli amici ti trovano così.".into(),
            }),
        },
        Account::Working => Err(PageStatus::Loading),
        Account::SignedOut => Err(PageStatus::Empty {
            message: "Accedi dal Profilo per trovare i tuoi amici e vedere cosa ascoltano.".into(),
        }),
        Account::Unavailable => Err(PageStatus::Empty {
            message: "L'accesso non è configurato in questa versione dell'app.".into(),
        }),
    }
}

pub(super) fn build(snapshot: &Snapshot) -> Built {
    let view = &snapshot.social;
    let mut page = Built::titled("Amici");
    page.action(PageAction::Retry, Target::Send(Command::Social(SocialCmd::RefreshFriends)));
    let own = match gate(view) {
        Ok(own) => own.to_owned(),
        Err(status) => {
            page.status = status;
            return page;
        }
    };
    page.status = PageStatus::Ready;
    let friends = view.friends.ready().map(Arc::as_ref);
    let add = |page: &mut Built, section: &str, profile: &Profile, line: (String, bool), actions: Actions| {
        let enabled = !view.pending.contains(&profile.handle);
        let button = |action, label: &str, kind| row_button(action, label, kind, enabled);
        let buttons = match actions {
            Actions::None => Vec::new(),
            Actions::Add => vec![button(RowAction::Add, "Aggiungi", ButtonKind::Secondary)],
            Actions::Friends => vec![row_button(RowAction::Add, "Amici", ButtonKind::Ghost, false)],
            Actions::Sent => vec![row_button(RowAction::Add, "Richiesta inviata", ButtonKind::Ghost, false)],
            Actions::Accept => vec![button(RowAction::Add, "Accetta", ButtonKind::Secondary)],
            Actions::AcceptReject => vec![
                button(RowAction::Add, "Accetta", ButtonKind::Secondary),
                button(RowAction::Refuse, "Rifiuta", ButtonKind::Ghost),
            ],
            Actions::Withdraw => vec![button(RowAction::Refuse, "Ritira", ButtonKind::Ghost)],
        };
        let row = page.rows.len();
        for button in buttons.iter().filter(|button| button.enabled) {
            let add = button.action == RowAction::Add;
            let command = SocialCmd::FriendAction { handle: profile.handle.clone(), add };
            page.row_action(row, button.action, Target::Send(Command::Social(command)));
        }
        let open = Target::Open(Route::Profile { handle: profile.handle.clone() });
        page.push(person(section, profile, line, buttons), Mark::None, open);
    };
    match &view.found {
        Load::Idle => {}
        Load::Loading => {
            page.heading("people", "Persone", String::new());
            page.note("searching", "Cerco…".into(), Tone::Waiting);
        }
        Load::Failed(_) => {
            page.heading("people", "Persone", String::new());
            page.note("search-failed", "La ricerca non è riuscita. Riprova.".into(), Tone::Error);
        }
        Load::Ready(found) => {
            page.heading("people", "Persone", found.len().to_string());
            if found.is_empty() {
                page.note("none-found", "Nessuno con questo nome.".into(), Tone::Plain);
            }
            for profile in found.iter() {
                let actions = found_actions(&profile.handle, &own, friends);
                add(&mut page, "found", profile, (format!("@{}", profile.handle), false), actions);
            }
        }
    }
    match &view.friends {
        Load::Ready(friends) => {
            if !friends.incoming.is_empty() {
                page.heading("requests", "Richieste", friends.incoming.len().to_string());
                for profile in &friends.incoming {
                    let line = (format!("@{}", profile.handle), false);
                    add(&mut page, "incoming", profile, line, Actions::AcceptReject);
                }
            }
            if !friends.outgoing.is_empty() {
                page.heading("waiting", "In attesa", friends.outgoing.len().to_string());
                for profile in &friends.outgoing {
                    let line = ("Richiesta inviata".to_owned(), false);
                    add(&mut page, "outgoing", profile, line, Actions::Withdraw);
                }
            }
            let nobody =
                friends.friends.is_empty() && friends.incoming.is_empty() && friends.outgoing.is_empty();
            if nobody && matches!(view.found, Load::Idle) {
                page.status = PageStatus::Empty {
                    message: "Ancora nessun amico: cerca qualcuno per nome utente e mandagli una richiesta."
                        .into(),
                };
                return page;
            }
            page.heading("friends", "I tuoi amici", friends.friends.len().to_string());
            if friends.friends.is_empty() {
                let text = "Ancora nessun amico: cerca qualcuno qui sopra e mandagli una richiesta.";
                page.note("no-friends", text.into(), Tone::Plain);
            }
            for friend in &friends.friends {
                let line = status(&friend.profile.handle, friend.now_playing.as_ref());
                add(&mut page, "friend", &friend.profile, line, Actions::None);
            }
        }
        Load::Failed(error) => {
            if page.rows.is_empty() {
                page.status = PageStatus::Failed { message: error.to_string() };
            } else {
                page.note("friends-failed", format!("Gli amici non si sono caricati: {error}"), Tone::Error);
            }
        }
        Load::Idle | Load::Loading => page.note("loading", "Carico gli amici…".into(), Tone::Waiting),
    }
    page
}

/// The search field as typed: a leading `@` and the spaces around go; an empty text clears the
/// results.
pub(crate) fn search_query(text: &str) -> String {
    text.trim().trim_start_matches('@').trim().to_owned()
}
