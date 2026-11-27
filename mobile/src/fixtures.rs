//! Test data for the social and jam screens (feature `fixtures`, off by default and never in a
//! release build; `scripts/android-kotlin.sh` turns it on only with `FIXTURES=1`), as the desktop's
//! `--fixture`: a fake social view (and, for the `jam-*` names, a fake jam view) replaces the core's,
//! so the sign-in steps, friends, profiles and the jam can be seen without an account, without a
//! room and without sending anything to Clerk or the server. While a fixture is on, social and jam
//! commands do not reach the core; opening a profile or a public playlist, and searching people,
//! are answered here. The people and tracks are made up; avatars and covers have no address.

use std::collections::VecDeque;
use std::sync::Arc;

use psytify_core::Command;
use psytify_core::api::models::{AlbumRef, ArtistRef, Licence};
use psytify_core::api::social::{
    Activity, Friend, Friends, JamInvite, Listen, MyProfile, NowPlaying, PlaylistSummary, Profile,
    PublicPlaylist, Relation, TopArtist, UserPage,
};
use psytify_core::api::{Me, Track};
use psytify_core::auth::Provider;
use psytify_core::catalog::Load;
use psytify_core::command::SocialCmd;
use psytify_core::error::AppError;
use psytify_core::jam::Member;
use psytify_core::snapshot::{
    Account, AuthView, CodeHint, JamEvent, JamPhase, JamRaised, JamView, OpenProfile, SocialView,
};
use uuid::Uuid;

/// The names `use_fixture` knows.
#[cfg(test)]
const NAMES: [&str; 12] = [
    "signin-code",
    "signin-waiting",
    "signin-error",
    "social",
    "social-own",
    "social-private",
    "social-empty",
    "jam-invites",
    "jam-connecting",
    "jam-host",
    "jam-guest",
    "jam-error",
];

/// A fixture on: the views that replace the core's.
#[derive(Debug, Clone)]
pub(crate) struct Fixture {
    pub(crate) social: Arc<SocialView>,
    pub(crate) jam: Option<Arc<JamView>>,
}

impl Fixture {
    pub(crate) fn named(name: &str) -> Option<Self> {
        Some(Self { social: Arc::new(social(name)?), jam: jam(name).map(Arc::new) })
    }

    /// A social or jam command while the fixture is on: never to the core. Some are answered here;
    /// `true` if the fake view changed.
    pub(crate) fn intercept(&mut self, command: &Command) -> bool {
        let Command::Social(command) = command else { return false };
        let view = Arc::make_mut(&mut self.social);
        match command {
            SocialCmd::OpenProfile(handle) => {
                if view.open.as_ref().is_some_and(|(open, _)| open == handle) {
                    return false;
                }
                view.open = Some((handle.clone(), Load::Ready(profile_of(handle))));
                true
            }
            SocialCmd::OpenFriendPlaylist { id, .. } => {
                view.playlist = Some(Load::Ready(Arc::new(public_playlist(*id))));
                true
            }
            SocialCmd::SearchUsers(query) if query.is_empty() => {
                view.found = Load::Idle;
                true
            }
            SocialCmd::SearchUsers(query) => {
                let all = [
                    person("luca", "Luca Bianchi"),
                    person("lucia", "Lucia Ferri"),
                    person("ludovica", "Ludovica Sala"),
                ];
                let found: Vec<Profile> =
                    all.into_iter().filter(|p| p.handle.contains(query.as_str())).collect();
                view.found = Load::Ready(Arc::from(found));
                true
            }
            _ => false,
        }
    }
}

fn person(handle: &str, name: &str) -> Profile {
    Profile { handle: handle.to_owned(), display_name: name.to_owned(), avatar_url: None }
}

fn track(n: u128, title: &str, artist: &str) -> Track {
    Track {
        id: Uuid::from_u128(0xF1C5_0000 + n),
        title: title.to_owned(),
        track_no: u32::try_from(n).ok(),
        duration_ms: 150_000 + 7_000 * u64::try_from(n).unwrap_or(0),
        audio_format: "flac".to_owned(),
        downloadable: false,
        has_video: false,
        video_ready: false,
        artist: ArtistRef { id: Uuid::from_u128(0xF1C5_1000 + n % 4), name: artist.to_owned() },
        album: Some(AlbumRef {
            id: Uuid::from_u128(0xF1C5_2000 + n % 3),
            title: "Banchi di sabbia".to_owned(),
        }),
        licence: Licence { kind: "cc-by".to_owned(), attribution: artist.to_owned() },
        cover_url: None,
        created_at: "2026-11-20T10:00:00Z".to_owned(),
    }
}

fn me() -> Me {
    Me {
        user_id: "user_fixture".to_owned(),
        admin: false,
        profile: Some(MyProfile {
            handle: "psy".to_owned(),
            display_name: "Psy".to_owned(),
            avatar_url: None,
            share_activity: true,
        }),
    }
}

fn recent() -> Vec<Track> {
    vec![
        track(1, "Low Tide", "Marea"),
        track(2, "Neon Harbour", "Faro Nord"),
        track(3, "Sale", "Marea"),
        track(4, "Vetro", "Lumen Quartet"),
        track(5, "Ritorno", "Faro Nord"),
    ]
}

fn activity(now: Option<NowPlaying>) -> Activity {
    let top = [("Marea", 42), ("Faro Nord", 31), ("Lumen Quartet", 12), ("Onde Corte", 5), ("Pietra", 1)];
    Activity {
        now_playing: now,
        recent: recent()
            .into_iter()
            .map(|track| Listen { track, listened_at: "2026-11-24T21:00:00Z".to_owned() })
            .collect(),
        top_artists: top
            .into_iter()
            .zip(0u128..)
            .map(|((name, plays), ix)| TopArtist {
                id: Uuid::from_u128(0xF1C5_1000 + ix),
                name: name.to_owned(),
                plays,
            })
            .collect(),
    }
}

const SERA: u128 = 0xF1C5_3001;
const CORRERE: u128 = 0xF1C5_3002;

fn page(profile: Profile, relation: Relation, activity: Option<Activity>) -> Arc<OpenProfile> {
    let recent: Arc<[Track]> = activity
        .as_ref()
        .map(|activity| activity.recent.iter().map(|listen| listen.track.clone()).collect())
        .unwrap_or_else(|| Arc::from(Vec::new()));
    Arc::new(OpenProfile {
        page: UserPage {
            profile,
            relation,
            friend_count: 12,
            mutual_friends: vec![person("sara", "Sara Verdi"), person("marco", "Marco Neri")],
            playlists: vec![
                PlaylistSummary {
                    id: Uuid::from_u128(SERA),
                    name: "Sera al porto".to_owned(),
                    track_count: 5,
                },
                PlaylistSummary { id: Uuid::from_u128(CORRERE), name: "Correre".to_owned(), track_count: 1 },
            ],
            activity,
        },
        recent,
    })
}

/// The profile shown for `handle`: Psy's own, Luca a friend who listens, Giulia who asks, Paolo
/// asked, anyone else not a friend (private).
fn profile_of(handle: &str) -> Arc<OpenProfile> {
    let listening =
        NowPlaying { track: track(2, "Neon Harbour", "Faro Nord"), position_ms: 61_000, paused: false };
    match handle {
        "psy" => page(person("psy", "Psy"), Relation::Me, Some(activity(None))),
        "luca" => page(person("luca", "Luca Bianchi"), Relation::Friend, Some(activity(Some(listening)))),
        "sara" => page(person("sara", "Sara Verdi"), Relation::Friend, Some(activity(None))),
        "marco" => page(person("marco", "Marco Neri"), Relation::Friend, Some(activity(None))),
        "giulia" => page(person("giulia", "Giulia Rossi"), Relation::Incoming, None),
        "paolo" => page(person("paolo", "Paolo Gialli"), Relation::Outgoing, None),
        other => page(person(other, &capitalised(other)), Relation::None, None),
    }
}

fn capitalised(handle: &str) -> String {
    let mut chars = handle.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn public_playlist(id: Uuid) -> PublicPlaylist {
    let (name, tracks) = if id == Uuid::from_u128(CORRERE) {
        ("Correre", recent()[..1].to_vec())
    } else {
        ("Sera al porto", recent())
    };
    PublicPlaylist {
        id,
        name: name.to_owned(),
        owner: person("luca", "Luca Bianchi"),
        tracks: Arc::from(tracks),
    }
}

fn friends() -> Arc<Friends> {
    let playing = |n: u128, title: &str, artist: &str, paused: bool| NowPlaying {
        track: track(n, title, artist),
        position_ms: 61_000,
        paused,
    };
    Arc::new(Friends {
        friends: vec![
            Friend {
                profile: person("luca", "Luca Bianchi"),
                now_playing: Some(playing(2, "Neon Harbour", "Faro Nord", false)),
            },
            Friend {
                profile: person("sara", "Sara Verdi"),
                now_playing: Some(playing(4, "Vetro", "Lumen Quartet", true)),
            },
            Friend { profile: person("marco", "Marco Neri"), now_playing: None },
        ],
        incoming: vec![person("giulia", "Giulia Rossi")],
        outgoing: vec![person("paolo", "Paolo Gialli")],
    })
}

fn signed_in(friends: Arc<Friends>) -> SocialView {
    SocialView {
        account: Account::SignedIn(me()),
        friends: Load::Ready(friends),
        found: Load::Idle,
        pending: Vec::new(),
        events: Arc::new(VecDeque::new()),
        ..SocialView::default()
    }
}

fn social(name: &str) -> Option<SocialView> {
    let signed_out =
        |auth: AuthView| SocialView { account: Account::SignedOut, auth, ..SocialView::default() };
    Some(match name {
        "signin-code" => signed_out(AuthView {
            step: Some(CodeHint { sent_to: "p***@example.com".to_owned() }),
            ..AuthView::default()
        }),
        "signin-waiting" => SocialView {
            account: Account::Working,
            auth: AuthView { busy: true, waiting: Some(Provider::GitHub), ..AuthView::default() },
            ..SocialView::default()
        },
        "signin-error" => signed_out(AuthView {
            error: Some(AppError::Auth("Password errata. Riprova o usa un'altra strada.".to_owned())),
            ..AuthView::default()
        }),
        "social" | "social-own" | "social-private" => {
            let mut view = signed_in(friends());
            view.found = Load::Ready(Arc::from(vec![
                person("luca", "Luca Bianchi"),
                person("lucia", "Lucia Ferri"),
                person("ludovica", "Ludovica Sala"),
            ]));
            view.pending = vec!["giulia".to_owned()];
            view
        }
        "social-empty" => signed_in(Arc::new(Friends::default())),
        "jam-invites" => {
            let mut view = signed_in(friends());
            view.jam_invites = Arc::from(vec![
                JamInvite {
                    code: "K7Q2MX".to_owned(),
                    from_handle: "luca".to_owned(),
                    from_name: "Luca Bianchi".to_owned(),
                },
                JamInvite {
                    code: "B4RN3S".to_owned(),
                    from_handle: "sara".to_owned(),
                    from_name: "Sara Verdi".to_owned(),
                },
            ]);
            view
        }
        "jam-connecting" | "jam-host" | "jam-guest" | "jam-error" => signed_in(friends()),
        _ => return None,
    })
}

fn member(n: u128, name: &str) -> Member {
    Member { id: Uuid::from_u128(0xF1C5_4000 + n), name: name.to_owned() }
}

fn jam(name: &str) -> Option<JamView> {
    let open = |members: Vec<Member>| JamView {
        phase: JamPhase::Open,
        code: Some("K7Q2MX".to_owned()),
        you: Some(Uuid::from_u128(0xF1C5_4001)),
        members: Arc::from(members),
        events: Arc::new(VecDeque::new()),
    };
    Some(match name {
        "jam-connecting" => JamView { phase: JamPhase::Connecting, ..JamView::default() },
        "jam-host" => open(vec![member(1, "Psy")]),
        "jam-guest" => open(vec![
            member(2, "Luca Bianchi"),
            member(3, "Sara Verdi"),
            member(1, "Psy"),
            member(4, "Marco Neri"),
        ]),
        // The room closed by itself: the card says so and offers «Riprova».
        "jam-error" => JamView {
            events: Arc::new(VecDeque::from([JamRaised {
                seq: 1,
                event: JamEvent::Closed("Impossibile avviare la jam".to_owned()),
            }])),
            ..JamView::default()
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_a_view_and_profiles_answer_by_handle() {
        for name in NAMES {
            assert!(Fixture::named(name).is_some(), "{name}");
        }
        assert!(Fixture::named("nowhere").is_none());
        let mut fixture = Fixture::named("social").unwrap_or_else(|| unreachable!());
        assert!(fixture.intercept(&Command::Social(SocialCmd::OpenProfile("psy".into()))));
        assert!(matches!(
            &fixture.social.open,
            Some((handle, Load::Ready(open))) if handle == "psy" && open.page.relation == Relation::Me
        ));
        assert!(!fixture.intercept(&Command::Social(SocialCmd::OpenProfile("psy".into()))), "already open");
        assert!(fixture.intercept(&Command::Social(SocialCmd::SearchUsers("luc".into()))));
        assert!(matches!(&fixture.social.found, Load::Ready(found) if found.len() == 2));
    }
}
