//! Account, profile, friends, presence and public playlists (step 4.7).
//!
//! **Sign-in.** Password, the code by email and GitHub, Google or Discord with the return on the loopback are
//! a state machine of the reducer: each step is a job (`Job`) that talks to Clerk, and its outcome
//! (`Done::Stepped`) leads to the next step (a code to type, or `Job::Enter`: the user on the server and, the first
//! time, a profile proposed by Clerk). The browser opens with an effect
//! (`Effect::OpenUrl`, which the host runs with `platform::open_url`), not inside a job, so the tests
//! see it without opening it. Each attempt has a number: if the user gives up, or the browser does not
//! open, the outcome that arrives later is thrown away. The text fields stay in the frontend, which sends the values
//! in the command; the password and the code are `Secret`s, which are not printed.
//!
//! **Where the token lives.** The session (`auth::Session`: the client token and session id, plus the
//! one-minute token the backend is sent) stays only here inside, never in the view, the events or the
//! logs, and the jobs carry a copy. The one-minute token is renewed by the job that needs it
//! (`Session::bearer`, a single lock): no timer renews it, because an idle app must not phone Clerk.
//! `session.json` sits in the core's data folder (`ClerkApp::session_file`).
//!
//! **What goes to the server by itself.** After each input the passing between domains calls
//! [`Social::sync`] and [`Social::listen`]: the track being listened to (when the track changes, pauses,
//! resumes or jumps far; never while the track loads, so it is sent once for each
//! track), the plays (a track heard for 30 seconds, only with a sign-in made: one heard before
//! does not go later) and the playlists made public (when their name or tracks change; or go private again, or
//! disappear). The comparison costs a few fields; the network starts only if something has changed.
//!
//! **When it wakes.** A core without a sign-in arms no timer. With a sign-in and a profile it arms one
//! of 15 seconds (friends, requests and jam invitations), which stops on exit; the deferral of a
//! position jump too close to the previous one is a one-shot timer.
//!
//! Change the view only if something really changes: a poll that brings back the same friends, a
//! repeated command or a superseded outcome publish nothing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use uuid::Uuid;

use crate::api::social::{
    Friends, JamInvite, MyProfile, Profile, ProfileDraft, PublicPlaylist, Relation, UserPage, suggest_handle,
};
use crate::api::{ApiClient, Me, Track};
use crate::auth::{self, ClerkApp, ClerkUser, CodeStep, Oauth, Provider, Secret, Session, Step};
use crate::catalog::Load;
use crate::config;
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::player::PlaybackState;
use crate::state::library::LibraryView;
use crate::state::playback::PlaybackView;

/// How often, with a sign-in and a profile, friend requests, friends and invitations are asked for.
pub const POLL: Duration = Duration::from_secs(15);
/// A jump beyond this difference from the expected position updates the track being listened to.
pub const PRESENCE_DRIFT: Duration = Duration::from_secs(5);
/// Between two updates of the track being listened to, at least this time (one jump after another).
pub const PRESENCE_GAP: Duration = Duration::from_secs(2);
/// Searched users shown.
pub const MAX_FOUND: usize = 20;
/// Tracks of a public playlist sent to the server, at most.
const PUBLIC_TRACKS: usize = 500;
/// Events kept in the view: a frontend that reads at each publish does not lose any.
const EVENTS: usize = 32;

/// The domain's commands.
#[derive(Debug, Clone)]
pub enum SocialCmd {
    /// Sign-in with email or username and password.
    SignIn { identifier: String, password: Secret },
    /// Sign-up: the email is confirmed with a code.
    SignUp { email: String, username: String, password: Secret },
    /// The code Clerk sent by email, for the step in progress (`AuthView::step`).
    SubmitCode(Secret),
    /// Another code for the same step.
    ResendCode,
    /// Sign-in with the browser (GitHub, Google or Discord): the core opens the provider and waits for the return.
    SignInWith(Provider),
    /// Returns from the code to the fields, or gives up waiting for the browser (frees the loopback port).
    /// A send already started is not stopped: it finishes by itself.
    CancelSignIn,
    /// Removes the sign-in error (the frontend sends it when the user resumes typing).
    DismissError,
    /// Signs out: closes the session on Clerk, removes the presence from the server and the file from disk.
    SignOut,
    /// Saves the username, name and, with `Some`, the sharing of the activity.
    SaveProfile { handle: String, display_name: String, share_activity: Option<bool> },
    /// Rereads the friends and the requests (the Friends page is open, or "Riprova").
    RefreshFriends,
    /// Asks for or accepts the friendship (`add`), or removes it, withdraws it or refuses it.
    FriendAction { handle: String, add: bool },
    /// Searches people by username; an empty text clears the results.
    SearchUsers(String),
    /// Opens the profile of `handle` (its loading is in the view); if it is already open and ready it does
    /// nothing.
    OpenProfile(String),
    /// "Riprova" on the open profile: rereads it.
    ReloadProfile,
    /// Opens a friend's public playlist.
    OpenFriendPlaylist { handle: String, id: Uuid },
    /// Invites friends to the jam one is in.
    InviteToJam(Vec<String>),
    /// Removes a jam invitation, at once from the view and from the server.
    DismissInvite(String),
    /// Tests only: the sign-in is already done, as if the last step had just finished.
    #[cfg(any(test, feature = "test-support"))]
    Adopt { session: Session, me: Me },
}

/// The state of the Clerk sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Account {
    /// Clerk is not configured: no sign-in button.
    Unavailable,
    #[default]
    SignedOut,
    /// A sign-in in progress (also the session restore at startup).
    Working,
    SignedIn(Me),
}

impl Account {
    pub fn me(&self) -> Option<&Me> {
        match self {
            Self::SignedIn(me) => Some(me),
            _ => None,
        }
    }

    /// One's own profile: missing until the user chooses a username.
    pub fn profile(&self) -> Option<&MyProfile> {
        self.me()?.profile.as_ref()
    }
}

/// Where the sign-in is, for the form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthView {
    /// A send has started and has not come back yet: the form does not accept another.
    pub busy: bool,
    /// The provider opened in the browser, while the return is awaited.
    pub waiting: Option<Provider>,
    /// Clerk has sent a code: that one is asked for.
    pub step: Option<CodeHint>,
    /// The last attempt did not succeed. It stays until another is made or the frontend removes it.
    pub error: Option<AppError>,
}

/// Where the code arrived (the address, partly hidden by Clerk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeHint {
    pub sent_to: String,
}

/// An open profile, with the recent plays already ready for the queue.
#[derive(Debug, Clone)]
pub struct OpenProfile {
    pub page: UserPage,
    pub recent: Arc<[Track]>,
}

/// Something the frontend must make known.
#[derive(Debug, Clone, PartialEq)]
pub enum SocialEvent {
    /// The sign-in succeeded (not the restore at startup): the profile's name, or "di nuovo".
    Welcomed(String),
    /// Another code has been sent, to this address.
    CodeResent(String),
    ProfileSaved,
    /// The chosen username already belongs to someone else.
    HandleTaken,
    /// The username is too short, or the display name is empty.
    HandleTooShort,
    /// A friendship action succeeded: the resulting relationship.
    FriendChanged(Relation),
    /// A jam invitation is new: who sent it, and how many others arrived together.
    JamInvite {
        from: String,
        more: usize,
    },
    /// The jam invitations have been sent.
    InvitesSent(usize),
    /// The jam has no code to give the friends yet.
    JamNotReady,
    /// An operation did not succeed (the expired session included: the core signs out by itself).
    Failed(AppError),
}

impl SocialEvent {
    /// The notice to show.
    pub fn message(&self) -> String {
        match self {
            Self::Welcomed(name) => format!("Ciao {name}!"),
            Self::CodeResent(to) => format!("Nuovo codice inviato a {to}"),
            Self::ProfileSaved => "Profilo salvato".into(),
            Self::HandleTaken => "Questo nome utente è già preso.".into(),
            Self::HandleTooShort => "Il nome utente vuole almeno 3 caratteri.".into(),
            Self::FriendChanged(Relation::Friend) => "Ora siete amici".into(),
            Self::FriendChanged(Relation::Outgoing) => "Richiesta inviata".into(),
            Self::FriendChanged(_) => "Fatto".into(),
            Self::JamInvite { from, more: 0 } => format!("{from} ti invita in una jam"),
            Self::JamInvite { from, more } => format!("{from} e altri {more} ti invitano in una jam"),
            Self::InvitesSent(1) => "Invito mandato".into(),
            Self::InvitesSent(count) => format!("{count} inviti mandati"),
            Self::JamNotReady => "Aspetta che la jam sia pronta".into(),
            Self::Failed(err) => err.to_string(),
        }
    }

    /// The notice is an error, not information.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::HandleTaken | Self::HandleTooShort | Self::Failed(_))
    }
}

/// A [`SocialEvent`] with its sequence number, which grows with each event.
#[derive(Debug, Clone, PartialEq)]
pub struct SocialRaised {
    pub seq: u64,
    pub event: SocialEvent,
}

/// What the frontends see of the domain.
#[derive(Debug, Clone, Default)]
pub struct SocialView {
    pub account: Account,
    pub auth: AuthView,
    /// The profile is being saved.
    pub profile_saving: bool,
    pub friends: Load<Arc<Friends>>,
    /// The people search result (at most [`MAX_FOUND`]).
    pub found: Load<Arc<[Profile]>>,
    /// The open profile: the username and the page.
    pub open: Option<(String, Load<Arc<OpenProfile>>)>,
    /// The open public playlist.
    pub playlist: Option<Load<Arc<PublicPlaylist>>>,
    /// Usernames with a friendship action in progress.
    pub pending: Vec<String>,
    /// Jam invitations received from friends, the most recent first.
    pub jam_invites: Arc<[JamInvite]>,
    /// The invitations to friends have been sent and have not answered yet.
    pub inviting: bool,
    /// The latest events, from the oldest: a frontend shows those with a new `seq`, in order.
    pub events: Arc<VecDeque<SocialRaised>>,
}

/// The domain's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// The poll every [`POLL`]: armed only with a sign-in and a profile.
    Poll,
    /// A position jump that arrived too soon after the last send: it is deferred to [`PRESENCE_GAP`].
    Presence,
}

/// The track being listened to that is to be told to the server.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Presence {
    Put { track: Uuid, position_ms: u64, paused: bool },
    Delete,
}

/// The domain's async jobs (network, files), which the host runs on the runtime.
#[derive(Debug)]
pub(crate) enum Job {
    SignIn {
        app: ClerkApp,
        identifier: String,
        password: Secret,
        attempt: u64,
    },
    SignUp {
        app: ClerkApp,
        email: String,
        username: String,
        password: Secret,
        attempt: u64,
    },
    SubmitCode {
        app: ClerkApp,
        step: CodeStep,
        code: Secret,
        attempt: u64,
    },
    ResendCode {
        app: ClerkApp,
        step: CodeStep,
    },
    /// Puts the loopback in listening and asks Clerk for the provider.
    OauthBegin {
        app: ClerkApp,
        provider: Provider,
        attempt: u64,
    },
    /// Waits for the browser's return, until `cancel` fires.
    OauthWait {
        app: ClerkApp,
        oauth: Box<Oauth>,
        cancel: Arc<Notify>,
        attempt: u64,
    },
    /// At startup: the saved session, if Clerk still considers it active.
    Restore {
        app: ClerkApp,
        api: Result<ApiClient, AppError>,
    },
    /// After the sign-in: the user on the server and, the first time, the profile proposed by Clerk.
    Enter {
        api: Result<ApiClient, AppError>,
        session: Session,
        user: ClerkUser,
    },
    SignOut {
        api: Result<ApiClient, AppError>,
        session: Session,
    },
    /// Removes `session.json`: Clerk no longer recognises the session.
    Forget {
        app: ClerkApp,
    },
    Friends {
        api: Result<ApiClient, AppError>,
        session: Session,
        ticket: u64,
    },
    Invites {
        api: Result<ApiClient, AppError>,
        session: Session,
    },
    PutProfile {
        api: Result<ApiClient, AppError>,
        session: Session,
        draft: ProfileDraft,
    },
    UserPage {
        api: Result<ApiClient, AppError>,
        session: Session,
        handle: String,
    },
    Users {
        api: Result<ApiClient, AppError>,
        session: Session,
        ticket: u64,
        query: String,
    },
    UserPlaylist {
        api: Result<ApiClient, AppError>,
        session: Session,
        ticket: u64,
        handle: String,
        id: Uuid,
    },
    FriendAction {
        api: Result<ApiClient, AppError>,
        session: Session,
        handle: String,
        add: bool,
    },
    InviteToJam {
        api: Result<ApiClient, AppError>,
        session: Session,
        code: String,
        handles: Vec<String>,
    },
    DismissInvite {
        api: Result<ApiClient, AppError>,
        session: Session,
        code: String,
    },
    Presence {
        api: Result<ApiClient, AppError>,
        session: Session,
        what: Presence,
    },
    Listen {
        api: Result<ApiClient, AppError>,
        session: Session,
        track: Uuid,
    },
    PutPlaylist {
        api: Result<ApiClient, AppError>,
        session: Session,
        id: Uuid,
        name: String,
        tracks: Vec<Uuid>,
    },
    DeletePlaylist {
        api: Result<ApiClient, AppError>,
        session: Session,
        id: Uuid,
    },
}

/// The outcome of a [`Job`], which goes back to the domain.
#[derive(Debug)]
pub(crate) enum Done {
    /// A sign-in step: done, or a code is needed. `attempt` says which attempt it answers.
    Stepped {
        attempt: u64,
        result: Result<Box<Step>, AppError>,
    },
    /// The loopback is listening and Clerk has given the provider's address.
    OauthReady {
        attempt: u64,
        result: Result<Box<Oauth>, AppError>,
    },
    /// The host did not manage to open the browser.
    BrowserFailed,
    CodeResent(Result<CodeStep, AppError>),
    /// The sign-in is complete (or the restore has finished): session, Clerk user and server user.
    Entered {
        restored: bool,
        result: Result<Box<(Session, ClerkUser, Me)>, AppError>,
    },
    SignedOut(Result<(), AppError>),
    Forgotten(Result<(), AppError>),
    ProfileSaved(Result<MyProfile, AppError>),
    Friends {
        ticket: u64,
        result: Result<Friends, AppError>,
    },
    Invites(Result<Vec<JamInvite>, AppError>),
    Invited(Result<usize, AppError>),
    Found {
        ticket: u64,
        result: Result<Vec<Profile>, AppError>,
    },
    ProfileLoaded {
        handle: String,
        result: Result<Box<OpenProfile>, AppError>,
    },
    FriendChanged {
        handle: String,
        result: Result<Relation, AppError>,
    },
    PlaylistLoaded {
        ticket: u64,
        result: Result<Arc<PublicPlaylist>, AppError>,
    },
    /// A send to the server that shows nothing (track being listened to, plays, playlist, invitation removed):
    /// only the expired session counts.
    Sync(Result<(), AppError>),
}

/// The last track being listened to sent to the server.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SentPresence {
    track: Uuid,
    paused: bool,
    position: Duration,
    at: Instant,
}

/// What the profile wants friends to see now.
enum Wanted {
    /// The track is loading: one waits for it to play (or fail), saying nothing in the meantime.
    Hold,
    Nothing,
    Track {
        track: Uuid,
        paused: bool,
        position: Duration,
    },
}

/// The fingerprint of name and tracks: it changes when the public playlist must be updated on the server.
fn fingerprint(name: &str, tracks: &[Track]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    for track in tracks {
        track.id.hash(&mut hasher);
    }
    hasher.finish()
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug)]
pub(crate) struct Social {
    pub(crate) view: Arc<SocialView>,
    /// The Clerk instance, if the sign-in is configured.
    clerk: Option<ClerkApp>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    /// There is a saved session to resume: `start` resumes it.
    restoring: bool,
    /// The Clerk session: never in the view.
    session: Option<Session>,
    /// The Clerk user of the last sign-in: from here the profile is proposed.
    clerk_user: Option<ClerkUser>,
    /// The step with the code to type.
    step: Option<CodeStep>,
    /// The number of the last sign-in attempt: the outcome of a superseded one is thrown away.
    attempt: u64,
    /// Stops the browser wait.
    wait: Option<Arc<Notify>>,
    /// The number of the last reading of the friends, the search and the open playlist.
    friends_ticket: u64,
    found_ticket: u64,
    playlist_ticket: u64,
    /// Codes of the invitations already announced: each is announced once only.
    announced: Vec<String>,
    /// The jam one is in, if one is in one and the room has said its code (the jam gives it).
    jam_code: Option<String>,
    sent: Option<SentPresence>,
    /// Fingerprints of the public playlists already sent.
    published: HashMap<Uuid, u64>,
    /// The playlist list seen last: while it is the same nothing is recalculated.
    seen_playlists: Option<crate::shared::Shared<config::Playlist>>,
    /// The timers armed.
    polling: bool,
    presence_timer: bool,
    /// The number of the last event.
    events: u64,
}

impl Default for Social {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl Social {
    /// The domain with the Clerk instance (if there is one) and the backend chosen in the settings.
    pub(crate) fn new(clerk: Option<ClerkApp>, server: Option<String>) -> Self {
        let account = if clerk.is_some() { Account::SignedOut } else { Account::Unavailable };
        Self {
            view: Arc::new(SocialView { account, ..SocialView::default() }),
            clerk,
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            restoring: false,
            session: None,
            clerk_user: None,
            step: None,
            attempt: 0,
            wait: None,
            friends_ticket: 0,
            found_ticket: 0,
            playlist_ticket: 0,
            announced: Vec::new(),
            jam_code: None,
            sent: None,
            published: HashMap::new(),
            seen_playlists: None,
            polling: false,
            presence_timer: false,
            events: 0,
        }
    }

    /// Where `session.json` sits and whether there is a session to resume (the host knows it, having looked at the
    /// disk before the first snapshot: the reducer does not touch files). With a saved session
    /// the view already starts on "sign-in in progress", and `start` resumes it.
    #[must_use]
    pub(crate) fn with_session(mut self, file: Option<PathBuf>, stored: bool) -> Self {
        if let Some(clerk) = &mut self.clerk {
            clerk.session_file = file;
            if stored && clerk.session_file.is_some() {
                self.restoring = true;
                Arc::make_mut(&mut self.view).account = Account::Working;
            }
        }
        self
    }

    fn view_mut(&mut self) -> &mut SocialView {
        Arc::make_mut(&mut self.view)
    }

    /// The Clerk instance, for the tests that open a sign-in already done.
    #[cfg(test)]
    pub(crate) fn clerk_app(&self) -> Option<ClerkApp> {
        self.clerk.clone()
    }

    /// The backend address in use, for the tests.
    #[cfg(test)]
    pub(crate) fn api_base(&self) -> Option<String> {
        self.api.as_ref().ok().map(|api| api.base_url().to_owned())
    }

    /// The numbers of the last requests, for the tests that hand over an outcome by hand.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn friends_ticket(&self) -> u64 {
        self.friends_ticket
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn attempt(&self) -> u64 {
        self.attempt
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn found_ticket(&self) -> u64 {
        self.found_ticket
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn playlist_ticket(&self) -> u64 {
        self.playlist_ticket
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::Social(job)));
    }

    /// The core has just started: if there is a saved session, it is resumed.
    pub(crate) fn start(&mut self, effects: &mut Vec<Effect>) {
        if !std::mem::take(&mut self.restoring) {
            return;
        }
        if let Some(app) = self.clerk.clone() {
            Self::run(effects, Job::Restore { app, api: self.api() });
        }
    }

    /// The jam room one is in, if the room has said its code: the passing between domains reads it from the jam view
    /// (step 4.8). It serves to not announce the invitation to one's own room and to send the right code.
    /// code. It is copied only when it changes.
    pub(crate) fn set_jam(&mut self, code: Option<&str>) {
        if self.jam_code.as_deref() != code {
            self.jam_code = code.map(str::to_owned);
        }
    }

    /// The backend chosen in the settings has changed: the HTTP client is made again.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    fn raise(&mut self, event: SocialEvent) {
        self.events += 1;
        let raised = SocialRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    /// An operation did not succeed: if the session has expired one signs out, otherwise only the error is said.
    fn fail(&mut self, err: AppError, effects: &mut Vec<Effect>) {
        let expired = err == AppError::AuthExpired;
        self.raise(SocialEvent::Failed(err));
        if expired {
            self.sign_out(effects);
        }
    }

    /// The session and the profile, if there are both: the social functions want a username.
    fn session_with_profile(&self) -> Option<Session> {
        self.session.clone().filter(|_| self.view.account.profile().is_some())
    }

    // ---------- Commands ----------

    pub(crate) fn apply(&mut self, command: SocialCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            SocialCmd::SignIn { identifier, password } => {
                if let Some((app, attempt)) = self.begin(None) {
                    Self::run(effects, Job::SignIn { app, identifier, password, attempt });
                }
            }
            SocialCmd::SignUp { email, username, password } => {
                if let Some((app, attempt)) = self.begin(None) {
                    Self::run(effects, Job::SignUp { app, email, username, password, attempt });
                }
            }
            SocialCmd::SubmitCode(code) => {
                if let (Some(step), Some((app, attempt))) = (self.step.clone(), self.begin(None)) {
                    Self::run(effects, Job::SubmitCode { app, step, code, attempt });
                }
            }
            SocialCmd::ResendCode => {
                if let (Some(step), Some(app)) = (self.step.clone(), self.clerk.clone()) {
                    Self::run(effects, Job::ResendCode { app, step });
                }
            }
            SocialCmd::SignInWith(provider) => {
                if let Some((app, attempt)) = self.begin(Some(provider)) {
                    Self::run(effects, Job::OauthBegin { app, provider, attempt });
                }
            }
            SocialCmd::CancelSignIn => self.cancel_sign_in(),
            SocialCmd::DismissError => {
                if self.view.auth.error.is_some() {
                    self.view_mut().auth.error = None;
                }
            }
            SocialCmd::SignOut => self.sign_out(effects),
            SocialCmd::SaveProfile { handle, display_name, share_activity } => {
                self.save_profile(handle, display_name, share_activity, effects);
            }
            SocialCmd::RefreshFriends => self.load_friends(effects),
            SocialCmd::FriendAction { handle, add } => self.friend_action(handle, add, effects),
            SocialCmd::SearchUsers(query) => self.search_users(&query, effects),
            SocialCmd::OpenProfile(handle) => {
                let open =
                    matches!(&self.view.open, Some((open, load)) if *open == handle && !load.needs_load());
                if !open {
                    self.load_profile(handle, effects);
                }
            }
            SocialCmd::ReloadProfile => {
                if let Some((handle, _)) = self.view.open.clone() {
                    self.load_profile(handle, effects);
                }
            }
            SocialCmd::OpenFriendPlaylist { handle, id } => self.open_friend_playlist(handle, id, effects),
            SocialCmd::InviteToJam(handles) => self.invite_to_jam(handles, effects),
            SocialCmd::DismissInvite(code) => self.dismiss_invite(code, effects),
            #[cfg(any(test, feature = "test-support"))]
            SocialCmd::Adopt { session, me } => {
                self.entered(true, Ok(Box::new((session, ClerkUser::default(), me))), effects);
            }
        }
        self.keep_polling(effects);
    }

    /// A new sign-in attempt: it needs the configured login, no sign-in in progress and none
    /// made. Returns the instance and the attempt's number.
    fn begin(&mut self, waiting: Option<Provider>) -> Option<(ClerkApp, u64)> {
        let clerk = self.clerk.clone()?;
        if self.view.auth.busy || self.view.account != Account::SignedOut {
            return None;
        }
        self.attempt += 1;
        let view = self.view_mut();
        view.auth.busy = true;
        view.auth.waiting = waiting;
        view.auth.error = None;
        view.account = Account::Working;
        Some((clerk, self.attempt))
    }

    /// Closes the attempt in progress without an outcome: neither an error nor a sign-in.
    fn end_attempt(&mut self, account: Account) {
        self.wait = None;
        let view = self.view_mut();
        view.auth.busy = false;
        view.auth.waiting = None;
        if !matches!(view.account, Account::SignedIn(_)) {
            view.account = account;
        }
    }

    fn cancel_sign_in(&mut self) {
        // A send to Clerk already started (password, code) finishes by itself: stopping it would leave a
        // session created on the server and forgotten here. One only gives up waiting for the browser.
        if self.view.auth.waiting.is_some() {
            if let Some(wait) = self.wait.take() {
                wait.notify_one();
            }
            self.attempt += 1;
            self.end_attempt(Account::SignedOut);
            return;
        }
        if self.view.auth.busy {
            return;
        }
        self.step = None;
        if self.view.auth.step.is_some() || self.view.auth.error.is_some() {
            let view = self.view_mut();
            view.auth.step = None;
            view.auth.error = None;
        }
    }

    /// The outcome of a sign-in step, if it belongs to the attempt in progress.
    fn stepped(&mut self, attempt: u64, result: Result<Box<Step>, AppError>, effects: &mut Vec<Effect>) {
        if attempt != self.attempt {
            return;
        }
        match result.map(|step| *step) {
            Ok(Step::Code(step)) => {
                self.end_attempt(Account::SignedOut);
                let hint = CodeHint { sent_to: step.sent_to.clone() };
                self.step = Some(step);
                self.view_mut().auth.step = Some(hint);
            }
            Ok(Step::Done(session, user)) => {
                // The attempt has finished, but the sign-in has not: the user on the server is still read.
                self.wait = None;
                self.step = None;
                let view = self.view_mut();
                view.auth.busy = false;
                view.auth.waiting = None;
                view.auth.step = None;
                Self::run(effects, Job::Enter { api: self.api(), session, user });
            }
            Err(err) => {
                self.end_attempt(Account::SignedOut);
                self.view_mut().auth.error = Some(err);
            }
        }
    }

    /// Clerk has given the provider's address: the host opens the browser, and meanwhile the return is awaited.
    fn oauth_ready(&mut self, attempt: u64, result: Result<Box<Oauth>, AppError>, effects: &mut Vec<Effect>) {
        if attempt != self.attempt {
            // The user gave up: the half sign-in closes with its `Oauth` (and the port).
            return;
        }
        match (result, self.clerk.clone()) {
            (Ok(oauth), Some(app)) => {
                let cancel = Arc::new(Notify::new());
                self.wait = Some(Arc::clone(&cancel));
                effects.push(Effect::OpenUrl(oauth.url().clone()));
                Self::run(effects, Job::OauthWait { app, oauth, cancel, attempt });
            }
            (Err(err), _) => self.stepped(attempt, Err(err), effects),
            (Ok(_), None) => self.stepped(attempt, Err(AppError::AuthNotConfigured), effects),
        }
    }

    /// The sign-in is complete, or the restore has finished.
    fn entered(
        &mut self,
        restored: bool,
        result: Result<Box<(Session, ClerkUser, Me)>, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        match result.map(|entered| *entered) {
            Ok((session, user, me)) => {
                if !restored {
                    let name = me.profile.as_ref().map_or("di nuovo", |p| p.display_name.as_str());
                    self.raise(SocialEvent::Welcomed(name.to_owned()));
                }
                self.session = Some(session);
                self.clerk_user = Some(user);
                self.published.clear();
                self.seen_playlists = None;
                self.sent = None;
                self.step = None;
                let view = self.view_mut();
                view.account = Account::SignedIn(me);
                view.auth = AuthView::default();
                self.load_friends(effects);
            }
            Err(err) => {
                self.session = None;
                let account = if self.clerk.is_some() { Account::SignedOut } else { Account::Unavailable };
                if self.view.account != account {
                    self.view_mut().account = account;
                }
                if !restored {
                    // On the form, where the user is looking.
                    self.view_mut().auth.error = Some(err.clone());
                } else if err != AppError::AuthExpired {
                    self.raise(SocialEvent::Failed(err.clone()));
                }
                // A session that Clerk no longer recognises is no use: it is removed from disk.
                if err == AppError::AuthExpired
                    && let Some(app) = self.clerk.clone()
                {
                    Self::run(effects, Job::Forget { app });
                }
            }
        }
    }

    /// Signs out: the session closes on Clerk, the track being listened to leaves the server and `session.json`
    /// from disk. Everything that was the user's (friends, open profiles, searches) is thrown away.
    fn sign_out(&mut self, effects: &mut Vec<Effect>) {
        let Some(session) = self.session.take() else { return };
        let account = if self.clerk.is_some() { Account::SignedOut } else { Account::Unavailable };
        let auth = self.view.auth.clone();
        let events = Arc::clone(&self.view.events);
        self.view = Arc::new(SocialView { account, auth, events, ..SocialView::default() });
        self.clerk_user = None;
        self.sent = None;
        self.published.clear();
        self.seen_playlists = None;
        self.announced.clear();
        // The replies in flight belong to whoever has signed out.
        self.friends_ticket += 1;
        self.found_ticket += 1;
        self.playlist_ticket += 1;
        Self::run(effects, Job::SignOut { api: self.api(), session });
    }

    fn save_profile(
        &mut self,
        handle: String,
        display_name: String,
        share: Option<bool>,
        effects: &mut Vec<Effect>,
    ) {
        let (Some(session), Account::SignedIn(me)) = (self.session.clone(), &self.view.account) else {
            return;
        };
        let current = me.profile.clone();
        if handle.len() < 3 || display_name.trim().is_empty() {
            self.raise(SocialEvent::HandleTooShort);
            return;
        }
        // The image stays the profile's; the first time, Clerk's (only from its domain).
        let avatar_url = current
            .as_ref()
            .and_then(|p| p.avatar_url.clone())
            .or_else(|| self.clerk_user.as_ref().and_then(|u| u.image_url.clone()))
            .filter(|url| url.starts_with("https://img.clerk.com/"));
        let draft = ProfileDraft {
            handle,
            display_name: display_name.trim().to_owned(),
            avatar_url,
            share_activity: share.or(current.as_ref().map(|p| p.share_activity)),
        };
        self.view_mut().profile_saving = true;
        Self::run(effects, Job::PutProfile { api: self.api(), session, draft });
    }

    fn profile_saved(&mut self, result: Result<MyProfile, AppError>, effects: &mut Vec<Effect>) {
        self.view_mut().profile_saving = false;
        match result {
            Ok(profile) => {
                if !profile.share_activity {
                    // The server removes by itself the track being listened to of whoever stops sharing.
                    self.sent = None;
                }
                let first = self.view.account.profile().is_none();
                if let Account::SignedIn(me) = &mut self.view_mut().account {
                    me.profile = Some(profile);
                }
                self.raise(SocialEvent::ProfileSaved);
                // The first profile opens the social functions: friends and requests are read at once.
                if first {
                    self.load_friends(effects);
                }
            }
            Err(AppError::Api { status: 409, .. }) => self.raise(SocialEvent::HandleTaken),
            Err(err) => self.fail(err, effects),
        }
    }

    // ---------- Friends and profiles ----------

    /// Reads the friends and the requests. Only the reply to the last reading counts.
    fn load_friends(&mut self, effects: &mut Vec<Effect>) {
        let Some(session) = self.session_with_profile() else { return };
        if !matches!(self.view.friends, Load::Ready(_)) && !self.view.friends.is_loading() {
            self.view_mut().friends = Load::Loading;
        }
        self.friends_ticket += 1;
        Self::run(effects, Job::Friends { api: self.api(), session, ticket: self.friends_ticket });
    }

    fn friends_loaded(&mut self, ticket: u64, result: Result<Friends, AppError>, effects: &mut Vec<Effect>) {
        if ticket != self.friends_ticket {
            return;
        }
        let expired = result == Err(AppError::AuthExpired);
        match (result, &self.view.friends) {
            // A periodic update that fails does not delete the list already shown.
            (Err(_), Load::Ready(_)) if !expired => {}
            // The same friends as before: nothing to publish.
            (Ok(friends), Load::Ready(old)) if **old == friends => {}
            (result, _) => self.view_mut().friends = Load::of(result.map(Arc::new)),
        }
        if expired {
            self.fail(AppError::AuthExpired, effects);
        }
    }

    fn load_invites(&mut self, effects: &mut Vec<Effect>) {
        if let Some(session) = self.session_with_profile() {
            Self::run(effects, Job::Invites { api: self.api(), session });
        }
    }

    fn invites_loaded(&mut self, result: Result<Vec<JamInvite>, AppError>, effects: &mut Vec<Effect>) {
        let invites = match result {
            Ok(invites) => invites,
            Err(AppError::AuthExpired) => return self.fail(AppError::AuthExpired, effects),
            // A missed update is retried at the next round.
            Err(_) => return,
        };
        // Whoever is already in a jam does not need the invitation to a room they are in.
        let current = self.jam_code.as_deref();
        let fresh: Vec<&JamInvite> = invites
            .iter()
            .filter(|i| Some(i.code.as_str()) != current && !self.announced.contains(&i.code))
            .collect();
        let announce = fresh.first().map(|first| (first.from_name.clone(), fresh.len() - 1));
        let codes: Vec<String> = fresh.iter().map(|i| i.code.clone()).collect();
        if let Some((from, more)) = announce {
            self.raise(SocialEvent::JamInvite { from, more });
        }
        self.announced.extend(codes);
        // Only the codes still around are remembered: the list does not grow without bounds.
        self.announced.retain(|code| invites.iter().any(|i| i.code == *code));
        if *self.view.jam_invites != *invites {
            self.view_mut().jam_invites = Arc::from(invites);
        }
    }

    fn invite_to_jam(&mut self, handles: Vec<String>, effects: &mut Vec<Effect>) {
        let Some(session) = self.session.clone() else { return };
        let Some(code) = self.jam_code.clone() else {
            self.raise(SocialEvent::JamNotReady);
            return;
        };
        if handles.is_empty() || self.view.inviting {
            return;
        }
        self.view_mut().inviting = true;
        Self::run(effects, Job::InviteToJam { api: self.api(), session, code, handles });
    }

    fn dismiss_invite(&mut self, code: String, effects: &mut Vec<Effect>) {
        if self.view.jam_invites.iter().any(|i| i.code == code) {
            let rest: Arc<[JamInvite]> =
                self.view.jam_invites.iter().filter(|i| i.code != code).cloned().collect();
            self.view_mut().jam_invites = rest;
        }
        if let Some(session) = self.session.clone() {
            Self::run(effects, Job::DismissInvite { api: self.api(), session, code });
        }
    }

    fn search_users(&mut self, query: &str, effects: &mut Vec<Effect>) {
        let query = query.trim();
        self.found_ticket += 1;
        if query.trim_start_matches('@').is_empty() {
            if !matches!(self.view.found, Load::Idle) {
                self.view_mut().found = Load::Idle;
            }
            return;
        }
        let Some(session) = self.session.clone() else { return };
        self.view_mut().found = Load::Loading;
        Self::run(
            effects,
            Job::Users { api: self.api(), session, ticket: self.found_ticket, query: query.to_owned() },
        );
    }

    fn load_profile(&mut self, handle: String, effects: &mut Vec<Effect>) {
        let Some(session) = self.session.clone() else { return };
        self.view_mut().open = Some((handle.clone(), Load::Loading));
        Self::run(effects, Job::UserPage { api: self.api(), session, handle });
    }

    fn friend_action(&mut self, handle: String, add: bool, effects: &mut Vec<Effect>) {
        let Some(session) = self.session.clone() else { return };
        if self.view.pending.contains(&handle) {
            return;
        }
        self.view_mut().pending.push(handle.clone());
        Self::run(effects, Job::FriendAction { api: self.api(), session, handle, add });
    }

    fn friend_changed(
        &mut self,
        handle: String,
        result: Result<Relation, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        self.view_mut().pending.retain(|h| *h != handle);
        match result {
            Ok(relation) => {
                self.raise(SocialEvent::FriendChanged(relation));
                // The open page of this person has the previous relationship: it is reread.
                if matches!(&self.view.open, Some((open, _)) if *open == handle) {
                    self.load_profile(handle, effects);
                }
                self.load_friends(effects);
            }
            Err(err) => self.fail(err, effects),
        }
    }

    fn open_friend_playlist(&mut self, handle: String, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(session) = self.session.clone() else { return };
        self.playlist_ticket += 1;
        self.view_mut().playlist = Some(Load::Loading);
        Self::run(
            effects,
            Job::UserPlaylist { api: self.api(), session, ticket: self.playlist_ticket, handle, id },
        );
    }

    // ---------- Synchronisation ----------

    /// A play counted as listened to: if one is connected with a profile it goes to the profile too. Without one,
    /// it is thrown away: it does not stay pending to start at a future sign-in.
    pub(crate) fn listen(&mut self, track: Uuid, effects: &mut Vec<Effect>) {
        if let Some(session) = self.session_with_profile() {
            Self::run(effects, Job::Listen { api: self.api(), session, track });
        }
    }

    /// After each input: sends the server what has changed in the track being listened to and in the public
    /// playlists. Almost always there is nothing to send.
    pub(crate) fn sync(
        &mut self,
        playback: &PlaybackView,
        library: &LibraryView,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        // The session is lent to the two passes without cloning it: it is copied only when a send starts
        // (this round happens at each tick of playback, and almost always sends nothing).
        let Some(session) = self.session.take() else { return };
        if let Some(share) = self.view.account.profile().map(|p| p.share_activity) {
            self.sync_presence(&session, share, playback, now, effects);
            self.sync_playlists(&session, library, effects);
        }
        self.session = Some(session);
    }

    fn wanted(playback: &PlaybackView, share: bool, now: Instant) -> Wanted {
        if matches!(playback.state, PlaybackState::Loading { .. }) {
            return Wanted::Hold;
        }
        match playback.queue.current().filter(|_| share) {
            Some(track) => Wanted::Track {
                track: track.id,
                paused: !playback.is_playing(),
                position: playback.position(now),
            },
            None => Wanted::Nothing,
        }
    }

    fn sync_presence(
        &mut self,
        session: &Session,
        share: bool,
        playback: &PlaybackView,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        let wanted = Self::wanted(playback, share, now);
        let changed = match (&wanted, self.sent) {
            (Wanted::Hold, _) | (Wanted::Nothing, None) => false,
            (Wanted::Track { .. }, None) | (Wanted::Nothing, Some(_)) => true,
            (Wanted::Track { track, paused, position }, Some(sent)) => {
                let elapsed = now.saturating_duration_since(sent.at);
                let expected = if sent.paused { sent.position } else { sent.position + elapsed };
                let far = position.abs_diff(expected) > PRESENCE_DRIFT;
                if far && elapsed <= PRESENCE_GAP && *track == sent.track && *paused == sent.paused {
                    // A jump right after a send waits: between two sends at least two seconds pass.
                    if !self.presence_timer {
                        self.presence_timer = true;
                        effects.push(Effect::Timer {
                            key: TimerKey::Social(Timer::Presence),
                            after: PRESENCE_GAP - elapsed + Duration::from_millis(1),
                        });
                    }
                }
                *track != sent.track || *paused != sent.paused || (far && elapsed > PRESENCE_GAP)
            }
        };
        if !changed {
            return;
        }
        let (what, sent) = match wanted {
            Wanted::Track { track, paused, position } => {
                let position_ms = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
                (
                    Presence::Put { track, position_ms, paused },
                    Some(SentPresence { track, paused, position, at: now }),
                )
            }
            Wanted::Hold | Wanted::Nothing => (Presence::Delete, None),
        };
        self.sent = sent;
        if std::mem::take(&mut self.presence_timer) {
            effects.push(Effect::CancelTimer(TimerKey::Social(Timer::Presence)));
        }
        Self::run(effects, Job::Presence { api: self.api(), session: session.clone(), what });
    }

    /// The public playlists changed since the last send, and those that no longer are. The fingerprint is
    /// computed only on the public ones (few) and only when the list of playlists is another one.
    fn sync_playlists(&mut self, session: &Session, library: &LibraryView, effects: &mut Vec<Effect>) {
        if self.seen_playlists.as_ref().is_some_and(|seen| seen.same(&library.playlists)) {
            return;
        }
        self.seen_playlists = Some(library.playlists.clone());
        let mut public = HashSet::new();
        for playlist in library.playlists.iter().filter(|p| p.public) {
            public.insert(playlist.id);
            let print = fingerprint(&playlist.name, &playlist.tracks);
            if self.published.get(&playlist.id) == Some(&print) {
                continue;
            }
            self.published.insert(playlist.id, print);
            let tracks: Vec<Uuid> = playlist.tracks.iter().map(|t| t.id).take(PUBLIC_TRACKS).collect();
            Self::run(
                effects,
                Job::PutPlaylist {
                    api: self.api(),
                    session: session.clone(),
                    id: playlist.id,
                    name: playlist.name.clone(),
                    tracks,
                },
            );
        }
        // Gone private again, or removed: it disappears from the profile too.
        let gone: Vec<Uuid> = self.published.keys().filter(|id| !public.contains(id)).copied().collect();
        for id in gone {
            self.published.remove(&id);
            Self::run(effects, Job::DeletePlaylist { api: self.api(), session: session.clone(), id });
        }
    }

    // ---------- Timers ----------

    /// The poll runs only with a sign-in and a profile: it arms or stops it accordingly.
    fn keep_polling(&mut self, effects: &mut Vec<Effect>) {
        let wanted = self.session.is_some() && self.view.account.profile().is_some();
        match (wanted, self.polling) {
            (true, false) => {
                self.polling = true;
                effects.push(Effect::Timer { key: TimerKey::Social(Timer::Poll), after: POLL });
            }
            (false, true) => {
                self.polling = false;
                effects.push(Effect::CancelTimer(TimerKey::Social(Timer::Poll)));
            }
            _ => {}
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Poll => {
                self.polling = false;
                self.load_friends(effects);
                self.load_invites(effects);
            }
            // The passing that follows each input rereads the track being listened to: it is enough to wake it.
            Timer::Presence => self.presence_timer = false,
        }
        self.keep_polling(effects);
    }

    // ---------- Outcomes ----------

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Stepped { attempt, result } => self.stepped(attempt, result, effects),
            Done::OauthReady { attempt, result } => self.oauth_ready(attempt, result, effects),
            Done::BrowserFailed => {
                if self.view.auth.waiting.is_some() {
                    if let Some(wait) = self.wait.take() {
                        wait.notify_one();
                    }
                    let attempt = self.attempt;
                    self.stepped(attempt, Err(AppError::AuthFailed), effects);
                }
            }
            Done::CodeResent(result) => match result {
                Ok(step) => {
                    let to = step.sent_to.clone();
                    self.step = Some(step);
                    self.raise(SocialEvent::CodeResent(to));
                }
                Err(err) => self.view_mut().auth.error = Some(err),
            },
            Done::Entered { restored, result } => self.entered(restored, result, effects),
            Done::SignedOut(result) | Done::Forgotten(result) | Done::Sync(result) => {
                // An error here does not change what is seen. Only an expired session signs out.
                match result {
                    Err(AppError::AuthExpired) => self.fail(AppError::AuthExpired, effects),
                    Err(err) if !matches!(err, AppError::Network | AppError::Timeout) => {
                        // The session file that is not removed, for example: it must be said.
                        self.raise(SocialEvent::Failed(err));
                    }
                    _ => {}
                }
            }
            Done::ProfileSaved(result) => self.profile_saved(result, effects),
            Done::Friends { ticket, result } => self.friends_loaded(ticket, result, effects),
            Done::Invites(result) => self.invites_loaded(result, effects),
            Done::Invited(result) => {
                self.view_mut().inviting = false;
                match result {
                    Ok(count) => self.raise(SocialEvent::InvitesSent(count)),
                    Err(err) => self.fail(err, effects),
                }
            }
            Done::Found { ticket, result } => {
                if ticket == self.found_ticket {
                    self.view_mut().found = Load::of(result.map(|mut found| {
                        found.truncate(MAX_FOUND);
                        Arc::from(found)
                    }));
                }
            }
            Done::ProfileLoaded { handle, result } => {
                if let Some((open, load)) = &self.view.open
                    && *open == handle
                    && load.is_loading()
                    && let Some((_, load)) = &mut self.view_mut().open
                {
                    *load = Load::of(result.map(Arc::from));
                }
            }
            Done::FriendChanged { handle, result } => self.friend_changed(handle, result, effects),
            Done::PlaylistLoaded { ticket, result } => {
                if ticket == self.playlist_ticket {
                    self.view_mut().playlist = Some(Load::of(result));
                }
            }
        }
        self.keep_polling(effects);
    }
}

/// An operation that wants the token: it asks the session for it (renewing it if expired), then calls.
async fn with_token<T, F, Fut>(
    session: Session,
    api: Result<ApiClient, AppError>,
    call: F,
) -> Result<T, AppError>
where
    F: FnOnce(ApiClient, String) -> Fut,
    Fut: std::future::Future<Output = Result<T, AppError>>,
{
    let token = session.bearer().await?;
    call(api?, token).await
}

/// After the sign-in: the user on the server and, the first time, the profile proposed by Clerk. A username
/// already taken receives a number at the end.
async fn enter(api: Result<ApiClient, AppError>, session: Session, user: ClerkUser) -> Result<Me, AppError> {
    let api = api?;
    let token = session.bearer().await?;
    let mut me = api.me(&token).await?;
    if me.profile.is_some() {
        return Ok(me);
    }
    let base = suggest_handle(user.username.as_deref(), user.email.as_deref());
    let display_name = user.name.clone().or(user.username.clone()).unwrap_or_else(|| base.clone());
    let avatar_url = user.image_url.filter(|url| url.starts_with("https://img.clerk.com/"));
    for attempt in 0..4u32 {
        let handle = if attempt == 0 { base.clone() } else { format!("{base}{}", random_digits()) };
        let draft = ProfileDraft {
            handle,
            display_name: display_name.clone(),
            avatar_url: avatar_url.clone(),
            share_activity: None,
        };
        match api.put_profile(&token, &draft).await {
            Ok(profile) => {
                me.profile = Some(profile);
                return Ok(me);
            }
            Err(AppError::Api { status: 409, .. }) => {}
            // A profile that cannot be created does not prevent entering: it is chosen in the settings.
            Err(_) => return Ok(me),
        }
    }
    Ok(me)
}

fn random_digits() -> u32 {
    let mut bytes = [0u8; 2];
    let _ = getrandom::fill(&mut bytes);
    u32::from(u16::from_le_bytes(bytes)) % 1000
}

/// Session, Clerk user and server user, for the outcome of a sign-in.
async fn enter_with(
    api: Result<ApiClient, AppError>,
    session: Session,
    user: ClerkUser,
) -> Result<Box<(Session, ClerkUser, Me)>, AppError> {
    let me = enter(api, session.clone(), user.clone()).await?;
    Ok(Box::new((session, user, me)))
}

/// The `Err` outcome of a lost job, for a job that talks to the server.
fn lost<T>() -> Result<T, AppError> {
    Err(AppError::Network)
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::SignIn { app, identifier, password, attempt } => Done::Stepped {
                attempt,
                result: auth::sign_in(app, identifier, password.into_inner()).await.map(Box::new),
            },
            Self::SignUp { app, email, username, password, attempt } => Done::Stepped {
                attempt,
                result: auth::sign_up(app, email, username, password.into_inner()).await.map(Box::new),
            },
            Self::SubmitCode { app, step, code, attempt } => Done::Stepped {
                attempt,
                result: auth::submit_code(app, step, code.into_inner()).await.map(Box::new),
            },
            Self::ResendCode { app, step } => Done::CodeResent(auth::resend_code(app, step).await),
            Self::OauthBegin { app, provider, attempt } => {
                Done::OauthReady { attempt, result: auth::oauth_begin(&app, provider).await.map(Box::new) }
            }
            Self::OauthWait { app, oauth, cancel, attempt } => {
                let result = tokio::select! {
                    result = auth::oauth_finish(app, *oauth) => result,
                    () = cancel.notified() => Err(AppError::AuthCancelled),
                };
                Done::Stepped { attempt, result: result.map(Box::new) }
            }
            Self::Restore { app, api } => {
                let result = async {
                    let (session, user) = auth::restore(app).await?;
                    enter_with(api, session, user).await
                }
                .await;
                Done::Entered { restored: true, result }
            }
            Self::Enter { api, session, user } => {
                Done::Entered { restored: false, result: enter_with(api, session, user).await }
            }
            Self::SignOut { api, session } => {
                // Whoever signs out does not stay "listening" for the friends.
                if let (Ok(api), Ok(token)) = (api, session.bearer().await) {
                    let _ = api.delete_presence(&token).await;
                }
                Done::SignedOut(session.sign_out().await)
            }
            Self::Forget { app } => Done::Forgotten(auth::forget_session(&app).await),
            Self::Friends { api, session, ticket } => Done::Friends {
                ticket,
                result: with_token(session, api, |api, token| async move { api.friends(&token).await }).await,
            },
            Self::Invites { api, session } => Done::Invites(
                with_token(session, api, |api, token| async move { api.jam_invites(&token).await }).await,
            ),
            Self::PutProfile { api, session, draft } => {
                Done::ProfileSaved(
                    with_token(session, api, move |api, token| async move {
                        api.put_profile(&token, &draft).await
                    })
                    .await,
                )
            }
            Self::UserPage { api, session, handle } => {
                let wanted = handle.clone();
                let result = with_token(session, api, move |api, token| async move {
                    let page = api.user_page(&token, &wanted).await?;
                    let recent: Vec<Track> = page
                        .activity
                        .as_ref()
                        .map(|a| a.recent.iter().map(|l| l.track.clone()).collect())
                        .unwrap_or_default();
                    Ok(Box::new(OpenProfile { page, recent: Arc::from(recent) }))
                })
                .await;
                Done::ProfileLoaded { handle, result }
            }
            Self::Users { api, session, ticket, query } => Done::Found {
                ticket,
                result: with_token(session, api, move |api, token| async move {
                    api.search_users(&token, &query).await
                })
                .await,
            },
            Self::UserPlaylist { api, session, ticket, handle, id } => Done::PlaylistLoaded {
                ticket,
                result: with_token(session, api, move |api, token| async move {
                    api.user_playlist(&token, &handle, id).await.map(Arc::new)
                })
                .await,
            },
            Self::FriendAction { api, session, handle, add } => {
                let target = handle.clone();
                let result = with_token(session, api, move |api, token| async move {
                    if add {
                        api.add_friend(&token, &target).await
                    } else {
                        api.remove_friend(&token, &target).await.map(|()| Relation::None)
                    }
                })
                .await;
                Done::FriendChanged { handle, result }
            }
            Self::InviteToJam { api, session, code, handles } => Done::Invited(
                with_token(session, api, move |api, token| async move {
                    api.invite_to_jam(&token, &code, &handles).await
                })
                .await,
            ),
            Self::DismissInvite { api, session, code } => Done::Sync(
                with_token(session, api, move |api, token| async move {
                    api.dismiss_jam_invite(&token, &code).await
                })
                .await,
            ),
            Self::Presence { api, session, what } => Done::Sync(
                with_token(session, api, move |api, token| async move {
                    match what {
                        Presence::Put { track, position_ms, paused } => {
                            api.put_presence(&token, track, position_ms, paused).await
                        }
                        Presence::Delete => api.delete_presence(&token).await,
                    }
                })
                .await,
            ),
            Self::Listen { api, session, track } => {
                Done::Sync(
                    with_token(session, api, move |api, token| async move {
                        api.post_listen(&token, track).await
                    })
                    .await,
                )
            }
            Self::PutPlaylist { api, session, id, name, tracks } => Done::Sync(
                with_token(session, api, move |api, token| async move {
                    api.put_playlist(&token, id, &name, &tracks).await
                })
                .await,
            ),
            Self::DeletePlaylist { api, session, id } => {
                Done::Sync(
                    with_token(session, api, move |api, token| async move {
                        api.delete_playlist(&token, id).await
                    })
                    .await,
                )
            }
        }
    }

    /// If the job is lost, the domain does not keep waiting: a sign-in returns to the form with an
    /// error, a read fails as without network.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::SignIn { attempt, .. }
            | Self::SignUp { attempt, .. }
            | Self::SubmitCode { attempt, .. }
            | Self::OauthWait { attempt, .. } => {
                Done::Stepped { attempt: *attempt, result: Err(AppError::AuthFailed) }
            }
            Self::OauthBegin { attempt, .. } => {
                Done::OauthReady { attempt: *attempt, result: Err(AppError::AuthFailed) }
            }
            Self::ResendCode { .. } => Done::CodeResent(lost()),
            Self::Restore { .. } => Done::Entered { restored: true, result: lost() },
            Self::Enter { .. } => Done::Entered { restored: false, result: lost() },
            Self::SignOut { .. } => Done::SignedOut(Ok(())),
            Self::Forget { .. } => Done::Forgotten(Ok(())),
            Self::Friends { ticket, .. } => Done::Friends { ticket: *ticket, result: lost() },
            Self::Invites { .. } => Done::Invites(lost()),
            Self::PutProfile { .. } => Done::ProfileSaved(lost()),
            Self::UserPage { handle, .. } => Done::ProfileLoaded { handle: handle.clone(), result: lost() },
            Self::Users { ticket, .. } => Done::Found { ticket: *ticket, result: lost() },
            Self::UserPlaylist { ticket, .. } => Done::PlaylistLoaded { ticket: *ticket, result: lost() },
            Self::FriendAction { handle, .. } => {
                Done::FriendChanged { handle: handle.clone(), result: lost() }
            }
            Self::InviteToJam { .. } => Done::Invited(lost()),
            Self::DismissInvite { .. }
            | Self::Presence { .. }
            | Self::Listen { .. }
            | Self::PutPlaylist { .. }
            | Self::DeletePlaylist { .. } => Done::Sync(lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;
    use crate::config::Playlist;
    use crate::player::{Clock, Queue};
    use crate::shared::Shared;
    use crate::testing::SocialWork;

    fn clerk() -> ClerkApp {
        ClerkApp { issuer: "https://x.clerk.accounts.dev".into(), port: 53682, session_file: None }
    }

    fn me(profile: bool, share: bool) -> Me {
        Me {
            user_id: "user_prova".into(),
            admin: false,
            profile: profile.then(|| MyProfile {
                handle: "prova".into(),
                display_name: "Prova".into(),
                avatar_url: None,
                share_activity: share,
            }),
        }
    }

    fn profile_of(handle: &str) -> Profile {
        Profile { handle: handle.into(), display_name: handle.to_uppercase(), avatar_url: None }
    }

    fn invite(code: &str, from: &str) -> JamInvite {
        JamInvite { code: code.into(), from_handle: from.into(), from_name: from.to_uppercase() }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// The domain alone, with a fake clock and a playback and a library built by hand:
    /// the passing between domains (`CoreState::handoff`) is done by the test, calling `sync` and `listen`.
    struct Rig {
        social: Social,
        playback: PlaybackView,
        library: LibraryView,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                social: Social::new(Some(clerk()), None),
                playback: PlaybackView::default(),
                library: LibraryView::default(),
                now: Instant::now(),
            }
        }

        /// A sign-in already done, as after the last step: with the profile, and sharing the activity
        /// if `share`. It also returns the effects of the entry.
        fn signed_in(share: bool) -> (Self, Vec<Effect>) {
            let mut rig = Self::new();
            let effects =
                rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(true, share) });
            (rig, effects)
        }

        fn cmd(&mut self, command: SocialCmd) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.social.apply(command, self.now, &mut effects);
            effects
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.social.done(done, self.now, &mut effects);
            effects
        }

        fn fire(&mut self, timer: Timer) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.social.fire(timer, self.now, &mut effects);
            effects
        }

        fn elapse(&mut self, by: Duration) {
            self.now += by;
        }

        /// The passing that follows each input, for the track being listened to and the playlists.
        fn sync(&mut self) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.social.sync(&self.playback, &self.library, self.now, &mut effects);
            effects
        }

        fn listen(&mut self, n: u128) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.social.listen(track(n).id, &mut effects);
            effects
        }

        fn view(&self) -> Arc<SocialView> {
            Arc::clone(&self.social.view)
        }

        fn events(&self) -> Vec<SocialEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }

        /// Track `n` in this state, with the clock at `position` (which runs if it plays).
        fn play(&mut self, n: u128, state: PlaybackState, position: Duration) {
            let mut queue = Queue::default();
            queue.replace(vec![track(n)], 0, 1);
            let clock = if state.is_playing() {
                Clock::running(position, self.now)
            } else {
                Clock::paused(position, self.now)
            };
            self.playback = PlaybackView { state, clock, queue: Arc::new(queue), ..PlaybackView::default() };
        }

        fn stop(&mut self) {
            self.playback = PlaybackView::default();
        }

        /// The library's playlists, new (another allocation) as after a real change.
        fn playlists(&mut self, playlists: Vec<Playlist>) {
            self.library.playlists = Shared::from(playlists);
        }
    }

    fn works(effects: &[Effect]) -> Vec<SocialWork> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Social(job)) => Some(SocialWork::of(job)),
                _ => None,
            })
            .collect()
    }

    fn timers(effects: &[Effect]) -> Vec<(Timer, Option<Duration>)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::Social(timer), after } => Some((*timer, Some(*after))),
                Effect::CancelTimer(TimerKey::Social(timer)) => Some((*timer, None)),
                _ => None,
            })
            .collect()
    }

    fn presence(track: u128, position: u64, paused: bool) -> SocialWork {
        SocialWork::Presence { track: Some(track_id(track)), position_ms: position, paused }
    }

    fn track_id(n: u128) -> Uuid {
        track(n).id
    }

    fn entered(rig: &mut Rig, restored: bool) -> Vec<Effect> {
        rig.done(Done::Entered {
            restored,
            result: Ok(Box::new((Session::for_tests(clerk()), ClerkUser::default(), me(true, true)))),
        })
    }

    fn password() -> Secret {
        Secret::new("una-password-segreta")
    }

    // ---------- Sign-in ----------

    /// An idle core without a sign-in arms no timer and asks for no job, whatever
    /// happens: the poll exists only with a sign-in.
    #[test]
    fn an_idle_signed_out_core_arms_nothing() {
        let mut rig = Rig::new();
        let mut effects = Vec::new();
        rig.social.start(&mut effects);
        assert!(effects.is_empty());
        assert!(rig.sync().is_empty());
        assert!(rig.listen(1).is_empty());
        assert!(rig.cmd(SocialCmd::DismissError).is_empty());
        assert!(rig.cmd(SocialCmd::RefreshFriends).is_empty());
        assert!(rig.cmd(SocialCmd::SignOut).is_empty());
        assert_eq!(rig.view().account, Account::SignedOut);
    }

    #[test]
    fn without_clerk_there_is_no_sign_in() {
        let mut social = Social::new(None, None);
        let mut effects = Vec::new();
        social.apply(
            SocialCmd::SignIn { identifier: "psy".into(), password: password() },
            Instant::now(),
            &mut effects,
        );
        assert!(effects.is_empty());
        assert_eq!(social.view.account, Account::Unavailable);
    }

    #[test]
    fn a_password_sign_in_walks_to_the_signed_in_account() {
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        assert_eq!(works(&effects), [SocialWork::SignIn]);
        let view = rig.view();
        assert_eq!(view.account, Account::Working);
        assert!(view.auth.busy && view.auth.error.is_none());

        // A second send while the first is in flight does not start.
        let again = rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        assert!(works(&again).is_empty());

        let effects = rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Done(Session::for_tests(clerk()), ClerkUser::default()))),
        });
        assert_eq!(works(&effects), [SocialWork::Enter]);
        assert!(!rig.view().auth.busy);

        let effects = entered(&mut rig, false);
        let view = rig.view();
        assert_eq!(view.account, Account::SignedIn(me(true, true)));
        assert_eq!(view.auth, AuthView::default());
        assert_eq!(rig.events(), [SocialEvent::Welcomed("Prova".into())]);
        assert_eq!(works(&effects), [SocialWork::Friends]);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))], "con un profilo il sondaggio parte");
    }

    #[test]
    fn a_code_step_asks_for_the_code_and_can_send_it_again() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignUp {
            email: "psy@example.com".into(),
            username: "psy".into(),
            password: password(),
        });
        let effects = rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Code(CodeStep::for_tests("p**@example.com")))),
        });
        assert!(works(&effects).is_empty());
        let view = rig.view();
        assert_eq!(view.account, Account::SignedOut, "il modulo aspetta il codice");
        assert!(!view.auth.busy);
        assert_eq!(view.auth.step, Some(CodeHint { sent_to: "p**@example.com".into() }));

        let effects = rig.cmd(SocialCmd::ResendCode);
        assert_eq!(works(&effects), [SocialWork::ResendCode]);
        rig.done(Done::CodeResent(Ok(CodeStep::for_tests("p**@example.com"))));
        assert_eq!(rig.events(), [SocialEvent::CodeResent("p**@example.com".into())]);

        let effects = rig.cmd(SocialCmd::SubmitCode(Secret::new("424242")));
        assert_eq!(works(&effects), [SocialWork::SubmitCode]);
        assert!(rig.view().auth.busy);

        // A wrong code leaves the step where it is and says the error: one retries.
        rig.done(Done::Stepped { attempt: 2, result: Err(AppError::AuthFailed) });
        let view = rig.view();
        assert_eq!(view.auth.error, Some(AppError::AuthFailed));
        assert!(view.auth.step.is_some(), "il passo del codice resta");
        let effects = rig.cmd(SocialCmd::SubmitCode(Secret::new("424242")));
        assert_eq!(works(&effects), [SocialWork::SubmitCode]);
        assert!(rig.view().auth.error.is_none(), "un nuovo tentativo toglie l'errore");

        // From the code one goes back to the fields.
        rig.done(Done::Stepped { attempt: 3, result: Err(AppError::AuthFailed) });
        rig.cmd(SocialCmd::CancelSignIn);
        let view = rig.view();
        assert!(view.auth.step.is_none() && view.auth.error.is_none());
        assert!(
            works(&rig.cmd(SocialCmd::SubmitCode(Secret::new("1")))).is_empty(),
            "nessun passo, niente invio"
        );
    }

    #[test]
    fn a_failed_sign_in_shows_the_error_until_the_next_attempt() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        rig.done(Done::Stepped { attempt: 1, result: Err(AppError::AuthFailed) });
        let view = rig.view();
        assert_eq!(view.account, Account::SignedOut);
        assert_eq!((view.auth.busy, view.auth.error.clone()), (false, Some(AppError::AuthFailed)));
        rig.cmd(SocialCmd::DismissError);
        assert!(rig.view().auth.error.is_none());
        let effects = rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        assert_eq!(works(&effects), [SocialWork::SignIn]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_browser_sign_in_opens_the_provider_and_waits_for_it() {
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::SignInWith(Provider::GitHub));
        assert_eq!(works(&effects), [SocialWork::OauthBegin(Provider::GitHub)]);
        assert_eq!(rig.view().auth.waiting, Some(Provider::GitHub));
        assert_eq!(rig.view().account, Account::Working);

        let oauth = Oauth::for_tests("https://github.com/login/oauth?x=1").await.unwrap();
        let effects = rig.done(Done::OauthReady { attempt: 1, result: Ok(Box::new(oauth)) });
        let opened: Vec<&str> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::OpenUrl(url) => Some(url.expose()),
                _ => None,
            })
            .collect();
        assert_eq!(opened, ["https://github.com/login/oauth?x=1"], "il browser si apre con un effetto");
        assert_eq!(works(&effects), [SocialWork::OauthWait]);
        assert!(rig.social.wait.is_some());

        // The browser's return is a step like the others.
        rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Done(Session::for_tests(clerk()), ClerkUser::default()))),
        });
        let view = rig.view();
        assert_eq!((view.auth.busy, view.auth.waiting), (false, None));
        assert!(rig.social.wait.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn giving_up_on_the_browser_frees_the_wait_and_drops_what_comes_after() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignInWith(Provider::Google));
        let oauth = Oauth::for_tests("https://accounts.google.com/x").await.unwrap();
        rig.done(Done::OauthReady { attempt: 1, result: Ok(Box::new(oauth)) });
        let wait = rig.social.wait.clone().unwrap();

        rig.cmd(SocialCmd::CancelSignIn);
        let view = rig.view();
        assert_eq!(
            (view.account.clone(), view.auth.busy, view.auth.waiting),
            (Account::SignedOut, false, None)
        );
        // Whoever waits for the return is woken: the loopback port is freed.
        tokio::time::timeout(secs(1), wait.notified()).await.expect("l'attesa del browser è stata fermata");

        // The outcome of that attempt, if it arrives all the same, is thrown away: no entering through a closed port.
        let late = rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Done(Session::for_tests(clerk()), ClerkUser::default()))),
        });
        assert!(late.is_empty());
        assert_eq!(rig.view().account, Account::SignedOut);

        // A new attempt has its own number.
        let effects = rig.cmd(SocialCmd::SignInWith(Provider::Discord));
        assert_eq!(works(&effects), [SocialWork::OauthBegin(Provider::Discord)]);
        // And the address of the old one, arrived late, does not open the browser.
        let stale = Oauth::for_tests("https://example.com/vecchio").await.unwrap();
        let effects = rig.done(Done::OauthReady { attempt: 1, result: Ok(Box::new(stale)) });
        assert!(effects.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_browser_that_does_not_open_ends_the_wait_with_an_error() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignInWith(Provider::GitHub));
        let oauth = Oauth::for_tests("https://github.com/x").await.unwrap();
        rig.done(Done::OauthReady { attempt: 1, result: Ok(Box::new(oauth)) });
        let wait = rig.social.wait.clone().unwrap();
        rig.done(Done::BrowserFailed);
        let view = rig.view();
        assert_eq!(view.auth.error, Some(AppError::AuthFailed));
        assert_eq!(
            (view.auth.busy, view.auth.waiting, view.account.clone()),
            (false, None, Account::SignedOut)
        );
        tokio::time::timeout(secs(1), wait.notified()).await.expect("l'attesa si ferma");
        // A second report of the same fault changes nothing.
        let before = rig.view();
        rig.done(Done::BrowserFailed);
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn a_provider_that_cannot_start_shows_why() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignInWith(Provider::GitHub));
        rig.done(Done::OauthReady { attempt: 1, result: Err(AppError::AuthPortBusy(53682)) });
        assert_eq!(rig.view().auth.error, Some(AppError::AuthPortBusy(53682)));
        assert_eq!(rig.view().account, Account::SignedOut);
    }

    // ---------- Saved session ----------

    #[test]
    fn a_stored_session_is_restored_silently_at_start() {
        let mut rig = Rig::new();
        rig.social =
            Social::new(Some(clerk()), None).with_session(Some(PathBuf::from("/dati/session.json")), true);
        assert_eq!(rig.view().account, Account::Working, "la prima istantanea dice già «accesso in corso»");
        let mut effects = Vec::new();
        rig.social.start(&mut effects);
        assert_eq!(works(&effects), [SocialWork::Restore]);
        assert_eq!(
            rig.social.clerk.as_ref().and_then(|c| c.session_file.clone()),
            Some(PathBuf::from("/dati/session.json"))
        );
        // Only once.
        let mut again = Vec::new();
        rig.social.start(&mut again);
        assert!(again.is_empty());

        entered(&mut rig, true);
        assert_eq!(rig.view().account, Account::SignedIn(me(true, true)));
        assert!(rig.events().is_empty(), "il ripristino non saluta");
    }

    #[test]
    fn without_a_stored_session_start_does_nothing_and_the_file_path_is_kept() {
        let mut social =
            Social::new(Some(clerk()), None).with_session(Some(PathBuf::from("/dati/session.json")), false);
        let mut effects = Vec::new();
        social.start(&mut effects);
        assert!(effects.is_empty());
        assert_eq!(social.view.account, Account::SignedOut);
        assert_eq!(
            social.clerk.as_ref().and_then(|c| c.session_file.clone()),
            Some(PathBuf::from("/dati/session.json"))
        );
    }

    #[test]
    fn a_session_clerk_no_longer_knows_is_forgotten_quietly() {
        let mut social =
            Social::new(Some(clerk()), None).with_session(Some(PathBuf::from("/dati/session.json")), true);
        let mut effects = Vec::new();
        social.start(&mut effects);
        let mut effects = Vec::new();
        social.done(
            Done::Entered { restored: true, result: Err(AppError::AuthExpired) },
            Instant::now(),
            &mut effects,
        );
        assert_eq!(social.view.account, Account::SignedOut);
        assert!(social.view.events.is_empty(), "scaduta in silenzio, senza avvisi");
        assert_eq!(works(&effects), [SocialWork::Forget], "il file della sessione si toglie dal disco");
    }

    #[test]
    fn a_restore_that_fails_for_the_network_keeps_the_file_and_says_so() {
        let mut social =
            Social::new(Some(clerk()), None).with_session(Some(PathBuf::from("/dati/session.json")), true);
        let mut effects = Vec::new();
        social.start(&mut effects);
        let mut effects = Vec::new();
        social.done(
            Done::Entered { restored: true, result: Err(AppError::Network) },
            Instant::now(),
            &mut effects,
        );
        assert_eq!(social.view.account, Account::SignedOut);
        assert!(works(&effects).is_empty(), "la sessione si riprova al prossimo avvio");
        assert_eq!(
            social.view.events.iter().map(|r| r.event.clone()).collect::<Vec<_>>(),
            [SocialEvent::Failed(AppError::Network)]
        );
    }

    #[test]
    fn a_failed_manual_sign_in_after_clerk_shows_on_the_form() {
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Done(Session::for_tests(clerk()), ClerkUser::default()))),
        });
        rig.done(Done::Entered { restored: false, result: Err(AppError::Network) });
        let view = rig.view();
        assert_eq!(view.account, Account::SignedOut);
        assert_eq!(view.auth.error, Some(AppError::Network), "sul modulo, dove l'utente sta guardando");
    }

    // ---------- Exit and expired session ----------

    #[test]
    fn signing_out_drops_everything_of_the_user_and_stops_the_poll() {
        let (mut rig, _) = Rig::signed_in(true);
        let ticket = rig.social.friends_ticket;
        rig.done(Done::Friends {
            ticket,
            result: Ok(Friends { incoming: vec![profile_of("sara")], ..Friends::default() }),
        });
        rig.done(Done::Invites(Ok(vec![invite("K7QXM3", "luca")])));
        rig.cmd(SocialCmd::OpenProfile("sara".into()));
        assert!(rig.social.polling && matches!(rig.view().friends, Load::Ready(_)));

        let effects = rig.cmd(SocialCmd::SignOut);
        assert_eq!(works(&effects), [SocialWork::SignOut]);
        assert_eq!(timers(&effects), [(Timer::Poll, None)], "uscito, nessun timer");
        let view = rig.view();
        assert_eq!(view.account, Account::SignedOut);
        assert!(matches!(view.friends, Load::Idle) && view.open.is_none() && view.jam_invites.is_empty());
        assert!(rig.social.session.is_none());

        // What was in flight for the previous user is thrown away.
        rig.done(Done::Friends { ticket, result: Ok(Friends::default()) });
        assert!(matches!(rig.view().friends, Load::Idle));
        // And signing out twice is the same as once.
        assert!(rig.cmd(SocialCmd::SignOut).is_empty());
    }

    #[test]
    fn an_expired_session_signs_out_by_itself_and_says_so() {
        let (mut rig, _) = Rig::signed_in(true);
        let effects = rig.done(Done::Sync(Err(AppError::AuthExpired)));
        assert_eq!(rig.view().account, Account::SignedOut);
        assert_eq!(rig.events(), [SocialEvent::Failed(AppError::AuthExpired)]);
        assert_eq!(works(&effects), [SocialWork::SignOut]);
        assert_eq!(timers(&effects), [(Timer::Poll, None)]);
    }

    #[test]
    fn a_failed_send_that_is_not_an_expiry_changes_nothing_on_screen() {
        let (mut rig, _) = Rig::signed_in(true);
        let before = rig.view();
        rig.done(Done::Sync(Err(AppError::Network)));
        assert!(Arc::ptr_eq(&before, &rig.view()), "un invio mancato non si vede");
        assert!(rig.events().is_empty());
    }

    // ---------- Poll ----------

    #[test]
    fn polling_runs_only_while_signed_in_with_a_profile() {
        let (mut rig, effects) = Rig::signed_in(true);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))]);
        assert_eq!(works(&effects), [SocialWork::Friends]);

        rig.elapse(POLL);
        let effects = rig.fire(Timer::Poll);
        assert_eq!(works(&effects), [SocialWork::Friends, SocialWork::Invites]);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))], "si riarma da sé finché c'è l'accesso");

        // A sign-in without a profile does not poll: the social functions want a username.
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(false, true) });
        assert!(works(&effects).is_empty() && timers(&effects).is_empty());
        // The first profile opens the social functions: friends at once, and then the poll.
        let effects = rig.done(Done::ProfileSaved(Ok(MyProfile {
            handle: "prova".into(),
            display_name: "Prova".into(),
            avatar_url: None,
            share_activity: true,
        })));
        assert_eq!(works(&effects), [SocialWork::Friends]);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))]);
    }

    #[test]
    fn a_poll_that_finds_nothing_new_publishes_nothing() {
        let (mut rig, _) = Rig::signed_in(true);
        let friends =
            Friends { friends: Vec::new(), incoming: vec![profile_of("sara")], outgoing: Vec::new() };
        let ticket = rig.social.friends_ticket;
        rig.done(Done::Friends { ticket, result: Ok(friends.clone()) });
        rig.done(Done::Invites(Ok(vec![invite("K7QXM3", "luca")])));
        let before = rig.view();

        rig.fire(Timer::Poll);
        let ticket = rig.social.friends_ticket;
        rig.done(Done::Friends { ticket, result: Ok(friends) });
        rig.done(Done::Invites(Ok(vec![invite("K7QXM3", "luca")])));
        assert!(Arc::ptr_eq(&before, &rig.view()), "gli stessi amici e inviti: la vista è la stessa");
    }

    #[test]
    fn a_failed_refresh_keeps_the_friends_and_invites_already_shown() {
        let (mut rig, _) = Rig::signed_in(true);
        let ticket = rig.social.friends_ticket;
        rig.done(Done::Friends {
            ticket,
            result: Ok(Friends { incoming: vec![profile_of("sara")], ..Friends::default() }),
        });
        rig.done(Done::Invites(Ok(vec![invite("K7QXM3", "luca")])));
        let before = rig.view();
        rig.fire(Timer::Poll);
        let ticket = rig.social.friends_ticket;
        rig.done(Done::Friends { ticket, result: Err(AppError::Network) });
        rig.done(Done::Invites(Err(AppError::Network)));
        let view = rig.view();
        assert_eq!(view.friends.ready().map(|f| f.incoming.len()), Some(1));
        assert_eq!(view.jam_invites.len(), 1);
        assert_eq!(rig.events().len(), before.events.len(), "un aggiornamento mancato non si annuncia");
    }

    #[test]
    fn only_the_answer_to_the_last_friends_request_counts() {
        let (mut rig, _) = Rig::signed_in(true);
        let first = rig.social.friends_ticket;
        rig.cmd(SocialCmd::RefreshFriends);
        let second = rig.social.friends_ticket;
        assert_ne!(first, second);
        rig.done(Done::Friends {
            ticket: first,
            result: Ok(Friends { incoming: vec![profile_of("vecchio")], ..Friends::default() }),
        });
        assert!(matches!(rig.view().friends, Load::Loading), "la risposta vecchia non vale");
        rig.done(Done::Friends { ticket: second, result: Ok(Friends::default()) });
        assert!(matches!(rig.view().friends, Load::Ready(_)));
    }

    // ---------- Jam invitations ----------

    #[test]
    fn each_jam_invite_is_announced_once() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert_eq!(rig.events(), [SocialEvent::JamInvite { from: "LUCA".into(), more: 0 }]);
        // The same invitation reread is not announced again.
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert_eq!(rig.events().len(), 1);
        // Two new together: a single notice, with "e altri".
        rig.done(Done::Invites(Ok(vec![
            invite("AAA111", "luca"),
            invite("BBB222", "sara"),
            invite("CCC333", "anna"),
        ])));
        assert_eq!(rig.events().last(), Some(&SocialEvent::JamInvite { from: "SARA".into(), more: 1 }));
        // An invitation that vanished and came back with the same code is announced again.
        rig.done(Done::Invites(Ok(Vec::new())));
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert_eq!(rig.events().len(), 3);
        assert_eq!(rig.view().jam_invites.len(), 1);
    }

    #[test]
    fn the_invite_to_the_jam_you_are_in_is_not_announced_but_is_listed() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.social.set_jam(Some("AAA111"));
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert!(rig.events().is_empty());
        assert_eq!(rig.view().jam_invites.len(), 1);
    }

    #[test]
    fn dismissing_an_invite_removes_it_at_once_and_tells_the_server() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca"), invite("BBB222", "sara")])));
        let effects = rig.cmd(SocialCmd::DismissInvite("AAA111".into()));
        assert_eq!(works(&effects), [SocialWork::DismissInvite("AAA111".into())]);
        assert_eq!(rig.view().jam_invites.iter().map(|i| i.code.as_str()).collect::<Vec<_>>(), ["BBB222"]);
    }

    #[test]
    fn inviting_friends_needs_a_jam_with_a_code() {
        let (mut rig, _) = Rig::signed_in(true);
        let effects = rig.cmd(SocialCmd::InviteToJam(vec!["luca".into()]));
        assert!(works(&effects).is_empty());
        assert_eq!(rig.events(), [SocialEvent::JamNotReady]);

        rig.social.set_jam(Some("K7QXM3"));
        let effects = rig.cmd(SocialCmd::InviteToJam(vec!["luca".into(), "sara".into()]));
        assert_eq!(
            works(&effects),
            [SocialWork::InviteToJam { code: "K7QXM3".into(), handles: vec!["luca".into(), "sara".into()] }]
        );
        assert!(rig.view().inviting);
        // Until it answers, a second send does not start.
        assert!(works(&rig.cmd(SocialCmd::InviteToJam(vec!["anna".into()]))).is_empty());
        rig.done(Done::Invited(Ok(2)));
        assert!(!rig.view().inviting);
        assert_eq!(rig.events().last(), Some(&SocialEvent::InvitesSent(2)));
    }

    // ---------- Profile ----------

    #[test]
    fn saving_the_profile_checks_it_then_reports() {
        let (mut rig, _) = Rig::signed_in(true);
        let effects = rig.cmd(SocialCmd::SaveProfile {
            handle: "ab".into(),
            display_name: "X".into(),
            share_activity: None,
        });
        assert!(works(&effects).is_empty());
        assert_eq!(rig.events(), [SocialEvent::HandleTooShort]);

        let effects = rig.cmd(SocialCmd::SaveProfile {
            handle: "nuovo".into(),
            display_name: " Nuovo ".into(),
            share_activity: Some(false),
        });
        assert_eq!(works(&effects), [SocialWork::SaveProfile]);
        assert!(rig.view().profile_saving);
        rig.done(Done::ProfileSaved(Err(AppError::Api {
            status: 409,
            code: "handle_taken".into(),
            message: "taken".into(),
        })));
        assert!(!rig.view().profile_saving);
        assert_eq!(rig.events().last(), Some(&SocialEvent::HandleTaken));

        rig.cmd(SocialCmd::SaveProfile {
            handle: "nuovo".into(),
            display_name: "Nuovo".into(),
            share_activity: Some(false),
        });
        rig.done(Done::ProfileSaved(Ok(MyProfile {
            handle: "nuovo".into(),
            display_name: "Nuovo".into(),
            avatar_url: None,
            share_activity: false,
        })));
        assert_eq!(rig.view().account.profile().map(|p| p.handle.as_str()), Some("nuovo"));
        assert_eq!(rig.events().last(), Some(&SocialEvent::ProfileSaved));
    }

    /// The profile draft that the `PutProfile` job brings to the server.
    fn draft_of(effects: &[Effect]) -> Option<&ProfileDraft> {
        effects.iter().find_map(|effect| match effect {
            Effect::Run(effect::Job::Social(Job::PutProfile { draft, .. })) => Some(draft),
            _ => None,
        })
    }

    #[test]
    fn only_clerk_avatars_are_taken_over_for_the_first_profile() {
        let save = SocialCmd::SaveProfile {
            handle: "nuovo".into(),
            display_name: "Nuovo".into(),
            share_activity: None,
        };
        for (image, kept) in [
            ("https://img.clerk.com/abc.png", true),
            ("https://evil.example/a.png", false),
            ("http://img.clerk.com/abc.png", false),
        ] {
            let mut rig = Rig::new();
            rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(false, true) });
            rig.social.clerk_user = Some(ClerkUser { image_url: Some(image.into()), ..ClerkUser::default() });
            let effects = rig.cmd(save.clone());
            let draft = draft_of(&effects).expect("la bozza parte");
            assert_eq!(draft.avatar_url.is_some(), kept, "{image}");
            assert_eq!(draft.display_name, "Nuovo");
        }
    }

    // ---------- People and profiles ----------

    #[test]
    fn searching_people_ignores_old_answers_and_empty_queries() {
        let (mut rig, _) = Rig::signed_in(true);
        assert!(works(&rig.cmd(SocialCmd::SearchUsers("  @ ".into()))).is_empty());
        assert!(matches!(rig.view().found, Load::Idle));

        let effects = rig.cmd(SocialCmd::SearchUsers("lu".into()));
        assert_eq!(works(&effects), [SocialWork::FindUsers("lu".into())]);
        assert!(rig.view().found.is_loading());
        let old = rig.social.found_ticket;
        rig.cmd(SocialCmd::SearchUsers("luc".into()));
        rig.done(Done::Found { ticket: old, result: Ok(vec![profile_of("vecchio")]) });
        assert!(rig.view().found.is_loading(), "la risposta di «lu» non vale più");
        let found: Vec<Profile> = (0..30).map(|n| profile_of(&format!("luca{n}"))).collect();
        let ticket = rig.social.found_ticket;
        rig.done(Done::Found { ticket, result: Ok(found) });
        assert_eq!(rig.view().found.ready().map(|f| f.len()), Some(MAX_FOUND));
        // Empty text clears, and a reply in flight does not put it back.
        rig.cmd(SocialCmd::SearchUsers(String::new()));
        assert!(matches!(rig.view().found, Load::Idle));
    }

    fn page(handle: &str, relation: Relation) -> UserPage {
        UserPage {
            profile: profile_of(handle),
            relation,
            friend_count: 0,
            mutual_friends: Vec::new(),
            playlists: Vec::new(),
            activity: None,
        }
    }

    #[test]
    fn an_open_profile_loads_once_and_reloads_after_a_friend_action() {
        let (mut rig, _) = Rig::signed_in(true);
        let effects = rig.cmd(SocialCmd::OpenProfile("luca".into()));
        assert_eq!(works(&effects), [SocialWork::OpenProfile("luca".into())]);
        assert!(matches!(&rig.view().open, Some((handle, load)) if handle == "luca" && load.is_loading()));
        rig.done(Done::ProfileLoaded {
            handle: "luca".into(),
            result: Ok(Box::new(OpenProfile {
                page: page("luca", Relation::None),
                recent: Arc::from(Vec::new()),
            })),
        });
        // Open and ready: reopening it does not repeat the request.
        assert!(works(&rig.cmd(SocialCmd::OpenProfile("luca".into()))).is_empty());
        // A reply for another profile does not touch the open one.
        let before = rig.view();
        rig.done(Done::ProfileLoaded { handle: "sara".into(), result: Err(AppError::Network) });
        assert!(Arc::ptr_eq(&before, &rig.view()));

        let effects = rig.cmd(SocialCmd::FriendAction { handle: "luca".into(), add: true });
        assert_eq!(works(&effects), [SocialWork::FriendAction { handle: "luca".into(), add: true }]);
        assert_eq!(rig.view().pending, ["luca"]);
        assert!(
            works(&rig.cmd(SocialCmd::FriendAction { handle: "luca".into(), add: true })).is_empty(),
            "una azione alla volta per persona"
        );
        let effects = rig.done(Done::FriendChanged { handle: "luca".into(), result: Ok(Relation::Outgoing) });
        assert!(rig.view().pending.is_empty());
        assert_eq!(rig.events(), [SocialEvent::FriendChanged(Relation::Outgoing)]);
        assert_eq!(works(&effects), [SocialWork::OpenProfile("luca".into()), SocialWork::Friends]);

        // "Riprova" on the open profile rereads it.
        assert_eq!(works(&rig.cmd(SocialCmd::ReloadProfile)), [SocialWork::OpenProfile("luca".into())]);
    }

    #[test]
    fn a_friend_playlist_answers_only_the_last_open() {
        let (mut rig, _) = Rig::signed_in(true);
        let id = Uuid::from_u128(5);
        let effects = rig.cmd(SocialCmd::OpenFriendPlaylist { handle: "luca".into(), id });
        assert_eq!(works(&effects), [SocialWork::OpenPlaylist { handle: "luca".into(), id }]);
        assert!(matches!(rig.view().playlist, Some(Load::Loading)));
        let old = rig.social.playlist_ticket;
        rig.cmd(SocialCmd::OpenFriendPlaylist { handle: "sara".into(), id });
        let playlist = Arc::new(PublicPlaylist {
            id,
            name: "Sera".into(),
            owner: profile_of("luca"),
            tracks: Arc::from(vec![track(1)]),
        });
        rig.done(Done::PlaylistLoaded { ticket: old, result: Ok(Arc::clone(&playlist)) });
        assert!(matches!(rig.view().playlist, Some(Load::Loading)));
        let ticket = rig.social.playlist_ticket;
        rig.done(Done::PlaylistLoaded { ticket, result: Ok(playlist) });
        assert!(matches!(rig.view().playlist, Some(Load::Ready(_))));
    }

    #[test]
    fn nothing_social_works_without_a_session() {
        let mut rig = Rig::new();
        for command in [
            SocialCmd::SearchUsers("luca".into()),
            SocialCmd::OpenProfile("luca".into()),
            SocialCmd::FriendAction { handle: "luca".into(), add: true },
            SocialCmd::OpenFriendPlaylist { handle: "luca".into(), id: Uuid::nil() },
            SocialCmd::InviteToJam(vec!["luca".into()]),
            SocialCmd::DismissInvite("AAA111".into()),
            SocialCmd::SaveProfile {
                handle: "luca".into(),
                display_name: "Luca".into(),
                share_activity: None,
            },
        ] {
            assert!(works(&rig.cmd(command)).is_empty());
        }
    }

    // ---------- Presence (BACKLOG 9) ----------

    /// BACKLOG 9: a track that starts sends the presence once, when it really plays; while
    /// it loads it sends nothing ("in pausa" and then "in riproduzione" were two sends per track).
    #[test]
    fn presence_goes_once_per_real_transition() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Loading { track_id: track_id(1) }, Duration::ZERO);
        assert!(works(&rig.sync()).is_empty(), "mentre si carica non si dice niente");

        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()), [presence(1, 0, false)]);

        // The tick (250 ms) and the other messages send nothing back.
        for _ in 0..8 {
            rig.elapse(Duration::from_millis(250));
            assert!(works(&rig.sync()).is_empty());
        }

        // The pause starts at once, and so does the resume.
        rig.play(1, PlaybackState::Paused, secs(2));
        assert_eq!(works(&rig.sync()), [presence(1, 2_000, true)]);
        assert!(works(&rig.sync()).is_empty());
        rig.elapse(secs(30));
        assert!(works(&rig.sync()).is_empty(), "in pausa la posizione non scorre: niente scarto");
        rig.play(1, PlaybackState::Playing, secs(2));
        assert_eq!(works(&rig.sync()), [presence(1, 2_000, false)]);

        // The next track: silent loading, then a single send.
        rig.elapse(secs(1));
        rig.play(2, PlaybackState::Loading { track_id: track_id(2) }, Duration::ZERO);
        assert!(works(&rig.sync()).is_empty());
        rig.play(2, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()), [presence(2, 0, false)]);

        // End of the queue: the track being listened to disappears from the server, once.
        rig.stop();
        assert_eq!(works(&rig.sync()), [SocialWork::Presence { track: None, position_ms: 0, paused: false }]);
        assert!(works(&rig.sync()).is_empty());
    }

    #[test]
    fn a_small_drift_sends_nothing_and_a_far_jump_waits_for_the_gap() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()).len(), 1);

        // Less than five seconds of difference: nothing.
        rig.elapse(secs(1));
        rig.playback.clock = Clock::running(secs(1) + Duration::from_secs(4), rig.now);
        assert!(works(&rig.sync()).is_empty());

        // A far jump right after a send waits: a timer, not a send.
        rig.playback.clock = Clock::running(secs(11), rig.now);
        let effects = rig.sync();
        assert!(works(&effects).is_empty());
        let waits = timers(&effects);
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].0, Timer::Presence);
        let after = waits[0].1.expect("armato");
        assert!(after > Duration::ZERO && after <= PRESENCE_GAP, "{after:?}");
        // The continuing tick does not re-arm the timer: the playback clock runs by itself.
        rig.elapse(Duration::from_millis(250));
        assert!(rig.sync().is_empty());

        // Once the wait has expired, the same step sends the new position.
        rig.elapse(after - Duration::from_millis(250));
        rig.fire(Timer::Presence);
        let sent = works(&rig.sync());
        assert!(
            matches!(&sent[..], [SocialWork::Presence { track: Some(_), paused: false, position_ms }] if (12_000..12_100).contains(position_ms)),
            "{sent:?}"
        );
        assert!(rig.sync().is_empty(), "una volta sola");
    }

    #[test]
    fn a_track_change_sends_at_once_even_inside_the_gap_and_cancels_the_wait() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        rig.sync();
        rig.elapse(Duration::from_millis(300));
        rig.playback.clock = Clock::running(secs(30), rig.now);
        let effects = rig.sync();
        assert!(timers(&effects).iter().any(|(t, a)| *t == Timer::Presence && a.is_some()));
        rig.play(2, PlaybackState::Playing, Duration::ZERO);
        let effects = rig.sync();
        assert_eq!(works(&effects), [presence(2, 0, false)]);
        assert_eq!(timers(&effects), [(Timer::Presence, None)], "il rinvio non serve più");
    }

    #[test]
    fn presence_is_not_sent_when_activity_is_private_or_without_an_account() {
        let (mut rig, _) = Rig::signed_in(false);
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert!(works(&rig.sync()).is_empty(), "il profilo non condivide");

        let mut rig = Rig::new();
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert!(rig.sync().is_empty(), "senza accesso nessuna rete");
    }

    #[test]
    fn turning_sharing_off_leaves_the_removal_to_the_server() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        rig.sync();
        rig.cmd(SocialCmd::SaveProfile {
            handle: "prova".into(),
            display_name: "Prova".into(),
            share_activity: Some(false),
        });
        rig.done(Done::ProfileSaved(Ok(MyProfile {
            handle: "prova".into(),
            display_name: "Prova".into(),
            avatar_url: None,
            share_activity: false,
        })));
        assert!(works(&rig.sync()).is_empty(), "`PUT /v1/me/profile` toglie la presenza da sé");
        // Turning sharing back on, the track that plays becomes visible again.
        rig.cmd(SocialCmd::SaveProfile {
            handle: "prova".into(),
            display_name: "Prova".into(),
            share_activity: Some(true),
        });
        rig.done(Done::ProfileSaved(Ok(MyProfile {
            handle: "prova".into(),
            display_name: "Prova".into(),
            avatar_url: None,
            share_activity: true,
        })));
        assert_eq!(works(&rig.sync()).len(), 1);
    }

    // ---------- Plays (BACKLOG 5) ----------

    /// BACKLOG 5: a track heard without a sign-in does not stay pending to start at a future sign-in:
    /// the play is thrown away, and after the sign-in nothing starts.
    #[test]
    fn a_listen_heard_while_signed_out_is_not_sent_after_signing_in() {
        let mut rig = Rig::new();
        assert!(rig.listen(1).is_empty());
        rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(true, true) });
        rig.play(2, PlaybackState::Paused, Duration::ZERO);
        let effects = rig.sync();
        assert!(!works(&effects).iter().any(|w| matches!(w, SocialWork::Listen(_))), "{effects:?}");
    }

    #[test]
    fn a_listen_heard_while_signed_in_is_sent_once() {
        let (mut rig, _) = Rig::signed_in(true);
        assert_eq!(works(&rig.listen(1)), [SocialWork::Listen(track_id(1))]);
        // A sign-in without a profile sends no plays.
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(false, true) });
        assert!(rig.listen(1).is_empty());
    }

    // ---------- Public playlists ----------

    fn playlist(name: &str, tracks: &[u128], public: bool) -> Playlist {
        let mut playlist = Playlist::new(name, tracks.iter().map(|n| track(*n)).collect());
        playlist.public = public;
        playlist
    }

    #[test]
    fn public_playlists_sync_by_fingerprint() {
        let (mut rig, _) = Rig::signed_in(true);
        let mut sera = playlist("Sera", &[1], true);
        let id = sera.id;
        rig.playlists(vec![sera.clone()]);
        assert_eq!(
            works(&rig.sync()),
            [SocialWork::PublishPlaylist { id, name: "Sera".into(), tracks: vec![track_id(1)] }]
        );
        // Frames, ticks and miscellaneous messages: nothing.
        assert!(works(&rig.sync()).is_empty());
        // Another allocation with the same content (another playlist has changed, or something else): nothing.
        rig.playlists(vec![sera.clone(), playlist("Privata", &[3], false)]);
        assert!(works(&rig.sync()).is_empty());
        // One more track: a send.
        sera.tracks.push(track(2));
        rig.playlists(vec![sera.clone()]);
        assert_eq!(
            works(&rig.sync()),
            [SocialWork::PublishPlaylist { id, name: "Sera".into(), tracks: vec![track_id(1), track_id(2)] }]
        );
        // Another name: a send.
        sera.name = "Notte".into();
        rig.playlists(vec![sera.clone()]);
        assert_eq!(works(&rig.sync()).len(), 1);
        // Gone private again: it disappears from the profile.
        sera.public = false;
        rig.playlists(vec![sera.clone()]);
        assert_eq!(works(&rig.sync()), [SocialWork::UnpublishPlaylist(id)]);
        assert!(works(&rig.sync()).is_empty());
        // Published again: again a send. Then removed altogether: it disappears.
        sera.public = true;
        rig.playlists(vec![sera.clone()]);
        assert_eq!(works(&rig.sync()).len(), 1);
        rig.playlists(Vec::new());
        assert_eq!(works(&rig.sync()), [SocialWork::UnpublishPlaylist(id)]);
    }

    #[test]
    fn a_public_playlist_is_sent_again_after_a_new_sign_in_and_never_while_signed_out() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.playlists(vec![playlist("Sera", &[1], true)]);
        assert_eq!(works(&rig.sync()).len(), 1);
        rig.cmd(SocialCmd::SignOut);
        assert!(rig.sync().is_empty(), "senza accesso niente parte");
        rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(true, true) });
        assert_eq!(works(&rig.sync()).len(), 1, "un accesso nuovo manda le pubbliche una volta");
    }

    #[test]
    fn only_the_first_five_hundred_tracks_of_a_public_playlist_go_to_the_server() {
        let (mut rig, _) = Rig::signed_in(true);
        let tracks: Vec<u128> = (1..=600).collect();
        rig.playlists(vec![playlist("Enorme", &tracks, true)]);
        let sent = works(&rig.sync());
        assert!(matches!(&sent[..], [SocialWork::PublishPlaylist { tracks, .. }] if tracks.len() == 500));
    }

    // ---------- Secrets, lost jobs, events ----------

    #[test]
    fn passwords_codes_and_urls_never_print() {
        let command = SocialCmd::SignIn { identifier: "psy".into(), password: password() };
        assert!(!format!("{command:?}").contains("segreta"));
        let command = SocialCmd::SubmitCode(Secret::new("424242"));
        assert!(!format!("{command:?}").contains("424242"));
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::SignUp {
            email: "psy@example.com".into(),
            username: "psy".into(),
            password: password(),
        });
        assert!(!format!("{effects:?}").contains("segreta"), "nemmeno nel lavoro che parte");
        let url = Effect::OpenUrl(Secret::new("https://github.com/login?token=abc"));
        assert!(!format!("{url:?}").contains("abc"));
        let (rig, _) = Rig::signed_in(true);
        let printed = format!("{:?}", rig.social);
        assert!(!printed.contains("token-di-prova") && !printed.contains("client-di-prova"), "{printed}");
    }

    /// A job that panics does not leave the domain waiting: every job has its outcome.
    #[test]
    fn a_lost_job_never_leaves_the_form_stuck() {
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::SignIn { identifier: "psy".into(), password: password() });
        let Some(Effect::Run(effect::Job::Social(job))) = effects.into_iter().next() else {
            panic!("manca il lavoro");
        };
        rig.done(job.lost());
        let view = rig.view();
        assert_eq!(
            (view.auth.busy, view.account.clone(), view.auth.error.clone()),
            (false, Account::SignedOut, Some(AppError::AuthFailed))
        );

        // A lost read too fails as without network, without deleting what is there.
        let (mut rig, effects) = Rig::signed_in(true);
        let Some(Effect::Run(effect::Job::Social(job))) =
            effects.into_iter().find(|e| matches!(e, Effect::Run(_)))
        else {
            panic!("manca il lavoro");
        };
        rig.done(job.lost());
        assert!(matches!(rig.view().friends, Load::Failed(AppError::Network)));
    }

    #[test]
    fn events_keep_a_numbered_ring_of_the_last_thirty_two() {
        let (mut rig, _) = Rig::signed_in(true);
        for _ in 0..40 {
            rig.cmd(SocialCmd::InviteToJam(vec!["luca".into()]));
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        let seqs: Vec<u64> = view.events.iter().map(|r| r.seq).collect();
        assert_eq!(seqs.first(), Some(&9));
        assert_eq!(seqs.last(), Some(&40));
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
    }
}
