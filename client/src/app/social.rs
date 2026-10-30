//! Account e amici: la schermata «Accedi a Psytify» (password, codice via email, GitHub, Google e
//! Discord), il profilo con il nome utente, gli amici con le richieste, le pagine degli altri e le
//! loro playlist pubbliche.
//!
//! Tre cose vanno al server da sole, senza che nessun punto dell'app debba ricordarsene
//! (`App::social_sync`, chiamata dopo ogni messaggio): il brano in ascolto (quando cambia brano, va
//! in pausa, riprende o si salta lontano), gli ascolti (un brano sentito per 30 secondi) e le
//! playlist rese pubbliche (quando cambiano nome o brani). Il confronto costa qualche campo per
//! messaggio; la rete parte solo quando qualcosa è cambiato.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use uuid::Uuid;

use super::{Account, App, Load, Message, Notice, Screen, into_load};
use crate::api::social::{
    Friends, JamInvite, MyProfile, Profile, ProfileDraft, PublicPlaylist, Relation, UserPage, suggest_handle,
};
use crate::api::{ApiClient, Me, Track};
use crate::auth::{self, ClerkUser, CodeStep, Provider, Session, Step};
use crate::error::AppError;

/// Un salto oltre questo scarto rispetto alla posizione attesa aggiorna il brano in ascolto.
const PRESENCE_DRIFT: Duration = Duration::from_secs(5);
/// Fra due aggiornamenti del brano in ascolto, almeno questo tempo (un salto dopo l'altro).
const PRESENCE_GAP: Duration = Duration::from_secs(2);
/// Utenti cercati mostrati.
pub const MAX_FOUND: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    SignIn,
    SignUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Identifier,
    Email,
    Username,
    Password,
    Code,
}

/// Il modulo di accesso. La password resta solo qui e sparisce appena il passo finisce.
#[derive(Debug, Default)]
pub struct SignInForm {
    pub mode: AuthMode,
    pub identifier: String,
    pub email: String,
    pub username: String,
    pub password: String,
    pub code: String,
    /// Clerk ha mandato un codice: si chiede quello.
    pub step: Option<CodeStep>,
    pub busy: bool,
    /// Il provider aperto nel browser, mentre si aspetta il ritorno.
    pub waiting: Option<Provider>,
    pub error: Option<String>,
}

impl SignInForm {
    pub fn can_submit(&self) -> bool {
        if self.busy {
            return false;
        }
        if self.step.is_some() {
            return self.code.trim().len() >= 6;
        }
        match self.mode {
            AuthMode::SignIn => !self.identifier.trim().is_empty() && !self.password.is_empty(),
            AuthMode::SignUp => self.email.contains('@') && self.password.chars().count() >= 8,
        }
    }

    fn clear_secrets(&mut self) {
        self.password.clear();
        self.code.clear();
    }
}

/// Un profilo aperto, con gli ascolti recenti già pronti per la coda.
#[derive(Debug, Clone)]
pub struct OpenProfile {
    pub page: UserPage,
    pub recent: Arc<[Track]>,
}

/// Le modifiche al proprio profilo, nelle impostazioni.
#[derive(Debug, Clone, Default)]
pub struct ProfileEdit {
    pub handle: String,
    pub display_name: String,
    pub saving: bool,
}

/// L'ultimo brano in ascolto mandato al server.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SentPresence {
    track: Uuid,
    paused: bool,
    position: Duration,
    at: Instant,
}

#[derive(Debug, Default)]
pub struct SocialState {
    pub form: SignInForm,
    pub session: Option<Session>,
    /// L'utente di Clerk dell'ultimo accesso: da qui si propone il profilo.
    pub clerk_user: Option<ClerkUser>,
    pub edit: ProfileEdit,
    pub friends: Load<Friends>,
    pub query: String,
    pub found: Load<Vec<Profile>>,
    /// Il profilo aperto: il nome utente e la pagina.
    pub open: Option<(String, Load<OpenProfile>)>,
    /// La playlist pubblica aperta.
    pub playlist: Option<Load<Arc<PublicPlaylist>>>,
    /// Nomi utente con un'azione d'amicizia in corso.
    pub pending: Vec<String>,
    /// Inviti alle jam ricevuti dagli amici, dal più recente.
    pub jam_invites: Vec<JamInvite>,
    /// Codici degli inviti già annunciati con un avviso: ognuno si annuncia una volta sola.
    announced_invites: Vec<String>,
    /// Amici scelti nella schermata Jam, da invitare, e l'invio in corso.
    pub invitees: Vec<String>,
    pub inviting: bool,
    sent: Option<SentPresence>,
    /// Il brano appena contato come ascoltato, da mandare.
    pub(super) listen_due: Option<Uuid>,
    /// Impronta delle playlist pubbliche già mandate.
    published: HashMap<Uuid, u64>,
}

/// Impronta di nome e brani: cambia quando la playlist pubblica va aggiornata sul server.
fn fingerprint(name: &str, tracks: &[Track]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    for track in tracks {
        track.id.hash(&mut hasher);
    }
    hasher.finish()
}

/// Un'operazione che vuole il token: lo chiede alla sessione (rinnovandolo se scaduto), poi chiama.
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

/// Dopo l'accesso: l'utente sul server e, la prima volta, il profilo proposto da Clerk. Un nome
/// utente già preso riceve un numero in fondo.
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
            // Un profilo che non si crea non impedisce di entrare: lo si sceglie nelle impostazioni.
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

impl App {
    pub fn signed_in_profile(&self) -> Option<&MyProfile> {
        match &self.account {
            Account::SignedIn(me) => me.profile.as_ref(),
            _ => None,
        }
    }

    /// Richieste d'amicizia ricevute: il numero accanto ad «Amici».
    pub fn friend_requests(&self) -> usize {
        self.social.friends.ready().map_or(0, |friends| friends.incoming.len())
    }

    // ---------- Accesso ----------

    /// All'avvio: la sessione salvata, se Clerk la considera ancora attiva.
    pub(super) fn restore_session(&mut self) -> Task<Message> {
        let Some(app) = self.clerk_app.clone() else { return Task::none() };
        if auth::session_path().is_none_or(|path| !path.exists()) {
            return Task::none();
        }
        self.account = Account::Working;
        let api = self.api();
        Task::perform(
            async move {
                let (session, user) = auth::restore(app).await?;
                let me = enter(api, session.clone(), user.clone()).await?;
                Ok((session, user, me))
            },
            |result| Message::SignedIn { restored: true, result },
        )
    }

    pub(super) fn open_sign_in(&mut self) -> Task<Message> {
        if self.clerk_app.is_none() {
            self.notice = Some(Notice::Error(AppError::AuthNotConfigured.to_string()));
            return Task::none();
        }
        self.social.form.error = None;
        self.open(Screen::SignIn)
    }

    pub(super) fn auth_input(&mut self, field: AuthField, value: String) -> Task<Message> {
        let form = &mut self.social.form;
        let target = match field {
            AuthField::Identifier => &mut form.identifier,
            AuthField::Email => &mut form.email,
            AuthField::Username => &mut form.username,
            AuthField::Password => &mut form.password,
            AuthField::Code => &mut form.code,
        };
        // Niente a capo né caratteri di controllo: arrivano solo incollando.
        *target = value.chars().filter(|c| !c.is_control()).take(256).collect();
        form.error = None;
        Task::none()
    }

    pub(super) fn auth_mode(&mut self, mode: AuthMode) -> Task<Message> {
        let form = &mut self.social.form;
        form.mode = mode;
        form.step = None;
        form.error = None;
        form.clear_secrets();
        Task::none()
    }

    pub(super) fn auth_submit(&mut self) -> Task<Message> {
        let Some(app) = self.clerk_app.clone() else { return Task::none() };
        let form = &mut self.social.form;
        if !form.can_submit() {
            return Task::none();
        }
        form.busy = true;
        form.error = None;
        let task = match (&form.step, form.mode) {
            (Some(step), _) => {
                let (step, code) = (step.clone(), std::mem::take(&mut form.code));
                Task::perform(auth::submit_code(app, step, code), Message::AuthStepped)
            }
            (None, AuthMode::SignIn) => {
                let (identifier, password) = (form.identifier.clone(), std::mem::take(&mut form.password));
                Task::perform(auth::sign_in(app, identifier, password), Message::AuthStepped)
            }
            (None, AuthMode::SignUp) => {
                let (email, username, password) =
                    (form.email.clone(), form.username.clone(), std::mem::take(&mut form.password));
                Task::perform(auth::sign_up(app, email, username, password), Message::AuthStepped)
            }
        };
        self.account = Account::Working;
        task
    }

    pub(super) fn auth_provider(&mut self, provider: Provider) -> Task<Message> {
        let Some(app) = self.clerk_app.clone() else { return Task::none() };
        let form = &mut self.social.form;
        if form.busy {
            return Task::none();
        }
        form.busy = true;
        form.waiting = Some(provider);
        form.error = None;
        self.account = Account::Working;
        Task::perform(auth::sign_in_with(app, provider), Message::AuthStepped)
    }

    pub(super) fn auth_resend(&mut self) -> Task<Message> {
        let Some(app) = self.clerk_app.clone() else { return Task::none() };
        let Some(step) = self.social.form.step.clone() else { return Task::none() };
        Task::perform(auth::resend_code(app, step), Message::AuthResent)
    }

    pub(super) fn auth_resent(&mut self, result: Result<CodeStep, AppError>) -> Task<Message> {
        match result {
            Ok(step) => {
                self.notice = Some(Notice::Info(format!("Nuovo codice inviato a {}", step.sent_to)));
                self.social.form.step = Some(step);
            }
            Err(err) => self.social.form.error = Some(err.to_string()),
        }
        Task::none()
    }

    /// Torna dal codice ai campi.
    pub(super) fn auth_back(&mut self) -> Task<Message> {
        let form = &mut self.social.form;
        form.step = None;
        form.error = None;
        form.clear_secrets();
        Task::none()
    }

    pub(super) fn auth_stepped(&mut self, result: Result<Step, AppError>) -> Task<Message> {
        let form = &mut self.social.form;
        form.busy = false;
        form.waiting = None;
        match result {
            Ok(Step::Code(step)) => {
                form.code.clear();
                form.step = Some(step);
                self.account = Account::SignedOut;
                Task::none()
            }
            Ok(Step::Done(session, user)) => {
                form.clear_secrets();
                form.step = None;
                let api = self.api();
                Task::perform(
                    async move {
                        let me = enter(api, session.clone(), user.clone()).await?;
                        Ok((session, user, me))
                    },
                    |result| Message::SignedIn { restored: false, result },
                )
            }
            Err(err) => {
                form.error = Some(err.to_string());
                self.account = Account::SignedOut;
                Task::none()
            }
        }
    }

    pub(super) fn signed_in(
        &mut self,
        restored: bool,
        result: Result<(Session, ClerkUser, Me), AppError>,
    ) -> Task<Message> {
        match result {
            Ok((session, user, me)) => {
                if !restored {
                    let name = me.profile.as_ref().map_or("di nuovo", |p| p.display_name.as_str());
                    self.notice = Some(Notice::Info(format!("Ciao {name}!")));
                }
                self.social.edit = me.profile.as_ref().map_or_else(ProfileEdit::default, |p| ProfileEdit {
                    handle: p.handle.clone(),
                    display_name: p.display_name.clone(),
                    saving: false,
                });
                self.account = Account::SignedIn(me);
                self.social.session = Some(session);
                self.social.clerk_user = Some(user);
                self.social.form = SignInForm::default();
                self.social.published.clear();
                let leave = if self.screen == Screen::SignIn { self.go_back_or_home() } else { Task::none() };
                let avatar = self.signed_in_profile().and_then(|p| p.avatar_url.clone());
                Task::batch([leave, self.load_friends(), self.fetch_covers(avatar.into_iter().collect())])
            }
            Err(err) => {
                self.account = Account::SignedOut;
                self.social.session = None;
                if !restored || err != AppError::AuthExpired {
                    if self.screen == Screen::SignIn {
                        self.social.form.error = Some(err.to_string());
                    } else {
                        self.notice = Some(Notice::Error(err.to_string()));
                    }
                }
                // Una sessione che Clerk non riconosce più non serve: la si toglie dal disco.
                if err == AppError::AuthExpired {
                    return Task::perform(auth::forget_session(), Message::SessionStored);
                }
                Task::none()
            }
        }
    }

    pub(super) fn sign_out(&mut self) -> Task<Message> {
        self.account = if self.clerk_app.is_some() { Account::SignedOut } else { Account::Unavailable };
        let session = self.social.session.take();
        let api = self.api();
        let form = std::mem::take(&mut self.social.form);
        self.social = SocialState { form, ..SocialState::default() };
        match session {
            Some(session) => Task::perform(
                async move {
                    // Chi esce non resta «in ascolto» per gli amici.
                    if let (Ok(api), Ok(token)) = (api, session.bearer().await) {
                        let _ = api.delete_presence(&token).await;
                    }
                    session.sign_out().await
                },
                Message::SessionStored,
            ),
            None => Task::perform(auth::forget_session(), Message::SessionStored),
        }
    }

    fn go_back_or_home(&mut self) -> Task<Message> {
        if self.can_go_back() { self.update(Message::GoBack) } else { self.open(Screen::Home) }
    }

    /// Esito di una chiamata che non mostra nulla: solo la sessione scaduta conta.
    pub(super) fn social_synced(&mut self, result: Result<(), AppError>) -> Task<Message> {
        if result == Err(AppError::AuthExpired) {
            self.notice = Some(Notice::Error(AppError::AuthExpired.to_string()));
            return self.sign_out();
        }
        Task::none()
    }

    // ---------- Profilo ----------

    pub(super) fn profile_input(&mut self, handle: Option<String>, name: Option<String>) -> Task<Message> {
        if let Some(handle) = handle {
            self.social.edit.handle = handle
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(30)
                .collect::<String>()
                .to_lowercase();
        }
        if let Some(name) = name {
            self.social.edit.display_name = name.chars().filter(|c| !c.is_control()).take(64).collect();
        }
        Task::none()
    }

    /// Salva nome utente, nome e condivisione dell'attività.
    pub(super) fn save_profile(&mut self, share: Option<bool>) -> Task<Message> {
        let (Some(session), Account::SignedIn(me)) = (self.social.session.clone(), &self.account) else {
            return Task::none();
        };
        let edit = &mut self.social.edit;
        if edit.handle.len() < 3 || edit.display_name.trim().is_empty() {
            self.notice = Some(Notice::Error("Il nome utente vuole almeno 3 caratteri.".into()));
            return Task::none();
        }
        edit.saving = true;
        let avatar_url = me
            .profile
            .as_ref()
            .and_then(|p| p.avatar_url.clone())
            .or_else(|| self.social.clerk_user.as_ref().and_then(|u| u.image_url.clone()))
            .filter(|url| url.starts_with("https://img.clerk.com/"));
        let draft = ProfileDraft {
            handle: edit.handle.clone(),
            display_name: edit.display_name.trim().to_owned(),
            avatar_url,
            share_activity: share.or(me.profile.as_ref().map(|p| p.share_activity)),
        };
        Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                api.put_profile(&token, &draft).await
            }),
            Message::ProfileSaved,
        )
    }

    pub(super) fn profile_saved(&mut self, result: Result<MyProfile, AppError>) -> Task<Message> {
        self.social.edit.saving = false;
        match result {
            Ok(profile) => {
                self.social.edit.handle = profile.handle.clone();
                self.social.edit.display_name = profile.display_name.clone();
                if !profile.share_activity {
                    self.social.sent = None;
                }
                if let Account::SignedIn(me) = &mut self.account {
                    me.profile = Some(profile);
                }
                self.notice = Some(Notice::Info("Profilo salvato".into()));
                let avatar = self.signed_in_profile().and_then(|p| p.avatar_url.clone());
                self.fetch_covers(avatar.into_iter().collect())
            }
            Err(AppError::Api { status: 409, .. }) => {
                self.notice = Some(Notice::Error("Questo nome utente è già preso.".into()));
                Task::none()
            }
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                self.social_synced(Err(err))
            }
        }
    }

    // ---------- Amici e profili ----------

    pub(super) fn load_friends(&mut self) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        if self.signed_in_profile().is_none() {
            return Task::none();
        }
        if !matches!(self.social.friends, Load::Ready(_)) {
            self.social.friends = Load::Loading;
        }
        Task::perform(
            with_token(session, self.api(), |api, token| async move { api.friends(&token).await }),
            Message::FriendsLoaded,
        )
    }

    pub(super) fn friends_loaded(&mut self, result: Result<Friends, AppError>) -> Task<Message> {
        let covers = match &result {
            Ok(friends) => {
                let urls = friends
                    .friends
                    .iter()
                    .flat_map(|f| {
                        f.profile
                            .avatar_url
                            .iter()
                            .chain(f.now_playing.iter().filter_map(|n| n.track.cover_url.as_ref()))
                    })
                    .chain(
                        friends
                            .incoming
                            .iter()
                            .chain(&friends.outgoing)
                            .filter_map(|p| p.avatar_url.as_ref()),
                    )
                    .cloned()
                    .collect();
                self.fetch_covers(urls)
            }
            Err(_) => Task::none(),
        };
        let expired = result == Err(AppError::AuthExpired);
        // Un aggiornamento periodico che fallisce non cancella la lista già mostrata.
        if result.is_err() && matches!(self.social.friends, Load::Ready(_)) && !expired {
            return covers;
        }
        self.social.friends = into_load(result);
        if expired {
            return self.social_synced(Err(AppError::AuthExpired));
        }
        covers
    }

    /// Ogni tanto: richieste d'amicizia e inviti alle jam arrivano senza che l'utente faccia nulla.
    pub(super) fn poll_social(&mut self) -> Task<Message> {
        Task::batch([self.load_friends(), self.load_jam_invites()])
    }

    /// Serve il timer del polling: solo con un accesso e un profilo.
    pub(super) fn polls_social(&self) -> bool {
        self.social.session.is_some() && self.signed_in_profile().is_some()
    }

    // ---------- Inviti alla jam ----------

    pub(super) fn load_jam_invites(&mut self) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        if self.signed_in_profile().is_none() {
            return Task::none();
        }
        Task::perform(
            with_token(session, self.api(), |api, token| async move { api.jam_invites(&token).await }),
            Message::JamInvitesLoaded,
        )
    }

    pub(super) fn jam_invites_loaded(&mut self, result: Result<Vec<JamInvite>, AppError>) -> Task<Message> {
        let invites = match result {
            Ok(invites) => invites,
            Err(AppError::AuthExpired) => return self.social_synced(Err(AppError::AuthExpired)),
            // Un aggiornamento mancato si riprova al prossimo giro.
            Err(_) => return Task::none(),
        };
        // Chi è già in una jam non ha bisogno dell'invito a una stanza in cui si trova.
        let current = self.jam.as_ref().and_then(|jam| jam.code.clone());
        let fresh: Vec<&JamInvite> = invites
            .iter()
            .filter(|i| Some(&i.code) != current.as_ref() && !self.social.announced_invites.contains(&i.code))
            .collect();
        if let Some(first) = fresh.first() {
            let more = fresh.len() - 1;
            let text = if more == 0 {
                format!("{} ti invita in una jam", first.from_name)
            } else {
                format!("{} e altri {more} ti invitano in una jam", first.from_name)
            };
            self.notice = Some(Notice::Info(text));
        }
        let announced: Vec<String> = fresh.iter().map(|i| i.code.clone()).collect();
        self.social.announced_invites.extend(announced);
        // Si ricordano solo i codici ancora in giro: la lista non cresce senza fondo.
        self.social.announced_invites.retain(|code| invites.iter().any(|i| i.code == *code));
        self.social.jam_invites = invites;
        Task::none()
    }

    /// Aggiunge o toglie un amico dalla scelta di chi invitare.
    pub(super) fn toggle_invitee(&mut self, handle: String) {
        if let Some(at) = self.social.invitees.iter().position(|h| *h == handle) {
            self.social.invitees.remove(at);
        } else if self.social.invitees.len() < crate::jam::MAX_MEMBERS {
            self.social.invitees.push(handle);
        }
    }

    /// Manda l'invito agli amici scelti, per la jam in cui si è.
    pub(super) fn send_jam_invites(&mut self) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        let Some(code) = self.jam.as_ref().and_then(|jam| jam.code.clone()) else {
            self.notice = Some(Notice::Info("Aspetta che la jam sia pronta".into()));
            return Task::none();
        };
        if self.social.invitees.is_empty() || self.social.inviting {
            return Task::none();
        }
        self.social.inviting = true;
        let handles = self.social.invitees.clone();
        Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                api.invite_to_jam(&token, &code, &handles).await
            }),
            Message::JamInviteSent,
        )
    }

    pub(super) fn jam_invite_sent(&mut self, result: Result<usize, AppError>) -> Task<Message> {
        self.social.inviting = false;
        match result {
            Ok(count) => {
                self.social.invitees.clear();
                let text =
                    if count == 1 { "Invito mandato".to_owned() } else { format!("{count} inviti mandati") };
                self.notice = Some(Notice::Info(text));
                Task::none()
            }
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                self.social_synced(Err(err))
            }
        }
    }

    /// Entra nella jam di un invito e lo toglie dall'elenco.
    pub(super) fn accept_jam_invite(&mut self, code: String) -> Task<Message> {
        if self.jam.is_some() {
            self.notice = Some(Notice::Info("Sei già in una jam: esci prima da quella".into()));
            return Task::none();
        }
        let dismiss = self.dismiss_jam_invite(code.clone());
        Task::batch([dismiss, self.start_jam(crate::jam::Target::Join(code))])
    }

    /// Toglie l'invito subito dalla vista e lo fa togliere anche dal server.
    pub(super) fn dismiss_jam_invite(&mut self, code: String) -> Task<Message> {
        self.social.jam_invites.retain(|i| i.code != code);
        let Some(session) = self.social.session.clone() else { return Task::none() };
        Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                api.dismiss_jam_invite(&token, &code).await
            }),
            Message::JamInviteDismissed,
        )
    }

    pub(super) fn user_query(&mut self, query: String) -> Task<Message> {
        self.social.query = query.chars().filter(|c| !c.is_control()).take(100).collect();
        if self.social.query.trim().trim_start_matches('@').is_empty() {
            self.social.found = Load::Idle;
        }
        Task::none()
    }

    pub(super) fn search_users(&mut self) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        let query = self.social.query.trim().to_owned();
        if query.trim_start_matches('@').is_empty() {
            return Task::none();
        }
        self.social.found = Load::Loading;
        Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                let mut found = api.search_users(&token, &query).await?;
                found.truncate(MAX_FOUND);
                Ok(found)
            }),
            Message::UsersFound,
        )
    }

    pub(super) fn users_found(&mut self, result: Result<Vec<Profile>, AppError>) -> Task<Message> {
        let covers = match &result {
            Ok(found) => self.fetch_covers(found.iter().filter_map(|p| p.avatar_url.clone()).collect()),
            Err(_) => Task::none(),
        };
        self.social.found = into_load(result);
        covers
    }

    pub(super) fn open_profile(&mut self, handle: String) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return self.open_sign_in() };
        let same =
            self.social.open.as_ref().is_some_and(|(open, load)| *open == handle && !load.needs_load());
        let navigate = self.open(Screen::Profile);
        if same {
            return navigate;
        }
        self.social.open = Some((handle.clone(), Load::Loading));
        let wanted = handle.clone();
        let load =
            Task::perform(
                with_token(session, self.api(), move |api, token| async move {
                    api.user_page(&token, &wanted).await
                }),
                move |result| Message::ProfileLoaded(handle.clone(), result),
            );
        Task::batch([navigate, load])
    }

    pub(super) fn profile_loaded(
        &mut self,
        handle: String,
        result: Result<UserPage, AppError>,
    ) -> Task<Message> {
        if self.social.open.as_ref().is_none_or(|(open, _)| *open != handle) {
            return Task::none();
        }
        let covers = match &result {
            Ok(page) => {
                let mut urls: Vec<String> = page.profile.avatar_url.iter().cloned().collect();
                urls.extend(page.mutual_friends.iter().filter_map(|p| p.avatar_url.clone()));
                if let Some(activity) = &page.activity {
                    urls.extend(activity.now_playing.iter().filter_map(|n| n.track.cover_url.clone()));
                    urls.extend(activity.recent.iter().filter_map(|l| l.track.cover_url.clone()));
                }
                self.fetch_covers(urls)
            }
            Err(_) => Task::none(),
        };
        let opened = result.map(|page| {
            let recent = page
                .activity
                .as_ref()
                .map(|a| a.recent.iter().map(|l| l.track.clone()).collect::<Vec<_>>())
                .unwrap_or_default();
            OpenProfile { page, recent: Arc::from(recent) }
        });
        if let Some((_, load)) = &mut self.social.open {
            *load = into_load(opened);
        }
        covers
    }

    /// Chiede o accetta l'amicizia (`add`), oppure la toglie, la ritira o la rifiuta.
    pub(super) fn friend_action(&mut self, handle: String, add: bool) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        if self.social.pending.contains(&handle) {
            return Task::none();
        }
        self.social.pending.push(handle.clone());
        let target = handle.clone();
        Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                if add {
                    api.add_friend(&token, &target).await
                } else {
                    api.remove_friend(&token, &target).await.map(|()| Relation::None)
                }
            }),
            move |result| Message::FriendChanged(handle.clone(), result),
        )
    }

    pub(super) fn friend_changed(
        &mut self,
        handle: String,
        result: Result<Relation, AppError>,
    ) -> Task<Message> {
        self.social.pending.retain(|h| *h != handle);
        match result {
            Ok(relation) => {
                let message = match relation {
                    Relation::Friend => "Ora siete amici",
                    Relation::Outgoing => "Richiesta inviata",
                    _ => "Fatto",
                };
                self.notice = Some(Notice::Info(message.into()));
                let reopen = match &self.social.open {
                    Some((open, _)) if *open == handle && self.screen == Screen::Profile => {
                        self.social.open = None;
                        self.open_profile(handle)
                    }
                    _ => Task::none(),
                };
                Task::batch([reopen, self.load_friends()])
            }
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                self.social_synced(Err(err))
            }
        }
    }

    pub(super) fn open_friend_playlist(&mut self, handle: String, id: Uuid) -> Task<Message> {
        let Some(session) = self.social.session.clone() else { return Task::none() };
        self.social.playlist = Some(Load::Loading);
        let navigate = self.open(Screen::FriendPlaylist);
        let load = Task::perform(
            with_token(session, self.api(), move |api, token| async move {
                api.user_playlist(&token, &handle, id).await.map(Arc::new)
            }),
            Message::FriendPlaylistLoaded,
        );
        Task::batch([navigate, load])
    }

    pub(super) fn friend_playlist_loaded(
        &mut self,
        result: Result<Arc<PublicPlaylist>, AppError>,
    ) -> Task<Message> {
        let covers = match &result {
            Ok(playlist) => {
                let mut urls: Vec<String> =
                    playlist.tracks.iter().filter_map(|t| t.cover_url.clone()).collect();
                urls.extend(playlist.owner.avatar_url.clone());
                self.fetch_covers(urls)
            }
            Err(_) => Task::none(),
        };
        self.social.playlist = Some(into_load(result));
        covers
    }

    /// Rende pubblica (o di nuovo privata) una playlist locale.
    pub(super) fn set_playlist_public(&mut self, id: Uuid, public: bool) -> Task<Message> {
        let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == id) else { return Task::none() };
        if playlist.public == public {
            return Task::none();
        }
        playlist.public = public;
        self.notice = Some(Notice::Info(
            if public { "La playlist ora è visibile sul tuo profilo" } else { "La playlist ora è privata" }
                .into(),
        ));
        let unpublish = match (public, self.social.session.clone()) {
            (false, Some(session)) => {
                self.social.published.remove(&id);
                Task::perform(
                    with_token(session, self.api(), move |api, token| async move {
                        api.delete_playlist(&token, id).await
                    }),
                    Message::SocialSynced,
                )
            }
            _ => Task::none(),
        };
        Task::batch([unpublish, self.persist()])
    }

    // ---------- Sincronizzazione ----------

    /// Dopo ogni messaggio: manda al server ciò che è cambiato nel brano in ascolto, negli ascolti e
    /// nelle playlist pubbliche. `None` se non c'è niente da mandare (quasi sempre).
    pub(super) fn social_sync(&mut self) -> Option<Task<Message>> {
        let session = self.social.session.clone()?;
        let share = self.signed_in_profile()?.share_activity;
        let mut tasks = Vec::new();

        let want = self
            .queue
            .current()
            .filter(|_| share)
            .map(|track| (track.id, !self.playback.is_playing(), self.playback.position));
        let changed = match (want, self.social.sent) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some((track, paused, position)), Some(sent)) => {
                let expected = if sent.paused { sent.position } else { sent.position + sent.at.elapsed() };
                let drift = position.abs_diff(expected);
                track != sent.track
                    || paused != sent.paused
                    || (drift > PRESENCE_DRIFT && sent.at.elapsed() > PRESENCE_GAP)
            }
        };
        if changed {
            self.social.sent = want.map(|(track, paused, position)| SentPresence {
                track,
                paused,
                position,
                at: Instant::now(),
            });
            let task = match want {
                Some((track, paused, position)) => {
                    let millis = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
                    with_token(session.clone(), self.api(), move |api, token| async move {
                        api.put_presence(&token, track, millis, paused).await
                    })
                    .boxed_task()
                }
                None => with_token(session.clone(), self.api(), |api, token| async move {
                    api.delete_presence(&token).await
                })
                .boxed_task(),
            };
            tasks.push(task);
        }

        if let Some(track) = self.social.listen_due.take() {
            tasks.push(
                with_token(session, self.api(), move |api, token| async move {
                    api.post_listen(&token, track).await
                })
                .boxed_task(),
            );
        }

        tasks.extend(self.sync_public_playlists());
        (!tasks.is_empty()).then(|| Task::batch(tasks))
    }

    /// Le playlist pubbliche cambiate dall'ultimo invio. L'impronta si calcola solo sulle playlist
    /// pubbliche, che sono poche.
    fn sync_public_playlists(&mut self) -> Vec<Task<Message>> {
        let Some(session) = self.social.session.clone() else { return Vec::new() };
        let mut tasks = Vec::new();
        for playlist in self.playlists.iter().filter(|p| p.public) {
            let print = fingerprint(&playlist.name, &playlist.tracks);
            if self.social.published.get(&playlist.id) == Some(&print) {
                continue;
            }
            self.social.published.insert(playlist.id, print);
            let (id, name) = (playlist.id, playlist.name.clone());
            let tracks: Vec<Uuid> = playlist.tracks.iter().map(|t| t.id).take(500).collect();
            tasks.push(
                with_token(session.clone(), self.api.clone(), move |api, token| async move {
                    api.put_playlist(&token, id, &name, &tracks).await
                })
                .boxed_task(),
            );
        }
        tasks
    }
}

// ---------- Discord ----------

impl App {
    /// L'Application ID di Discord: quello di `config.json`, oppure quello della build.
    fn discord_app_id(&self) -> &str {
        let chosen = self.discord_app_id.trim();
        if chosen.is_empty() { crate::config::DEFAULT_DISCORD_APP_ID } else { chosen }
    }

    /// Accende o spegne il collegamento con Discord secondo la scelta dell'utente. Spento, il
    /// thread si ferma e l'attività sparisce da Discord.
    pub(super) fn start_discord(&mut self) {
        #[cfg(not(target_os = "android"))]
        {
            if !self.discord_presence {
                self.discord = None;
            } else if self.discord.is_none() {
                self.discord = crate::discord::Presence::start(self.discord_app_id());
            }
        }
    }

    /// `true` se la build o la configurazione hanno un'app Discord: senza, l'interruttore non c'è.
    pub fn discord_available(&self) -> bool {
        cfg!(not(target_os = "android")) && !self.discord_app_id().is_empty()
    }

    /// Il brano in corso su Discord: chiamata con il widget del sistema, quando cambia brano, va in
    /// pausa o si salta. In pausa l'attività sparisce, come nelle altre app di musica.
    pub(super) fn publish_discord(&self) {
        #[cfg(not(target_os = "android"))]
        {
            let Some(presence) = &self.discord else { return };
            let activity = self.queue.current().filter(|_| self.playback.is_playing()).map(|track| {
                crate::discord::Activity::playing(
                    &track.title,
                    &track.artist.name,
                    track.album.as_ref().map(|album| album.title.as_str()),
                    track.cover_url.as_deref(),
                    self.playback.position,
                    track.duration(),
                )
            });
            presence.set(activity);
        }
    }
}

/// Un'operazione di sincronizzazione come `Task` che riporta solo l'esito.
trait SyncTask {
    fn boxed_task(self) -> Task<Message>;
}

impl<F> SyncTask for F
where
    F: std::future::Future<Output = Result<(), AppError>> + Send + 'static,
{
    fn boxed_task(self) -> Task<Message> {
        Task::perform(self, Message::SocialSynced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_asks_only_for_what_it_needs() {
        let mut form = SignInForm { identifier: "psy".into(), password: "x".into(), ..SignInForm::default() };
        assert!(form.can_submit());
        form.mode = AuthMode::SignUp;
        form.email = "psy@example.com".into();
        assert!(!form.can_submit(), "la password di una registrazione vuole 8 caratteri");
        form.password = "abcdefgh".into();
        assert!(form.can_submit());
        form.busy = true;
        assert!(!form.can_submit());
    }

    fn invite(code: &str, from: &str) -> JamInvite {
        JamInvite { code: code.into(), from_handle: from.into(), from_name: from.to_uppercase() }
    }

    #[test]
    fn friends_are_chosen_and_unchosen_for_a_jam() {
        let mut app = App::new().0;
        app.toggle_invitee("luca".into());
        app.toggle_invitee("sara".into());
        app.toggle_invitee("luca".into());
        assert_eq!(app.social.invitees, ["sara"]);
        for i in 0..20 {
            app.toggle_invitee(format!("u{i}"));
        }
        assert_eq!(app.social.invitees.len(), crate::jam::MAX_MEMBERS, "non più dei posti della stanza");
    }

    #[test]
    fn each_jam_invite_is_announced_once() {
        let mut app = App::new().0;
        let _ = app.jam_invites_loaded(Ok(vec![invite("K7QXM3", "luca")]));
        assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("LUCA")));
        assert_eq!(app.social.jam_invites.len(), 1);

        app.notice = None;
        let _ = app.jam_invites_loaded(Ok(vec![invite("K7QXM3", "luca")]));
        assert!(app.notice.is_none(), "lo stesso invito non si annuncia di nuovo");

        let _ = app.jam_invites_loaded(Ok(Vec::new()));
        assert!(app.social.jam_invites.is_empty());
        assert!(app.social.announced_invites.is_empty(), "i codici chiusi si dimenticano");
    }

    #[test]
    fn a_failed_refresh_keeps_the_invites_and_the_friends_already_shown() {
        let mut app = App::new().0;
        let _ = app.jam_invites_loaded(Ok(vec![invite("K7QXM3", "luca")]));
        let _ = app.jam_invites_loaded(Err(AppError::Network));
        assert_eq!(app.social.jam_invites.len(), 1);

        app.social.friends = Load::Ready(Friends::default());
        let _ = app.friends_loaded(Err(AppError::Network));
        assert!(matches!(app.social.friends, Load::Ready(_)));
    }

    #[test]
    fn a_playlist_fingerprint_changes_with_its_tracks_and_name() {
        assert_ne!(fingerprint("Notte", &[]), fingerprint("Giorno", &[]));
        assert_eq!(fingerprint("Notte", &[]), fingerprint("Notte", &[]));
    }
}
