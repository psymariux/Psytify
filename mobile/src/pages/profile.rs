//! A profile, the person's own included, as the GPUI page (`desktop/src/app/profile_page.rs`) in the
//! Iced phone order: the avatar in the pixel circle, the name, `@handle` and the friends, and the
//! relation button (Aggiungi agli amici, Accetta la richiesta, Richiesta inviata to withdraw it, or
//! Amici, which removes the friend after a confirmation); «Sta ascoltando»; the most played artists
//! (they open); the recent listens (they play); the public playlists (they open as collection
//! pages); the friends in common. For someone who is not a friend, the activity is private. On one's
//! own profile the header has «Esci». The core keeps one open profile (`SocialView::open`).

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::Track;
use psytify_core::api::social::{Activity, Relation, UserPage};
use psytify_core::catalog::Load;
use psytify_core::command::SocialCmd;

use super::album::button;
use super::friends::person;
use super::{Art, Built, Header, Mark, PageAction, PageStatus, Route, Row, RowState, Target, Tone};
use crate::dialog::Request;
use crate::downloads::DownloadMark;
use crate::format;

pub(super) fn build(handle: &str, snapshot: &Snapshot) -> Built {
    let mut page = Built::titled("Profilo");
    page.action(PageAction::Retry, Target::Send(Command::Social(SocialCmd::ReloadProfile)));
    let open = match &snapshot.social.open {
        Some((open, Load::Ready(profile))) if open == handle => Arc::clone(profile),
        Some((open, Load::Failed(error))) if open == handle => {
            page.status = PageStatus::Failed { message: error.to_string() };
            return page;
        }
        _ => return page,
    };
    let user = &open.page;
    let pending = snapshot.social.pending.contains(&user.profile.handle);
    page.header = Some(header(user, pending));
    if user.relation == Relation::Me {
        page.action(PageAction::SignOut, Target::Send(Command::Social(SocialCmd::SignOut)));
    } else if !pending {
        page.action(PageAction::Relation, relation_target(user));
    }
    page.status = PageStatus::Ready;
    match &user.activity {
        Some(activity) => listened(&mut page, activity, &open.recent),
        None if user.relation != Relation::Me => {
            let text = "L'attività è per gli amici: quando lo siete vedi cosa ascolta, gli ascolti recenti e \
                        gli artisti preferiti.";
            page.note("private", text.to_owned(), Tone::Plain);
        }
        None => {}
    }
    if !user.playlists.is_empty() {
        page.heading("playlists", "Playlist pubbliche", user.playlists.len().to_string());
        for playlist in &user.playlists {
            let row = Row::Item {
                key: format!("playlist:{}", playlist.id),
                art: Art::Cover { url: None },
                title: playlist.name.clone(),
                detail: format::tracks(playlist.track_count as usize),
                state: RowState::Idle,
            };
            let route =
                Route::FriendPlaylist { handle: user.profile.handle.clone(), id: playlist.id.to_string() };
            page.push(row, Mark::None, Target::Open(route));
        }
    }
    if !user.mutual_friends.is_empty() {
        page.heading("mutual", "Amici in comune", user.mutual_friends.len().to_string());
        for friend in &user.mutual_friends {
            let line = (format!("@{}", friend.handle), false);
            let open = Target::Open(Route::Profile { handle: friend.handle.clone() });
            page.push(person("mutual", friend, line, Vec::new()), Mark::None, open);
        }
    }
    page
}

/// What the relation button does: removing a friend asks first.
fn relation_target(user: &UserPage) -> Target {
    let handle = user.profile.handle.clone();
    match user.relation {
        Relation::Friend => {
            Target::Dialog(Request::Unfriend { handle, name: user.profile.display_name.clone() })
        }
        Relation::Outgoing => Target::Send(Command::Social(SocialCmd::FriendAction { handle, add: false })),
        Relation::Me => Target::None,
        Relation::Incoming | Relation::None => {
            Target::Send(Command::Social(SocialCmd::FriendAction { handle, add: true }))
        }
    }
}

/// «Sta ascoltando», the most played artists and the recent listens.
fn listened(page: &mut Built, activity: &Activity, recent: &Arc<[Track]>) {
    if let Some(now) = &activity.now_playing {
        page.heading("now", if now.paused { "In pausa" } else { "Sta ascoltando" }, String::new());
        let track: Arc<[Track]> = Arc::from(vec![now.track.clone()]);
        push_track(page, "now", &track, 0);
    }
    if !activity.top_artists.is_empty() {
        page.heading("top", "Artisti più ascoltati", "ultimi 30 giorni".to_owned());
        for (rank, artist) in activity.top_artists.iter().enumerate() {
            let row = Row::Item {
                key: format!("artist:{}", artist.id),
                art: Art::Round { url: None },
                title: format!("{}. {}", rank + 1, artist.name),
                detail: plays(artist.plays),
                state: RowState::Idle,
            };
            page.push(row, Mark::None, Target::Open(Route::artist(artist.id)));
        }
    }
    page.heading("recent", "Ascolti recenti", String::new());
    if recent.is_empty() {
        page.note("no-listens", "Ancora nessun ascolto.".to_owned(), Tone::Plain);
    }
    for index in 0..recent.len() {
        push_track(page, "recent", recent, index);
    }
}

/// A track of the profile: a tap plays `list` from there.
fn push_track(page: &mut Built, key: &str, list: &Arc<[Track]>, index: usize) {
    let Some(track) = list.get(index) else { return };
    let row = Row::Track {
        key: format!("{key}:{index}:{}", track.id),
        number: None,
        art: Some(Art::Cover { url: track.cover_url.clone() }),
        title: track.title.clone(),
        subtitle: track.artist.name.clone(),
        duration: format::clock(track.duration_ms),
        state: RowState::Idle,
        more: false,
        download: DownloadMark::None,
    };
    page.push(row, Mark::Track(track.id), Target::Play(Arc::clone(list), index));
}

fn header(user: &UserPage, pending: bool) -> Header {
    let profile = &user.profile;
    let mut buttons = Vec::new();
    let relation = match user.relation {
        Relation::Me => None,
        Relation::Friend => Some(("Amici", true)),
        Relation::Incoming => Some(("Accetta la richiesta", false)),
        Relation::Outgoing => Some(("Richiesta inviata", false)),
        Relation::None => Some(("Aggiungi agli amici", false)),
    };
    match relation {
        None => buttons.push(button(PageAction::SignOut, "Esci", false, false)),
        Some((label, on)) => {
            let mut shown = button(PageAction::Relation, label, false, on);
            shown.enabled = !pending;
            buttons.push(shown);
        }
    }
    Header {
        kicker: if user.relation == Relation::Me { "Il tuo profilo".into() } else { "Profilo".into() },
        title: profile.display_name.clone(),
        subtitle: Some(format!("@{}", profile.handle)),
        subtitle_opens: false,
        meta: friends(user.friend_count),
        art: Art::Round { url: profile.avatar_url.clone() },
        buttons,
        note: None,
    }
}

/// «1 amico», «12 amici».
fn friends(count: u32) -> String {
    if count == 1 { "1 amico".to_owned() } else { format!("{count} amici") }
}

/// «1 ascolto», «12 ascolti».
fn plays(count: u32) -> String {
    if count == 1 { "1 ascolto".to_owned() } else { format!("{count} ascolti") }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::Me;
    use psytify_core::api::social::{Activity, Listen, MyProfile, NowPlaying, Profile, TopArtist};
    use psytify_core::auth::{ClerkApp, Session};
    use psytify_core::config::Config;
    use psytify_core::testing::LocalCore;
    use uuid::Uuid;

    use super::*;
    use crate::test_support::track;

    fn signed_in() -> LocalCore {
        let mut config = Config::default();
        config.core.clerk_issuer = "https://clerk.example.test".into();
        let mut core = LocalCore::new(config, 1);
        let profile = MyProfile {
            handle: "ada".into(),
            display_name: "Ada".into(),
            avatar_url: None,
            share_activity: true,
        };
        let me = Me { user_id: "user_1".into(), admin: false, profile: Some(profile) };
        let app = ClerkApp { issuer: "https://clerk.example.test".into(), port: 0, session_file: None };
        core.signed_in(Session::for_tests(app), me);
        core
    }

    fn page(relation: Relation, activity: Option<Activity>) -> UserPage {
        UserPage {
            profile: Profile { handle: "ada".into(), display_name: "Ada".into(), avatar_url: None },
            relation,
            friend_count: 1,
            mutual_friends: Vec::new(),
            playlists: Vec::new(),
            activity,
        }
    }

    #[test]
    fn the_own_profile_has_sign_out_the_artists_and_the_listens() {
        let mut core = signed_in();
        core.send(Command::Social(SocialCmd::OpenProfile("ada".into())));
        assert_eq!(build("ada", &core.snapshot()).status, PageStatus::Loading);

        let listened = track(1);
        let activity = Activity {
            now_playing: Some(NowPlaying { track: listened.clone(), position_ms: 0, paused: false }),
            recent: vec![Listen { track: listened, listened_at: "2026-11-27T10:00:00Z".into() }],
            top_artists: vec![TopArtist { id: Uuid::nil(), name: "Onde".into(), plays: 3 }],
        };
        core.user_opened("ada", Ok(page(Relation::Me, Some(activity))));
        let built = build("ada", &core.snapshot());
        assert_eq!(built.status, PageStatus::Ready);
        let header = built.header.as_ref().map(|h| (h.meta.clone(), h.buttons.len()));
        assert_eq!(header, Some(("1 amico".to_owned(), 1)));
        assert!(built.actions.iter().any(|(action, _)| *action == PageAction::SignOut));
        let titles: Vec<String> = built
            .rows
            .iter()
            .map(|row| match row {
                Row::Heading { title, .. } | Row::Item { title, .. } | Row::Track { title, .. } => {
                    title.clone()
                }
                Row::Note { text, .. } => text.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(
            titles,
            ["Sta ascoltando", "Track 1", "Artisti più ascoltati", "1. Onde", "Ascolti recenti", "Track 1"]
        );
        assert_eq!(build("bea", &core.snapshot()).status, PageStatus::Loading, "another profile is not this");
    }

    #[test]
    fn someone_elses_activity_is_private() {
        let mut core = signed_in();
        core.send(Command::Social(SocialCmd::OpenProfile("ada".into())));
        core.user_opened("ada", Ok(page(Relation::None, None)));
        let built = build("ada", &core.snapshot());
        assert!(built.actions.iter().all(|(action, _)| *action != PageAction::SignOut));
        assert!(matches!(built.rows.as_slice(), [Row::Note { .. }]));
    }
}
