//! «Jam», listening together, as the GPUI page (`desktop/src/app/jam_page.rs`) and the phone.
//!
//! No jam: the friends' invites (Entra, Ignora), then the card «Ascolta insieme», whose name and code
//! fields the app draws (`PsytifyCore::jam_create`, `jam_join`); if the last jam closed by itself the
//! card says why, with «Riprova». Connecting: the room card with the loader and «Esci dalla jam». In
//! the room: the code with «Copia» and `LIVE`, who is there, then the friends to invite («Scegli» /
//! «Scelto», «Invia l'invito»). The core has no «reconnecting» state: a lost link closes the jam
//! (`JamEvent::Closed`) and the page goes back to the card. No timer: a still jam wakes nothing.

use psytify_core::Snapshot;
use psytify_core::catalog::Load;
use psytify_core::command::{Command, JamCmd, SocialCmd};
use psytify_core::jam::{self, CODE_LEN, MAX_MEMBERS, MAX_NAME_CHARS};
use psytify_core::snapshot::{Account, JamEvent, JamPhase, JamView};

use super::friends::{gate, person, row_button, status};
use super::{Built, ButtonKind, Mark, PageStatus, Route, Row, RowAction, RowButton, Target, Tone, Ui};

/// The code as typed: ASCII letters and digits, upper case, at most six.
#[uniffi::export]
pub fn jam_code_chars(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).take(CODE_LEN).collect()
}

/// «Entra» lights up only with a whole code.
#[uniffi::export]
pub fn jam_code_valid(code: &str) -> bool {
    jam::valid_code(code)
}

/// The name shown in the room as typed: one line, at most the server's length.
#[uniffi::export]
pub fn jam_name_chars(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect()
}

/// How the last jam was started: «Riprova» does that again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attempt {
    Create,
    Join,
}

/// The jam form's memory in the bridge, across the page's handles: the last attempt and the last
/// jam event left behind (by typing or starting again): an older error is not shown any more.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Form {
    pub(crate) attempt: Option<Attempt>,
    pub(crate) dismissed: u64,
}

/// The command of «Riprova» with the fields as they are now. Without a known attempt (another
/// frontend started it) it joins if the code is whole, otherwise it creates.
pub(crate) fn retry(attempt: Option<Attempt>, name: &str, code: &str) -> (Attempt, JamCmd) {
    let attempt = attempt.unwrap_or(if jam::valid_code(code) { Attempt::Join } else { Attempt::Create });
    let name = name.trim().to_owned();
    let command = match attempt {
        Attempt::Create => JamCmd::Create { name },
        Attempt::Join => JamCmd::Join { code: code.to_owned(), name },
    };
    (attempt, command)
}

/// «Entra» on an invite: the invite goes (from the server too) and the room opens; already in a jam,
/// the person stays there and is told.
pub(crate) fn accept(code: &str, active: bool) -> Result<Vec<Command>, String> {
    if active {
        return Err("Sei già in una jam: esci prima da quella".to_owned());
    }
    Ok(vec![
        Command::Social(SocialCmd::DismissInvite(code.to_owned())),
        Command::Jam(JamCmd::Join { code: code.to_owned(), name: String::new() }),
    ])
}

/// Chooses a friend to invite, or drops the choice; no more than the room's seats.
pub(crate) fn toggle(chosen: &mut Vec<String>, handle: &str) {
    if let Some(at) = chosen.iter().position(|h| h == handle) {
        chosen.remove(at);
    } else if chosen.len() < MAX_MEMBERS {
        chosen.push(handle.to_owned());
    }
}

/// An event that leaves the page without a jam and earns a line under the buttons.
fn ends_the_jam(event: &JamEvent) -> bool {
    matches!(event, JamEvent::BadCode | JamEvent::Rejected(_) | JamEvent::Closed(_))
}

/// The error that closed the jam, if not left behind yet: its text and whether «Riprova» fits.
pub(crate) fn failure(view: &JamView, dismissed: u64) -> Option<(u64, String, bool)> {
    let raised = view.events.back().filter(|_| view.phase == JamPhase::Idle)?;
    (raised.seq > dismissed && ends_the_jam(&raised.event))
        .then(|| (raised.seq, raised.event.message(), !matches!(raised.event, JamEvent::BadCode)))
}

/// «N scelti» (empty if none).
fn chosen_label(chosen: usize) -> String {
    match chosen {
        0 => String::new(),
        1 => "1 scelto".to_owned(),
        n => format!("{n} scelti"),
    }
}

pub(super) fn build(snapshot: &Snapshot, ui: &Ui) -> Built {
    let (view, social) = (&snapshot.jam, &snapshot.social);
    let mut page = Built::titled("Jam");
    page.status = PageStatus::Ready;
    match view.phase {
        JamPhase::Idle => {
            if !social.jam_invites.is_empty() {
                page.heading("invites", "Inviti", social.jam_invites.len().to_string());
                for invite in social.jam_invites.iter() {
                    let row = page.rows.len();
                    page.row_action(row, RowAction::Join, Target::JoinInvite(invite.code.clone()));
                    let dismiss = Command::Social(SocialCmd::DismissInvite(invite.code.clone()));
                    page.row_action(row, RowAction::Ignore, Target::Send(dismiss));
                    let line = Row::Person {
                        key: format!("invite:{}", invite.code),
                        avatar_url: None,
                        name: invite.from_name.clone(),
                        status: format!("@{} ti invita in una jam", invite.from_handle),
                        live: false,
                        buttons: vec![
                            row_button(RowAction::Join, "Entra", ButtonKind::Secondary, true),
                            row_button(RowAction::Ignore, "Ignora", ButtonKind::Ghost, true),
                        ],
                    };
                    let open = Target::Open(Route::Profile { handle: invite.from_handle.clone() });
                    page.push(line, Mark::None, open);
                }
            }
            let failure = failure(view, ui.jam_dismissed);
            let retry = failure.as_ref().is_some_and(|(_, _, retry)| *retry);
            let row = Row::JamStart { key: "start".into(), failure: failure.map(|(_, text, _)| text), retry };
            page.push(row, Mark::None, Target::None);
        }
        JamPhase::Connecting => room(&mut page, view),
        JamPhase::Open => {
            room(&mut page, view);
            page.heading("members", "Persone", format!("{} di {MAX_MEMBERS}", view.members.len()));
            for member in view.members.iter() {
                let you = view.you == Some(member.id);
                let row = Row::Person {
                    key: format!("member:{}", member.id),
                    avatar_url: None,
                    name: member.name.clone(),
                    status: if you { "Tu".into() } else { "Nella jam".into() },
                    live: false,
                    buttons: Vec::new(),
                };
                page.push(row, Mark::None, Target::None);
            }
            invitable(&mut page, snapshot, ui);
        }
    }
    page
}

/// The room card: the code once the room said it, who is in it, and the way out.
fn room(page: &mut Built, view: &JamView) {
    let row = page.rows.len();
    page.row_action(row, RowAction::Leave, Target::Send(Command::Jam(JamCmd::Leave)));
    if let Some(code) = &view.code {
        page.row_action(row, RowAction::Copy, Target::Copy(code.clone()));
    }
    let members = view.members.iter().map(|member| member.name.clone()).collect();
    let code = view.code.clone().filter(|_| view.phase == JamPhase::Open);
    page.push(Row::JamRoom { key: "room".into(), code, members }, Mark::None, Target::None);
}

/// «Invita amici»: the friends, each to choose, then «Invia l'invito».
fn invitable(page: &mut Built, snapshot: &Snapshot, ui: &Ui) {
    let social = &snapshot.social;
    page.heading("invite", "Invita amici", chosen_label(ui.chosen.len()));
    if gate(social).is_err() {
        page.note("sign-in", "Accedi con un profilo per invitare i tuoi amici.".into(), Tone::Plain);
        return;
    }
    let friends = match (&social.account, &social.friends) {
        (Account::SignedIn(_), Load::Ready(friends)) => friends,
        _ => {
            page.note("loading", "Carico i tuoi amici…".into(), Tone::Waiting);
            return;
        }
    };
    if friends.friends.is_empty() {
        let text = "Non hai ancora amici da invitare: trovali dalla pagina Amici.";
        page.note("no-friends", text.into(), Tone::Plain);
        return;
    }
    for friend in &friends.friends {
        let handle = &friend.profile.handle;
        let on = ui.chosen.contains(handle);
        let label = if on { "Scelto" } else { "Scegli" };
        let button = RowButton { on, ..row_button(RowAction::Choose, label, ButtonKind::Secondary, true) };
        let row = page.rows.len();
        page.row_action(row, RowAction::Choose, Target::Choose(handle.clone()));
        let line = status(handle, friend.now_playing.as_ref());
        let open = Target::Open(Route::Profile { handle: handle.clone() });
        page.push(person("invite", &friend.profile, line, vec![button]), Mark::None, open);
    }
    if !ui.chosen.is_empty() && !social.inviting {
        let label = if ui.chosen.len() == 1 { "Invia l'invito" } else { "Invia gli inviti" };
        page.push(Row::Action { key: "send".into(), label: label.into() }, Mark::None, Target::Invite);
    } else if social.inviting {
        page.note("inviting", "Mando gli inviti…".into(), Tone::Waiting);
    }
}
