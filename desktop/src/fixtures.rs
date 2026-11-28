//! Test data for the social and jam screens (feature `fixtures`, off by default and never in a
//! release binary): `--fixture NAME` shows a fake social view (and, for the `jam-*` names, also a
//! fake jam view) in place of the core's, so the sign-in screens (code, wait for the
//! browser, error), the friends, the profiles and the jam (invitations, connecting, in a room, error) are
//! photographed without a real sign-in, without opening a room and without sending anything to Clerk or the server.
//! With the test data the social and jam commands do not start (`CoreModel::social`, `CoreModel::jam`);
//! the rest of the app is the real one.
//!
//! The `update-*` names put a test update (`CoreModel::update_view`) in place of the
//! core's, for the banner and the Settings: the update commands then do not start and no
//! package is really downloaded.
//!
//! People and tracks are invented; avatars and covers have no address (the placeholder).

use std::collections::VecDeque;
use std::sync::Arc;

use psytify_core::api::models::{AlbumRef, ArtistRef, Licence};
use psytify_core::api::social::JamInvite;
use psytify_core::api::social::{
    Activity, Friend, Friends, Listen, MyProfile, NowPlaying, PlaylistSummary, Profile, PublicPlaylist,
    Relation, TopArtist, UserPage,
};
use psytify_core::api::update::{Asset, Offer};
use psytify_core::api::{Me, Track};
use psytify_core::auth::Provider;
use psytify_core::catalog::Load;
use psytify_core::error::AppError;
use psytify_core::jam::Member;
use psytify_core::snapshot::{
    Account, AuthView, CodeHint, JamEvent, JamPhase, JamRaised, JamView, OpenProfile, SocialView,
};
use psytify_core::snapshot::{UpdateState, UpdateStatus, UpdateView};
use std::path::PathBuf;
use uuid::Uuid;

use crate::notices::Notice;

/// I nomi accettati da `--fixture`.
pub const NAMES: [&str; 18] = [
    "signin-code",
    "signin-waiting",
    "signin-error",
    "signin-busy",
    "social",
    "social-own",
    "social-private",
    "social-empty",
    "jam-invites",
    "jam-connecting",
    "jam-host",
    "jam-guest",
    "jam-error",
    "update-offered",
    "update-downloading",
    "update-ready",
    "update-error",
    "update-check-failed",
];

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
    Activity {
        now_playing: now,
        recent: recent()
            .into_iter()
            .map(|track| Listen { track, listened_at: "2026-11-24T21:00:00Z".to_owned() })
            .collect(),
        top_artists: [
            ("Marea", 42),
            ("Faro Nord", 31),
            ("Lumen Quartet", 12),
            ("Onde Corte", 5),
            ("Pietra", 1),
        ]
        .into_iter()
        .enumerate()
        .map(|(ix, (name, plays))| TopArtist {
            id: Uuid::from_u128(0xF1C5_1000 + ix as u128),
            name: name.to_owned(),
            plays,
        })
        .collect(),
    }
}

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
                    id: Uuid::from_u128(0xF1C5_3001),
                    name: "Sera al porto".to_owned(),
                    track_count: 18,
                },
                PlaylistSummary {
                    id: Uuid::from_u128(0xF1C5_3002),
                    name: "Correre".to_owned(),
                    track_count: 1,
                },
            ],
            activity,
        },
        recent,
    })
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

fn signed_in(open: Option<(String, Load<Arc<OpenProfile>>)>, friends: Arc<Friends>) -> SocialView {
    SocialView {
        account: Account::SignedIn(me()),
        friends: Load::Ready(friends),
        found: Load::Ready(Arc::from(vec![
            person("luca", "Luca Bianchi"),
            person("lucia", "Lucia Ferri"),
            person("ludovica", "Ludovica Sala"),
        ])),
        open,
        playlist: Some(Load::Ready(Arc::new(PublicPlaylist {
            id: Uuid::from_u128(0xF1C5_3001),
            name: "Sera al porto".to_owned(),
            owner: person("luca", "Luca Bianchi"),
            tracks: Arc::from(recent()),
        }))),
        pending: vec!["giulia".to_owned()],
        events: Arc::new(VecDeque::new()),
        ..SocialView::default()
    }
}

/// La vista sociale di prova `name`, se esiste.
pub fn social(name: &str) -> Option<Arc<SocialView>> {
    let signed_out =
        |auth: AuthView| SocialView { account: Account::SignedOut, auth, ..SocialView::default() };
    let view = match name {
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
        "signin-busy" => SocialView {
            account: Account::Working,
            auth: AuthView { busy: true, ..AuthView::default() },
            ..SocialView::default()
        },
        "social" => {
            let now = NowPlaying {
                track: track(2, "Neon Harbour", "Faro Nord"),
                position_ms: 61_000,
                paused: false,
            };
            let luca = page(person("luca", "Luca Bianchi"), Relation::Friend, Some(activity(Some(now))));
            signed_in(Some(("luca".to_owned(), Load::Ready(luca))), friends())
        }
        "social-own" => {
            let own = page(person("psy", "Psy"), Relation::Me, Some(activity(None)));
            signed_in(Some(("psy".to_owned(), Load::Ready(own))), friends())
        }
        "social-private" => {
            let stranger = page(person("lucia", "Lucia Ferri"), Relation::None, None);
            signed_in(Some(("lucia".to_owned(), Load::Ready(stranger))), friends())
        }
        "social-empty" => {
            let mut view = signed_in(None, Arc::new(Friends::default()));
            view.found = Load::Idle;
            view.pending.clear();
            view
        }
        "jam-invites" => {
            let mut view = signed_in(None, friends());
            view.found = Load::Idle;
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
        "jam-connecting" | "jam-host" | "jam-guest" | "jam-error" => {
            let mut view = signed_in(None, friends());
            view.found = Load::Idle;
            view
        }
        _ => return None,
    };
    Some(Arc::new(view))
}

fn member(n: u128, name: &str) -> Member {
    Member { id: Uuid::from_u128(0xF1C5_4000 + n), name: name.to_owned() }
}

/// The test jam view `name` (only for the `jam-*` names that have one: the invitations are of the
/// vista sociale, la jam resta ferma).
pub fn jam(name: &str) -> Option<Arc<JamView>> {
    let open = |members: Vec<Member>, you: u128| JamView {
        phase: JamPhase::Open,
        code: Some("K7Q2MX".to_owned()),
        you: Some(Uuid::from_u128(0xF1C5_4000 + you)),
        members: Arc::from(members),
        events: Arc::new(VecDeque::new()),
    };
    let view = match name {
        "jam-connecting" => JamView { phase: JamPhase::Connecting, ..JamView::default() },
        // Just created: only you are there.
        "jam-host" => open(vec![member(1, "Psy")], 1),
        "jam-guest" => open(
            vec![
                member(2, "Luca Bianchi"),
                member(3, "Sara Verdi"),
                member(1, "Psy"),
                member(4, "Marco Neri"),
            ],
            1,
        ),
        // The room has closed (the jam ended on its own): the page says so and offers «Riprova».
        "jam-error" => JamView {
            events: Arc::new(VecDeque::from([JamRaised {
                seq: 1,
                event: JamEvent::Closed("Impossibile avviare la jam".to_owned()),
            }])),
            ..JamView::default()
        },
        _ => return None,
    };
    Some(Arc::new(view))
}

/// La release di prova: un numero inventato, mai chiesto al server.
const UPDATE_TAG: &str = "v2026.12.1";

/// The test update for the `update-*` names: offered, downloading, ready, the download failed
/// (the core removes the offer and says so with a notice, see [`notice`]) or the check failed.
pub fn update(name: &str) -> Option<UpdateView> {
    let mut view = UpdateView::default();
    match name {
        "update-offered" => {
            view.state = Some(UpdateState::Offered(Offer {
                tag: UPDATE_TAG.to_owned(),
                asset: Asset { name: format!("psytify-{UPDATE_TAG}-linux-x86_64.tar.gz"), size: 18_400_000 },
            }));
        }
        "update-downloading" => {
            view.state = Some(UpdateState::Downloading {
                tag: UPDATE_TAG.to_owned(),
                received: 7_360_000,
                total: Some(18_400_000),
            });
        }
        "update-ready" => {
            view.state = Some(UpdateState::Downloaded(PathBuf::from(format!(
                "/home/tu/.local/share/psytify/updates/psytify-{UPDATE_TAG}-linux-x86_64.tar.gz"
            ))));
        }
        "update-error" => {}
        "update-check-failed" => view.status = UpdateStatus::Failed,
        _ => return None,
    }
    Some(view)
}

/// The notice that goes with the test data, if there is one (as the core's event would give it).
pub fn notice(name: &str) -> Option<Notice> {
    match name {
        "update-error" => Some(Notice::error(format!("Download non riuscito: {}", AppError::Network))),
        "update-check-failed" => {
            Some(Notice::error(format!("Controllo non riuscito: {}", AppError::Network)))
        }
        // Like the stage's notice when the core counts a video that stopped.
        #[cfg(feature = "video")]
        "video-failed" => Some(Notice::error(AppError::Video.to_string())),
        _ => None,
    }
}

/// The names of the test videos (only with the `video` feature).
#[cfg(feature = "video")]
pub const VIDEO_NAMES: [&str; 3] = ["video-loading", "video-ready", "video-failed"];

/// Il file del videoclip di `video-ready`: un percorso locale (nessuna richiesta al server).
#[cfg(feature = "video")]
pub const VIDEO_FILE_VAR: &str = "PSYTIFY_VIDEO_FIXTURE";

/// The test video for the track `track` (the one playing at start-up): preparing, ready on the file
/// of `PSYTIFY_VIDEO_FIXTURE` (without the file it stays preparing) or failed.
#[cfg(feature = "video")]
pub fn video(name: &str, track: Option<Uuid>) -> Option<crate::video::ClipView> {
    use crate::video::{Clip, ClipView, Source};
    let clip = match name {
        "video-loading" => Clip::Loading,
        "video-ready" => match std::env::var_os(VIDEO_FILE_VAR).map(PathBuf::from) {
            Some(path) => {
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                Clip::Ready(Source::local(1, format!("file://{}", path.display())))
            }
            None => Clip::Loading,
        },
        "video-failed" => Clip::Failed(AppError::Video.to_string()),
        _ => return None,
    };
    let failures = u64::from(name == "video-failed");
    Some(ClipView { track, clip, failures })
}

/// Un nome di `--fixture` esiste.
pub fn known(name: &str) -> bool {
    #[cfg(feature = "video")]
    if VIDEO_NAMES.contains(&name) {
        return true;
    }
    NAMES.contains(&name)
}

/// All the names, for the `--fixture` error message.
pub fn names() -> String {
    let names = NAMES.join(", ");
    #[cfg(feature = "video")]
    let names = format!("{names}, {}", VIDEO_NAMES.join(", "));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_fixture_exists() {
        for name in NAMES.iter().filter(|name| !name.starts_with("update-")) {
            assert!(social(name).is_some(), "{name}");
        }
        for name in NAMES.iter().filter(|name| name.starts_with("update-")) {
            assert!(update(name).is_some() && social(name).is_none(), "{name}");
        }
        assert!(update("social").is_none());
        assert!(known("update-ready") && !known("nope"));
        let offered = update("update-offered").unwrap_or_default();
        assert!(offered.banner().is_some());
        assert!(update("update-error").unwrap_or_default().banner().is_none());
        assert!(notice("update-error").is_some_and(|notice| notice.error));
        assert!(social("nope").is_none());
        for name in NAMES.iter().filter(|name| name.starts_with("jam-") && **name != "jam-invites") {
            assert!(jam(name).is_some(), "{name}");
        }
        assert!(jam("social").is_none() && jam("jam-invites").is_none());
    }

    #[cfg(feature = "video")]
    #[test]
    fn the_video_fixtures_exist_and_are_not_social_ones() {
        let track = Some(Uuid::from_u128(9));
        for name in VIDEO_NAMES {
            assert!(known(name) && social(name).is_none() && update(name).is_none(), "{name}");
            assert_eq!(video(name, track).map(|view| view.track), Some(track), "{name}");
        }
        assert!(
            video("video-failed", track).is_some_and(|view| view.failed_for(Uuid::from_u128(9)).is_some())
        );
        assert!(notice("video-failed").is_some_and(|notice| notice.error));
        assert!(video("social", track).is_none());
        assert!(names().contains("video-ready"));
    }
}
