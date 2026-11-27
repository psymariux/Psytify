//! Account, profilo, amici, presenza e playlist pubbliche (passo 4.7).
//!
//! **Accesso.** Password, codice via email e GitHub, Google o Discord con ritorno sul loopback sono
//! una macchina a stati del riduttore: ogni passo è un lavoro (`Job`) che parla con Clerk e il suo
//! esito (`Done::Stepped`) porta al passo dopo (un codice da scrivere, oppure `Job::Enter`: l'utente
//! sul server e, la prima volta, un profilo proposto da Clerk). Il browser si apre con un effetto
//! (`Effect::OpenUrl`, che l'host esegue con `platform::open_url`), non dentro un lavoro, così i test
//! lo vedono senza aprirlo. Ogni tentativo ha un numero: se l'utente rinuncia, o il browser non si
//! apre, l'esito che arriva dopo si butta. I campi di testo restano al frontend, che manda i valori
//! nel comando; la password e il codice sono `Secret`, che non si stampa.
//!
//! **Dove vive il token.** La sessione (`auth::Session`: token del client e id di sessione, più il
//! token di un minuto che manda il backend) sta solo qui dentro, mai nella vista, negli eventi o nei
//! log, e i lavori ne portano una copia. Il token di un minuto lo rinnova il lavoro che ne ha bisogno
//! (`Session::bearer`, una sola serratura): nessun timer lo rinnova, perché un'app ferma non deve
//! telefonare a Clerk. `session.json` sta nella cartella dei dati del core (`ClerkApp::session_file`).
//!
//! **Cosa va al server da solo.** Dopo ogni ingresso il passaggio fra i domini chiama
//! [`Social::sync`] e [`Social::listen`]: il brano in ascolto (quando cambia brano, va in pausa,
//! riprende o si salta lontano; mai mentre il brano si carica, così parte una volta sola per ogni
//! brano), gli ascolti (un brano sentito per 30 secondi, solo con un accesso fatto: uno sentito prima
//! non parte dopo) e le playlist rese pubbliche (quando cambiano nome o brani; o tornano private, o
//! spariscono). Il confronto costa qualche campo; la rete parte solo se qualcosa è cambiato.
//!
//! **Quando si sveglia.** Un core senza accesso non arma nessun timer. Con un accesso e un profilo
//! ne arma uno da 15 secondi (amici, richieste e inviti alle jam), che si ferma uscendo; il rinvio di
//! un salto di posizione troppo ravvicinato è un timer a colpo solo.
//!
//! Cambiare vista solo se qualcosa cambia davvero: un sondaggio che riporta gli stessi amici, un
//! comando ripetuto o un esito superato non pubblicano niente.

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

/// Ogni quanto, con un accesso e un profilo, si chiedono richieste d'amicizia, amici e inviti.
pub const POLL: Duration = Duration::from_secs(15);
/// Un salto oltre questo scarto rispetto alla posizione attesa aggiorna il brano in ascolto.
pub const PRESENCE_DRIFT: Duration = Duration::from_secs(5);
/// Fra due aggiornamenti del brano in ascolto, almeno questo tempo (un salto dopo l'altro).
pub const PRESENCE_GAP: Duration = Duration::from_secs(2);
/// Utenti cercati mostrati.
pub const MAX_FOUND: usize = 20;
/// Brani di una playlist pubblica mandati al server, al massimo.
const PUBLIC_TRACKS: usize = 500;
/// Eventi tenuti nella vista: un frontend che legge a ogni pubblicazione non ne perde.
const EVENTS: usize = 32;

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum SocialCmd {
    /// Accesso con email o nome utente e password.
    SignIn { identifier: String, password: Secret },
    /// Registrazione: l'email si conferma con un codice.
    SignUp { email: String, username: String, password: Secret },
    /// Il codice che Clerk ha mandato via email, per il passo in corso (`AuthView::step`).
    SubmitCode(Secret),
    /// Un altro codice per lo stesso passo.
    ResendCode,
    /// Accesso con il browser (GitHub, Google o Discord): il core apre il provider e aspetta il ritorno.
    SignInWith(Provider),
    /// Torna dal codice ai campi, o rinuncia ad aspettare il browser (libera la porta di loopback).
    /// Un invio già partito non si ferma: finisce da solo.
    CancelSignIn,
    /// Toglie l'errore dell'accesso (il frontend lo manda quando l'utente riprende a scrivere).
    DismissError,
    /// Esce: chiude la sessione su Clerk, toglie il presenza dal server e il file dal disco.
    SignOut,
    /// Salva nome utente, nome e, con `Some`, la condivisione dell'attività.
    SaveProfile { handle: String, display_name: String, share_activity: Option<bool> },
    /// Rilegge gli amici e le richieste (la pagina Amici è aperta, o «Riprova»).
    RefreshFriends,
    /// Chiede o accetta l'amicizia (`add`), oppure la toglie, la ritira o la rifiuta.
    FriendAction { handle: String, add: bool },
    /// Cerca persone per nome utente; un testo vuoto azzera i risultati.
    SearchUsers(String),
    /// Apre il profilo di `handle` (il suo caricamento è nella vista); se è già aperto e pronto non
    /// fa niente.
    OpenProfile(String),
    /// «Riprova» sul profilo aperto: lo rilegge.
    ReloadProfile,
    /// Apre una playlist pubblica di un amico.
    OpenFriendPlaylist { handle: String, id: Uuid },
    /// Invita degli amici nella jam in cui si è.
    InviteToJam(Vec<String>),
    /// Toglie un invito alla jam, subito dalla vista e dal server.
    DismissInvite(String),
    /// Solo per i test: l'accesso è già fatto, come se l'ultimo passo fosse appena finito.
    #[cfg(any(test, feature = "test-support"))]
    Adopt { session: Session, me: Me },
}

/// Lo stato del login con Clerk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Account {
    /// Clerk non è configurato: niente pulsante di accesso.
    Unavailable,
    #[default]
    SignedOut,
    /// Un accesso in corso (anche il ripristino della sessione all'avvio).
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

    /// Il proprio profilo: manca finché l'utente non sceglie un nome utente.
    pub fn profile(&self) -> Option<&MyProfile> {
        self.me()?.profile.as_ref()
    }
}

/// Dov'è l'accesso, per il modulo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthView {
    /// Un invio è partito e non è ancora tornato: il modulo non ne accetta un altro.
    pub busy: bool,
    /// Il provider aperto nel browser, mentre si aspetta il ritorno.
    pub waiting: Option<Provider>,
    /// Clerk ha mandato un codice: si chiede quello.
    pub step: Option<CodeHint>,
    /// L'ultimo tentativo non è riuscito. Resta finché non se ne fa un altro o il frontend lo toglie.
    pub error: Option<AppError>,
}

/// Dove è arrivato il codice (l'indirizzo, in parte nascosto da Clerk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeHint {
    pub sent_to: String,
}

/// Un profilo aperto, con gli ascolti recenti già pronti per la coda.
#[derive(Debug, Clone)]
pub struct OpenProfile {
    pub page: UserPage,
    pub recent: Arc<[Track]>,
}

/// Qualcosa che il frontend deve far sapere.
#[derive(Debug, Clone, PartialEq)]
pub enum SocialEvent {
    /// L'accesso è riuscito (non il ripristino all'avvio): il nome del profilo, o «di nuovo».
    Welcomed(String),
    /// Un altro codice è partito, per questo indirizzo.
    CodeResent(String),
    ProfileSaved,
    /// Il nome utente scelto è già di qualcun altro.
    HandleTaken,
    /// Il nome utente è troppo corto, o il nome visibile è vuoto.
    HandleTooShort,
    /// Un'azione d'amicizia è riuscita: la relazione che ne risulta.
    FriendChanged(Relation),
    /// Un invito alla jam è nuovo: chi l'ha mandato, e quanti altri ne sono arrivati insieme.
    JamInvite {
        from: String,
        more: usize,
    },
    /// Gli inviti alla jam sono partiti.
    InvitesSent(usize),
    /// La jam non ha ancora un codice da dare agli amici.
    JamNotReady,
    /// Un'operazione non è riuscita (la sessione scaduta compresa: il core esce da solo).
    Failed(AppError),
}

impl SocialEvent {
    /// L'avviso da mostrare.
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

    /// L'avviso è un errore, non un'informazione.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::HandleTaken | Self::HandleTooShort | Self::Failed(_))
    }
}

/// Un [`SocialEvent`] con il suo numero d'ordine, che cresce a ogni evento.
#[derive(Debug, Clone, PartialEq)]
pub struct SocialRaised {
    pub seq: u64,
    pub event: SocialEvent,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct SocialView {
    pub account: Account,
    pub auth: AuthView,
    /// Il profilo si sta salvando.
    pub profile_saving: bool,
    pub friends: Load<Arc<Friends>>,
    /// Il risultato della ricerca di persone (al più [`MAX_FOUND`]).
    pub found: Load<Arc<[Profile]>>,
    /// Il profilo aperto: il nome utente e la pagina.
    pub open: Option<(String, Load<Arc<OpenProfile>>)>,
    /// La playlist pubblica aperta.
    pub playlist: Option<Load<Arc<PublicPlaylist>>>,
    /// Nomi utente con un'azione d'amicizia in corso.
    pub pending: Vec<String>,
    /// Inviti alle jam ricevuti dagli amici, dal più recente.
    pub jam_invites: Arc<[JamInvite]>,
    /// Gli inviti agli amici sono partiti e non hanno ancora risposto.
    pub inviting: bool,
    /// Gli ultimi eventi, dal più vecchio: un frontend mostra quelli con `seq` nuovo, in ordine.
    pub events: Arc<VecDeque<SocialRaised>>,
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Il sondaggio da [`POLL`]: armato solo con un accesso e un profilo.
    Poll,
    /// Un salto di posizione arrivato troppo presto dopo l'ultimo invio: si rimanda a [`PRESENCE_GAP`].
    Presence,
}

/// Il brano in ascolto da dire al server.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Presence {
    Put { track: Uuid, position_ms: u64, paused: bool },
    Delete,
}

/// I lavori asincroni del dominio (rete, file), che l'host esegue sul runtime.
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
    /// Mette in ascolto il loopback e chiede a Clerk il provider.
    OauthBegin {
        app: ClerkApp,
        provider: Provider,
        attempt: u64,
    },
    /// Aspetta il ritorno del browser, finché `cancel` non scatta.
    OauthWait {
        app: ClerkApp,
        oauth: Box<Oauth>,
        cancel: Arc<Notify>,
        attempt: u64,
    },
    /// All'avvio: la sessione salvata, se Clerk la considera ancora attiva.
    Restore {
        app: ClerkApp,
        api: Result<ApiClient, AppError>,
    },
    /// Dopo l'accesso: l'utente sul server e, la prima volta, il profilo proposto da Clerk.
    Enter {
        api: Result<ApiClient, AppError>,
        session: Session,
        user: ClerkUser,
    },
    SignOut {
        api: Result<ApiClient, AppError>,
        session: Session,
    },
    /// Toglie `session.json`: Clerk non riconosce più la sessione.
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

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    /// Un passo dell'accesso: fatto, oppure serve un codice. `attempt` dice a quale tentativo risponde.
    Stepped {
        attempt: u64,
        result: Result<Box<Step>, AppError>,
    },
    /// Il loopback è in ascolto e Clerk ha dato l'indirizzo del provider.
    OauthReady {
        attempt: u64,
        result: Result<Box<Oauth>, AppError>,
    },
    /// L'host non è riuscito ad aprire il browser.
    BrowserFailed,
    CodeResent(Result<CodeStep, AppError>),
    /// L'accesso è completo (o il ripristino è finito): sessione, utente di Clerk e utente del server.
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
    /// Un invio al server che non mostra nulla (brano in ascolto, ascolti, playlist, invito tolto):
    /// solo la sessione scaduta conta.
    Sync(Result<(), AppError>),
}

/// L'ultimo brano in ascolto mandato al server.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SentPresence {
    track: Uuid,
    paused: bool,
    position: Duration,
    at: Instant,
}

/// Che cosa il profilo vuole far vedere agli amici adesso.
enum Wanted {
    /// Il brano si sta caricando: si aspetta che suoni (o fallisca), senza dire niente nel frattempo.
    Hold,
    Nothing,
    Track {
        track: Uuid,
        paused: bool,
        position: Duration,
    },
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

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Social {
    pub(crate) view: Arc<SocialView>,
    /// L'istanza Clerk, se il login è configurato.
    clerk: Option<ClerkApp>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    /// C'è una sessione salvata da riprendere: la riprende `start`.
    restoring: bool,
    /// La sessione di Clerk: mai nella vista.
    session: Option<Session>,
    /// L'utente di Clerk dell'ultimo accesso: da qui si propone il profilo.
    clerk_user: Option<ClerkUser>,
    /// Il passo con il codice da scrivere.
    step: Option<CodeStep>,
    /// Il numero dell'ultimo tentativo di accesso: l'esito di uno superato si butta.
    attempt: u64,
    /// Ferma l'attesa del browser.
    wait: Option<Arc<Notify>>,
    /// Il numero dell'ultima lettura degli amici, della ricerca e della playlist aperta.
    friends_ticket: u64,
    found_ticket: u64,
    playlist_ticket: u64,
    /// Codici degli inviti già annunciati: ognuno si annuncia una volta sola.
    announced: Vec<String>,
    /// La jam in cui si è, se si è in una e la stanza ha detto il suo codice (la dà la jam).
    jam_code: Option<String>,
    sent: Option<SentPresence>,
    /// Impronta delle playlist pubbliche già mandate.
    published: HashMap<Uuid, u64>,
    /// L'elenco delle playlist visto per ultimo: finché è lo stesso non si ricalcola niente.
    seen_playlists: Option<crate::shared::Shared<config::Playlist>>,
    /// I timer armati.
    polling: bool,
    presence_timer: bool,
    /// Il numero dell'ultimo evento.
    events: u64,
}

impl Default for Social {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl Social {
    /// Il dominio con l'istanza Clerk (se c'è) e il backend scelto nelle impostazioni.
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

    /// Dove sta `session.json` e se c'è una sessione da riprendere (lo sa l'host, che ha guardato il
    /// disco prima della prima istantanea: il riduttore non tocca i file). Con una sessione salvata
    /// la vista parte già su «accesso in corso», e `start` la riprende.
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

    /// L'istanza Clerk, per i test che aprono un accesso già fatto.
    #[cfg(test)]
    pub(crate) fn clerk_app(&self) -> Option<ClerkApp> {
        self.clerk.clone()
    }

    /// L'indirizzo del backend in uso, per i test.
    #[cfg(test)]
    pub(crate) fn api_base(&self) -> Option<String> {
        self.api.as_ref().ok().map(|api| api.base_url().to_owned())
    }

    /// I numeri delle ultime richieste, per i test che consegnano un esito a mano.
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

    /// Il core è appena partito: se c'è una sessione salvata, la si riprende.
    pub(crate) fn start(&mut self, effects: &mut Vec<Effect>) {
        if !std::mem::take(&mut self.restoring) {
            return;
        }
        if let Some(app) = self.clerk.clone() {
            Self::run(effects, Job::Restore { app, api: self.api() });
        }
    }

    /// La stanza della jam in cui si è, se la stanza ha detto il suo codice: lo legge il passaggio fra
    /// i domini dalla vista della jam (passo 4.8). Serve a non annunciare l'invito alla propria
    /// stanza e a mandare gli inviti con il codice giusto. Si copia solo quando cambia.
    pub(crate) fn set_jam(&mut self, code: Option<&str>) {
        if self.jam_code.as_deref() != code {
            self.jam_code = code.map(str::to_owned);
        }
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client HTTP si rifà.
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

    /// Un'operazione non è riuscita: se la sessione è scaduta si esce, altrimenti si dice solo l'errore.
    fn fail(&mut self, err: AppError, effects: &mut Vec<Effect>) {
        let expired = err == AppError::AuthExpired;
        self.raise(SocialEvent::Failed(err));
        if expired {
            self.sign_out(effects);
        }
    }

    /// La sessione e il profilo, se ci sono entrambi: le funzioni social vogliono un nome utente.
    fn session_with_profile(&self) -> Option<Session> {
        self.session.clone().filter(|_| self.view.account.profile().is_some())
    }

    // ---------- Comandi ----------

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

    /// Un nuovo tentativo di accesso: serve il login configurato, nessun accesso in corso e nessuno
    /// fatto. Restituisce l'istanza e il numero del tentativo.
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

    /// Chiude il tentativo in corso senza esito: né un errore né un accesso.
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
        // Un invio a Clerk già partito (password, codice) finisce da solo: fermarlo lascerebbe una
        // sessione creata sul server e dimenticata qui. Si rinuncia solo ad aspettare il browser.
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

    /// L'esito di un passo dell'accesso, se è del tentativo in corso.
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
                // Il tentativo è finito, ma l'accesso no: l'utente sul server si legge ancora.
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

    /// Clerk ha dato l'indirizzo del provider: il browser lo apre l'host, e intanto si aspetta il ritorno.
    fn oauth_ready(&mut self, attempt: u64, result: Result<Box<Oauth>, AppError>, effects: &mut Vec<Effect>) {
        if attempt != self.attempt {
            // L'utente ha rinunciato: l'accesso a metà si chiude con il suo `Oauth` (e la porta).
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

    /// L'accesso è completo, o il ripristino è finito.
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
                    // Sul modulo, dove l'utente sta guardando.
                    self.view_mut().auth.error = Some(err.clone());
                } else if err != AppError::AuthExpired {
                    self.raise(SocialEvent::Failed(err.clone()));
                }
                // Una sessione che Clerk non riconosce più non serve: la si toglie dal disco.
                if err == AppError::AuthExpired
                    && let Some(app) = self.clerk.clone()
                {
                    Self::run(effects, Job::Forget { app });
                }
            }
        }
    }

    /// Esce: la sessione si chiude su Clerk, il brano in ascolto sparisce dal server e `session.json`
    /// dal disco. Tutto ciò che era dell'utente (amici, profili aperti, ricerche) si butta.
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
        // Le risposte in volo sono di chi è uscito.
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
        // L'immagine resta quella del profilo; la prima volta, quella di Clerk (solo dal suo dominio).
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
                    // Il server toglie da sé il brano in ascolto di chi smette di condividere.
                    self.sent = None;
                }
                let first = self.view.account.profile().is_none();
                if let Account::SignedIn(me) = &mut self.view_mut().account {
                    me.profile = Some(profile);
                }
                self.raise(SocialEvent::ProfileSaved);
                // Il primo profilo apre le funzioni sociali: amici e richieste si leggono subito.
                if first {
                    self.load_friends(effects);
                }
            }
            Err(AppError::Api { status: 409, .. }) => self.raise(SocialEvent::HandleTaken),
            Err(err) => self.fail(err, effects),
        }
    }

    // ---------- Amici e profili ----------

    /// Legge gli amici e le richieste. Vale solo la risposta all'ultima lettura.
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
            // Un aggiornamento periodico che fallisce non cancella la lista già mostrata.
            (Err(_), Load::Ready(_)) if !expired => {}
            // Gli stessi amici di prima: niente da pubblicare.
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
            // Un aggiornamento mancato si riprova al prossimo giro.
            Err(_) => return,
        };
        // Chi è già in una jam non ha bisogno dell'invito a una stanza in cui si trova.
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
        // Si ricordano solo i codici ancora in giro: la lista non cresce senza fondo.
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
                // La pagina aperta di questa persona ha la relazione di prima: si rilegge.
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

    // ---------- Sincronizzazione ----------

    /// Un brano contato come ascoltato: se si è collegati con un profilo va anche al profilo. Senza,
    /// si butta: non resta in sospeso per partire a un accesso futuro.
    pub(crate) fn listen(&mut self, track: Uuid, effects: &mut Vec<Effect>) {
        if let Some(session) = self.session_with_profile() {
            Self::run(effects, Job::Listen { api: self.api(), session, track });
        }
    }

    /// Dopo ogni ingresso: manda al server ciò che è cambiato nel brano in ascolto e nelle playlist
    /// pubbliche. Quasi sempre non c'è niente da mandare.
    pub(crate) fn sync(
        &mut self,
        playback: &PlaybackView,
        library: &LibraryView,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        // La sessione si presta ai due passaggi senza clonarla: si copia solo quando parte un invio
        // (questo giro avviene a ogni battito di una riproduzione, e quasi sempre non manda niente).
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
                    // Un salto subito dopo un invio aspetta: fra due invii passano almeno due secondi.
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

    /// Le playlist pubbliche cambiate dall'ultimo invio, e quelle che non lo sono più. L'impronta si
    /// calcola solo sulle pubbliche (poche) e solo quando l'elenco delle playlist è un altro.
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
        // Tornata privata, o tolta: sparisce anche dal profilo.
        let gone: Vec<Uuid> = self.published.keys().filter(|id| !public.contains(id)).copied().collect();
        for id in gone {
            self.published.remove(&id);
            Self::run(effects, Job::DeletePlaylist { api: self.api(), session: session.clone(), id });
        }
    }

    // ---------- Timer ----------

    /// Il sondaggio gira solo con un accesso e un profilo: lo arma o lo ferma di conseguenza.
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
            // Il passaggio che segue ogni ingresso rilegge il brano in ascolto: basta svegliarlo.
            Timer::Presence => self.presence_timer = false,
        }
        self.keep_polling(effects);
    }

    // ---------- Esiti ----------

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
                // Un errore qui non cambia ciò che si vede. Solo una sessione scaduta fa uscire.
                match result {
                    Err(AppError::AuthExpired) => self.fail(AppError::AuthExpired, effects),
                    Err(err) if !matches!(err, AppError::Network | AppError::Timeout) => {
                        // Il file della sessione che non si toglie, per esempio: va detto.
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

/// Sessione, utente di Clerk e utente del server, per l'esito di un accesso.
async fn enter_with(
    api: Result<ApiClient, AppError>,
    session: Session,
    user: ClerkUser,
) -> Result<Box<(Session, ClerkUser, Me)>, AppError> {
    let me = enter(api, session.clone(), user.clone()).await?;
    Ok(Box::new((session, user, me)))
}

/// L'esito `Err` di un lavoro andato perso, per un lavoro che parla col server.
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
                // Chi esce non resta «in ascolto» per gli amici.
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

    /// Se il lavoro va perso, il dominio non resta ad aspettare: un accesso torna al modulo con un
    /// errore, una lettura fallisce come senza rete.
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

    /// Il dominio da solo, con un orologio finto e una riproduzione e una libreria costruite a mano:
    /// il passaggio fra i domini (`CoreState::handoff`) lo fa il test, chiamando `sync` e `listen`.
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

        /// Un accesso già fatto, come dopo l'ultimo passo: con il profilo, e condividendo l'attività
        /// se `share`. Restituisce anche gli effetti dell'ingresso.
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

        /// Il passaggio che segue ogni ingresso, per il brano in ascolto e le playlist.
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

        /// Il brano `n` in questo stato, con l'orologio a `position` (che scorre se suona).
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

        /// Le playlist della libreria, nuove (un'altra allocazione) come dopo un cambio vero.
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

    // ---------- Accesso ----------

    /// Un core fermo e senza accesso non arma nessun timer e non chiede nessun lavoro, qualunque
    /// cosa succeda: il sondaggio nasce solo con un accesso.
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

        // Un secondo invio mentre il primo è in volo non parte.
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

        // Un codice sbagliato lascia il passo dov'è e dice l'errore: si riprova.
        rig.done(Done::Stepped { attempt: 2, result: Err(AppError::AuthFailed) });
        let view = rig.view();
        assert_eq!(view.auth.error, Some(AppError::AuthFailed));
        assert!(view.auth.step.is_some(), "il passo del codice resta");
        let effects = rig.cmd(SocialCmd::SubmitCode(Secret::new("424242")));
        assert_eq!(works(&effects), [SocialWork::SubmitCode]);
        assert!(rig.view().auth.error.is_none(), "un nuovo tentativo toglie l'errore");

        // Dal codice si torna ai campi.
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

        // Il ritorno del browser è un passo come gli altri.
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
        // Chi aspetta il ritorno viene svegliato: la porta di loopback si libera.
        tokio::time::timeout(secs(1), wait.notified()).await.expect("l'attesa del browser è stata fermata");

        // L'esito di quel tentativo, se arriva lo stesso, si butta: non si entra da una porta chiusa.
        let late = rig.done(Done::Stepped {
            attempt: 1,
            result: Ok(Box::new(Step::Done(Session::for_tests(clerk()), ClerkUser::default()))),
        });
        assert!(late.is_empty());
        assert_eq!(rig.view().account, Account::SignedOut);

        // Un nuovo tentativo ha il suo numero.
        let effects = rig.cmd(SocialCmd::SignInWith(Provider::Discord));
        assert_eq!(works(&effects), [SocialWork::OauthBegin(Provider::Discord)]);
        // E l'indirizzo di quello vecchio, arrivato tardi, non apre il browser.
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
        // Una seconda notizia dello stesso guasto non cambia niente.
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

    // ---------- Sessione salvata ----------

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
        // Una volta sola.
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

    // ---------- Uscita e sessione scaduta ----------

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

        // Ciò che era in volo per l'utente di prima si butta.
        rig.done(Done::Friends { ticket, result: Ok(Friends::default()) });
        assert!(matches!(rig.view().friends, Load::Idle));
        // E uscire due volte è come una.
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

    // ---------- Sondaggio ----------

    #[test]
    fn polling_runs_only_while_signed_in_with_a_profile() {
        let (mut rig, effects) = Rig::signed_in(true);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))]);
        assert_eq!(works(&effects), [SocialWork::Friends]);

        rig.elapse(POLL);
        let effects = rig.fire(Timer::Poll);
        assert_eq!(works(&effects), [SocialWork::Friends, SocialWork::Invites]);
        assert_eq!(timers(&effects), [(Timer::Poll, Some(POLL))], "si riarma da sé finché c'è l'accesso");

        // Un accesso senza profilo non sonda: le funzioni sociali vogliono un nome utente.
        let mut rig = Rig::new();
        let effects = rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(false, true) });
        assert!(works(&effects).is_empty() && timers(&effects).is_empty());
        // Il primo profilo apre le funzioni sociali: amici subito, e poi il sondaggio.
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

    // ---------- Inviti alle jam ----------

    #[test]
    fn each_jam_invite_is_announced_once() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert_eq!(rig.events(), [SocialEvent::JamInvite { from: "LUCA".into(), more: 0 }]);
        // Lo stesso invito riletto non si annuncia di nuovo.
        rig.done(Done::Invites(Ok(vec![invite("AAA111", "luca")])));
        assert_eq!(rig.events().len(), 1);
        // Due nuovi insieme: un solo avviso, con «e altri».
        rig.done(Done::Invites(Ok(vec![
            invite("AAA111", "luca"),
            invite("BBB222", "sara"),
            invite("CCC333", "anna"),
        ])));
        assert_eq!(rig.events().last(), Some(&SocialEvent::JamInvite { from: "SARA".into(), more: 1 }));
        // Un invito sparito e tornato con lo stesso codice si annuncia di nuovo.
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
        // Finché non risponde, un secondo invio non parte.
        assert!(works(&rig.cmd(SocialCmd::InviteToJam(vec!["anna".into()]))).is_empty());
        rig.done(Done::Invited(Ok(2)));
        assert!(!rig.view().inviting);
        assert_eq!(rig.events().last(), Some(&SocialEvent::InvitesSent(2)));
    }

    // ---------- Profilo ----------

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

    /// La bozza del profilo che il lavoro `PutProfile` porta al server.
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

    // ---------- Persone e profili ----------

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
        // Il testo vuoto azzera, e una risposta in volo non lo rimette.
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
        // Aperto e pronto: riaprirlo non rifà la richiesta.
        assert!(works(&rig.cmd(SocialCmd::OpenProfile("luca".into()))).is_empty());
        // Una risposta per un altro profilo non tocca quello aperto.
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

        // «Riprova» sul profilo aperto lo rilegge.
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

    // ---------- Presenza (BACKLOG 9) ----------

    /// BACKLOG 9: un brano che parte manda la presenza una volta sola, quando suona davvero; mentre
    /// si carica non manda niente («in pausa» e poi «in riproduzione» erano due invii per brano).
    #[test]
    fn presence_goes_once_per_real_transition() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Loading { track_id: track_id(1) }, Duration::ZERO);
        assert!(works(&rig.sync()).is_empty(), "mentre si carica non si dice niente");

        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()), [presence(1, 0, false)]);

        // Il battito (250 ms) e gli altri messaggi non rimandano niente.
        for _ in 0..8 {
            rig.elapse(Duration::from_millis(250));
            assert!(works(&rig.sync()).is_empty());
        }

        // La pausa parte subito, e la ripresa pure.
        rig.play(1, PlaybackState::Paused, secs(2));
        assert_eq!(works(&rig.sync()), [presence(1, 2_000, true)]);
        assert!(works(&rig.sync()).is_empty());
        rig.elapse(secs(30));
        assert!(works(&rig.sync()).is_empty(), "in pausa la posizione non scorre: niente scarto");
        rig.play(1, PlaybackState::Playing, secs(2));
        assert_eq!(works(&rig.sync()), [presence(1, 2_000, false)]);

        // Il brano dopo: caricamento muto, poi un invio solo.
        rig.elapse(secs(1));
        rig.play(2, PlaybackState::Loading { track_id: track_id(2) }, Duration::ZERO);
        assert!(works(&rig.sync()).is_empty());
        rig.play(2, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()), [presence(2, 0, false)]);

        // Fine della coda: il brano in ascolto sparisce dal server, una volta.
        rig.stop();
        assert_eq!(works(&rig.sync()), [SocialWork::Presence { track: None, position_ms: 0, paused: false }]);
        assert!(works(&rig.sync()).is_empty());
    }

    #[test]
    fn a_small_drift_sends_nothing_and_a_far_jump_waits_for_the_gap() {
        let (mut rig, _) = Rig::signed_in(true);
        rig.play(1, PlaybackState::Playing, Duration::ZERO);
        assert_eq!(works(&rig.sync()).len(), 1);

        // Fra cinque secondi di scarto e meno: niente.
        rig.elapse(secs(1));
        rig.playback.clock = Clock::running(secs(1) + Duration::from_secs(4), rig.now);
        assert!(works(&rig.sync()).is_empty());

        // Un salto lontano subito dopo un invio aspetta: un timer, non un invio.
        rig.playback.clock = Clock::running(secs(11), rig.now);
        let effects = rig.sync();
        assert!(works(&effects).is_empty());
        let waits = timers(&effects);
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].0, Timer::Presence);
        let after = waits[0].1.expect("armato");
        assert!(after > Duration::ZERO && after <= PRESENCE_GAP, "{after:?}");
        // Il battito che continua non riarma il timer: l'orologio di riproduzione scorre da sé.
        rig.elapse(Duration::from_millis(250));
        assert!(rig.sync().is_empty());

        // Scaduta l'attesa, lo stesso passaggio manda la posizione nuova.
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
        // Riaccendendo la condivisione, il brano che suona torna visibile.
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

    // ---------- Ascolti (BACKLOG 5) ----------

    /// BACKLOG 5: un brano sentito senza accesso non resta in sospeso per partire a un accesso
    /// futuro: l'ascolto si butta, e dopo l'accesso non parte niente.
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
        // Un accesso senza profilo non manda ascolti.
        let mut rig = Rig::new();
        rig.cmd(SocialCmd::Adopt { session: Session::for_tests(clerk()), me: me(false, true) });
        assert!(rig.listen(1).is_empty());
    }

    // ---------- Playlist pubbliche ----------

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
        // Frame, battiti e messaggi vari: niente.
        assert!(works(&rig.sync()).is_empty());
        // Un'altra allocazione con lo stesso contenuto (è cambiata un'altra playlist, o altro): niente.
        rig.playlists(vec![sera.clone(), playlist("Privata", &[3], false)]);
        assert!(works(&rig.sync()).is_empty());
        // Un brano in più: un invio.
        sera.tracks.push(track(2));
        rig.playlists(vec![sera.clone()]);
        assert_eq!(
            works(&rig.sync()),
            [SocialWork::PublishPlaylist { id, name: "Sera".into(), tracks: vec![track_id(1), track_id(2)] }]
        );
        // Un altro nome: un invio.
        sera.name = "Notte".into();
        rig.playlists(vec![sera.clone()]);
        assert_eq!(works(&rig.sync()).len(), 1);
        // Tornata privata: sparisce dal profilo.
        sera.public = false;
        rig.playlists(vec![sera.clone()]);
        assert_eq!(works(&rig.sync()), [SocialWork::UnpublishPlaylist(id)]);
        assert!(works(&rig.sync()).is_empty());
        // Ripubblicata: di nuovo un invio. Poi tolta del tutto: sparisce.
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

    // ---------- Segreti, lavori persi, eventi ----------

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

    /// Un lavoro che va in panic non lascia il dominio ad aspettare: ogni lavoro ha il suo esito.
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

        // Anche uno di lettura perso fallisce come senza rete, senza cancellare ciò che c'è.
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
