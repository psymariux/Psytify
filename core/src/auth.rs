//! Optional login with Clerk, inside the app and all in Rust: the «Accedi a Psytify» screen
//! talks to Clerk's Frontend API in native mode (`_is_native=1`), like the mobile apps.
//!
//! - Email or username and password; if Clerk asks for a confirmation (new device, second
//!   factor) it sends a code by email that is typed into the app.
//! - Sign-up with email, username and password, confirmed by the code sent by email.
//! - GitHub, Google and Discord go through the browser and return to the app on
//!   `http://127.0.0.1:<port>/callback`.
//!
//! Clerk recognises the app by a client token (an `Authorization` header with no scheme), which can
//! change with every reply. The client token and the session id are in `session.json`,
//! readable only by the user, in the core's data folder (`ClerkApp::session_file`: since step
//! 4.7 there is no per-process folder any more); the session token to send to the backend lasts
//! a minute, stays in memory and renews itself (`Session::bearer`). No token ends up in logs or
//! error messages: `Session`, `CodeStep`, `Oauth` and `Secret` do not print them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::error::AppError;

/// Time allowed to the user to complete the login in the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// The browser's callback request is a GET line with a few headers.
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
/// The session token lasts a minute: it is renewed a little before.
const SESSION_TOKEN_REUSE: Duration = Duration::from_secs(45);
const CALLBACK_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Psytify</title>\
<body style=\"background:#000;color:#fff;font-family:sans-serif;padding:48px\">\
<p style=\"color:#c6f432;font-weight:600\">Psytify</p>\
<p>Accesso completato. Puoi tornare all'app.</p></body>";

/// A string that is not printed: the password and the confirmation code travel in the core's commands
/// (which are `Debug`), and the browser address in the effects. `Debug` writes only `[redacted]`.
/// It is not zeroed in memory when dropped: the `zeroize` crate is not in the graph, and the text is already
/// in clear in the interface field that wrote it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The text, for whoever must send it to Clerk or open it in the browser.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// The Clerk instance. Nothing secret: the Frontend API is public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClerkApp {
    /// The instance's Frontend API, for example `https://your-instance.clerk.accounts.dev`.
    pub issuer: String,
    /// Loopback port for the return from the browser (GitHub, Google, Discord).
    pub port: u16,
    /// Where `session.json` sits: whoever starts the core chooses it (`<data_dir>/session.json`). Without it, the
    /// session lives only in memory and at the next start one signs in by hand (the tests).
    pub session_file: Option<PathBuf>,
}

impl ClerkApp {
    fn url(&self, path: &str) -> String {
        format!("{}/v1{path}?_is_native=1", self.issuer)
    }
}

/// The sign-ins that go through the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    Google,
    Discord,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Google, Self::GitHub, Self::Discord];

    fn strategy(self) -> &'static str {
        match self {
            Self::GitHub => "oauth_github",
            Self::Google => "oauth_google",
            Self::Discord => "oauth_discord",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::Google => "Google",
            Self::Discord => "Discord",
        }
    }
}

/// The Clerk user, as much as is needed to propose Psytify's profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClerkUser {
    pub username: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub image_url: Option<String>,
}

/// A Clerk session. Cloning it is cheap; `Debug` does not print the tokens.
#[derive(Clone)]
pub struct Session {
    app: ClerkApp,
    state: Arc<Mutex<SessionState>>,
}

struct SessionState {
    client: String,
    session_id: String,
    token: Option<(String, Instant)>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session([redacted])")
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Session {
    fn new(app: ClerkApp, client: String, session_id: String) -> Self {
        Self { app, state: Arc::new(Mutex::new(SessionState { client, session_id, token: None })) }
    }

    /// A session for the tests, with a token just renewed: `bearer` returns it without
    /// asking Clerk anything.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(app: ClerkApp) -> Self {
        let state = SessionState {
            client: "client-di-prova".into(),
            session_id: "sessione-di-prova".into(),
            token: Some(("token-di-prova".into(), Instant::now())),
        };
        Self { app, state: Arc::new(Mutex::new(state)) }
    }

    /// The token to send to the backend as `Authorization: Bearer`. Renewed when it expires;
    /// `AuthExpired` if Clerk has closed the session.
    pub async fn bearer(&self) -> Result<String, AppError> {
        let mut state = self.state.lock().await;
        if let Some((token, at)) = &state.token
            && at.elapsed() < SESSION_TOKEN_REUSE
        {
            return Ok(token.clone());
        }
        let path = format!("/client/sessions/{}/tokens", state.session_id);
        let mut client = state.client.clone();
        let reply = call(&self.app, reqwest::Method::POST, &path, &mut client, &[]).await;
        if client != state.client {
            state.client = client;
            store(&self.app, &state).await;
        }
        let body = match reply {
            Ok(body) => body,
            Err(Failure::Status(401 | 403 | 404)) => return Err(AppError::AuthExpired),
            Err(failure) => return Err(failure.into_error()),
        };
        let token: TokenBody = serde_json::from_value(body).map_err(|_| AppError::InvalidResponse)?;
        state.token = Some((token.jwt.clone(), Instant::now()));
        Ok(token.jwt)
    }

    /// Closes the session on Clerk and forgets the one on disk. A network error does not prevent
    /// signing out.
    pub async fn sign_out(self) -> Result<(), AppError> {
        {
            let state = self.state.lock().await;
            let path = format!("/client/sessions/{}/remove", state.session_id);
            let mut client = state.client.clone();
            let _ = call(&self.app, reqwest::Method::POST, &path, &mut client, &[]).await;
        }
        forget_session(&self.app).await
    }
}

/// A step of the login: done, or a code that arrived by email is needed.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Done(Session, ClerkUser),
    Code(CodeStep),
}

/// The code to ask the user for: where they received it and which attempt it belongs to.
#[derive(Clone, PartialEq, Eq)]
pub struct CodeStep {
    client: String,
    target: CodeTarget,
    /// The address, partly hidden by Clerk, where the code arrived.
    pub sent_to: String,
}

impl std::fmt::Debug for CodeStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeStep").field("sent_to", &self.sent_to).finish_non_exhaustive()
    }
}

impl CodeStep {
    /// A step with the code for the tests: the address it would have reached.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(sent_to: &str) -> Self {
        Self {
            client: "client-di-prova".into(),
            target: CodeTarget::SignUp("su_prova".into()),
            sent_to: sent_to.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodeTarget {
    SignUp(String),
    FirstFactor { sign_in: String, strategy: String },
    SecondFactor { sign_in: String, strategy: String },
}

// ---------- Clerk's replies ----------

/// Clerk sends `null` where a list is empty (the factors of an already complete sign-in).
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct Envelope<T> {
    response: T,
    #[serde(default)]
    client: Option<ClientBody>,
}

#[derive(Deserialize)]
struct TokenBody {
    jwt: String,
}

#[derive(Deserialize)]
struct ClientBody {
    #[serde(default, deserialize_with = "null_as_empty")]
    sessions: Vec<SessionBody>,
    #[serde(default)]
    last_active_session_id: Option<String>,
}

#[derive(Deserialize)]
struct SessionBody {
    id: String,
    status: String,
    user: Option<UserBody>,
}

#[derive(Deserialize)]
struct UserBody {
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    image_url: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    email_addresses: Vec<EmailBody>,
    primary_email_address_id: Option<String>,
}

#[derive(Deserialize)]
struct EmailBody {
    id: String,
    email_address: String,
}

#[derive(Deserialize)]
struct SignInBody {
    id: String,
    status: String,
    created_session_id: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    supported_first_factors: Vec<FactorBody>,
    #[serde(default, deserialize_with = "null_as_empty")]
    supported_second_factors: Vec<FactorBody>,
    first_factor_verification: Option<VerificationBody>,
}

#[derive(Deserialize)]
struct SignUpBody {
    id: String,
    status: String,
    created_session_id: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    missing_fields: Vec<String>,
    email_address: Option<String>,
}

#[derive(Deserialize)]
struct FactorBody {
    strategy: String,
    safe_identifier: Option<String>,
    email_address_id: Option<String>,
    phone_number_id: Option<String>,
}

#[derive(Deserialize)]
struct VerificationBody {
    status: Option<String>,
    external_verification_redirect_url: Option<String>,
}

#[derive(Deserialize)]
struct ErrorsBody {
    #[serde(default, deserialize_with = "null_as_empty")]
    errors: Vec<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    #[serde(default)]
    long_message: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl From<UserBody> for ClerkUser {
    fn from(user: UserBody) -> Self {
        let name = [user.first_name, user.last_name].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let email = user
            .email_addresses
            .iter()
            .find(|e| Some(&e.id) == user.primary_email_address_id.as_ref())
            .or(user.email_addresses.first())
            .map(|e| e.email_address.clone());
        Self {
            username: user.username.filter(|u| !u.is_empty()),
            name: Some(name).filter(|n| !n.trim().is_empty()),
            email,
            image_url: user.image_url,
        }
    }
}

// ---------- Errors ----------

/// An error from the Frontend API, before it becomes an `AppError` to show.
enum Failure {
    /// Clerk replied with an error: the message is already in Italian.
    Clerk(String),
    Status(u16),
    Other(AppError),
}

impl Failure {
    fn into_error(self) -> AppError {
        match self {
            Self::Clerk(message) => AppError::Auth(message),
            Self::Status(_) => AppError::AuthFailed,
            Self::Other(err) => err,
        }
    }
}

/// The most common Clerk error codes, in Italian; the others with Clerk's text.
pub fn clerk_message(code: &str, fallback: Option<&str>) -> String {
    let known = match code {
        "form_password_incorrect" => "Password errata.",
        "form_identifier_not_found" => "Nessun account con questa email o questo nome utente.",
        "form_identifier_exists" => "Esiste già un account con questa email o questo nome utente.",
        "form_username_invalid_length" => "Il nome utente deve avere da 4 a 64 caratteri.",
        "form_username_invalid_character" => "Il nome utente può avere solo lettere, cifre, - e _.",
        "form_password_length_too_short" => "La password è troppo corta: almeno 8 caratteri.",
        "form_password_pwned" => "Questa password è comparsa in una fuga di dati: scegline un'altra.",
        "form_password_not_strong_enough" => "La password è troppo debole.",
        "form_param_format_invalid" => "Controlla i campi: un valore non è nel formato giusto.",
        "form_code_incorrect" => "Codice errato.",
        "verification_expired" => "Il codice è scaduto: fattene mandare un altro.",
        "verification_failed" => "Troppi tentativi: fatti mandare un nuovo codice.",
        "strategy_for_user_invalid" => {
            "Questo account non ha una password: entra con Google, GitHub o Discord."
        }
        "session_exists" => "Hai già fatto l'accesso.",
        "too_many_requests" => "Troppi tentativi: riprova fra qualche minuto.",
        "external_account_not_found" => "Nessun account collegato: registrati prima.",
        "identifier_already_signed_in" => "Hai già fatto l'accesso.",
        _ => "",
    };
    if known.is_empty() {
        fallback.map_or_else(|| "Accesso non riuscito. Riprova.".into(), str::to_owned)
    } else {
        known.to_owned()
    }
}

// ---------- HTTP ----------

fn http() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("psytify/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AppError::Network)
}

/// A call to the Frontend API. `client` is the client token: it is sent and, if Clerk
/// returns a new one, it is updated.
async fn call(
    app: &ClerkApp,
    method: reqwest::Method,
    path: &str,
    client: &mut String,
    form: &[(&str, &str)],
) -> Result<serde_json::Value, Failure> {
    let mut request = http().map_err(Failure::Other)?.request(method.clone(), app.url(path));
    if !client.is_empty() {
        request = request.header(reqwest::header::AUTHORIZATION, client.as_str());
    }
    if method != reqwest::Method::GET {
        request = request.form(form);
    } else if !form.is_empty() {
        request = request.query(form);
    }
    let response = request.send().await.map_err(|e| Failure::Other(AppError::from_http(&e)))?;
    if let Some(fresh) = response.headers().get(reqwest::header::AUTHORIZATION).and_then(|v| v.to_str().ok())
        && !fresh.is_empty()
    {
        *client = fresh.to_owned();
    }
    let status = response.status().as_u16();
    let body: serde_json::Value =
        response.json().await.map_err(|_| Failure::Other(AppError::InvalidResponse))?;
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let errors: ErrorsBody = serde_json::from_value(body).unwrap_or(ErrorsBody { errors: Vec::new() });
    match errors.errors.first() {
        Some(first) => Err(Failure::Clerk(clerk_message(
            &first.code,
            first.long_message.as_deref().or(first.message.as_deref()),
        ))),
        None => Err(Failure::Status(status)),
    }
}

async fn call_as<T: serde::de::DeserializeOwned>(
    app: &ClerkApp,
    method: reqwest::Method,
    path: &str,
    client: &mut String,
    form: &[(&str, &str)],
) -> Result<Envelope<T>, AppError> {
    let body = call(app, method, path, client, form).await.map_err(Failure::into_error)?;
    serde_json::from_value(body).map_err(|_| AppError::InvalidResponse)
}

/// A new client: the starting point of every sign-in.
async fn new_client(app: &ClerkApp) -> Result<String, AppError> {
    let mut client = String::new();
    call(app, reqwest::Method::POST, "/client", &mut client, &[]).await.map_err(Failure::into_error)?;
    if client.is_empty() { Err(AppError::AuthFailed) } else { Ok(client) }
}

/// The session just created, with the user: it is saved to disk.
async fn finish(app: &ClerkApp, mut client: String, session_id: &str) -> Result<Step, AppError> {
    let envelope: Envelope<ClientBody> =
        call_as(app, reqwest::Method::GET, "/client", &mut client, &[]).await?;
    let user = envelope
        .response
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)
        .and_then(|s| s.user)
        .map(ClerkUser::from)
        .unwrap_or_default();
    let session = Session::new(app.clone(), client, session_id.to_owned());
    store(app, &*session.state.lock().await).await;
    Ok(Step::Done(session, user))
}

/// The factor with a code (first email, then phone) among those Clerk proposes.
fn code_factor(factors: &[FactorBody]) -> Option<&FactorBody> {
    factors
        .iter()
        .find(|f| f.strategy == "email_code")
        .or_else(|| factors.iter().find(|f| f.strategy == "phone_code"))
}

/// Prepares the code for the given factor and returns the step to show.
async fn ask_code(
    app: &ClerkApp,
    mut client: String,
    sign_in: &str,
    factor: &FactorBody,
    second: bool,
) -> Result<Step, AppError> {
    let (path, target) = if second {
        (
            format!("/client/sign_ins/{sign_in}/prepare_second_factor"),
            CodeTarget::SecondFactor { sign_in: sign_in.to_owned(), strategy: factor.strategy.clone() },
        )
    } else {
        (
            format!("/client/sign_ins/{sign_in}/prepare_first_factor"),
            CodeTarget::FirstFactor { sign_in: sign_in.to_owned(), strategy: factor.strategy.clone() },
        )
    };
    let mut form = vec![("strategy", factor.strategy.as_str())];
    if let Some(id) = &factor.email_address_id {
        form.push(("email_address_id", id));
    }
    if let Some(id) = &factor.phone_number_id {
        form.push(("phone_number_id", id));
    }
    call(app, reqwest::Method::POST, &path, &mut client, &form).await.map_err(Failure::into_error)?;
    let sent_to = factor.safe_identifier.clone().unwrap_or_default();
    Ok(Step::Code(CodeStep { client, target, sent_to }))
}

/// Where the state of a sign-in attempt leads.
async fn after_sign_in(app: &ClerkApp, client: String, sign_in: SignInBody) -> Result<Step, AppError> {
    match sign_in.status.as_str() {
        "complete" => {
            let session = sign_in.created_session_id.ok_or(AppError::AuthFailed)?;
            finish(app, client, &session).await
        }
        // New device or second factor: Clerk sends a code.
        "needs_second_factor" | "needs_client_trust" => {
            let factor = code_factor(&sign_in.supported_second_factors).ok_or(AppError::AuthFailed)?;
            ask_code(app, client, &sign_in.id, factor, true).await
        }
        "needs_first_factor" => {
            let factor = code_factor(&sign_in.supported_first_factors).ok_or(AppError::AuthFailed)?;
            ask_code(app, client, &sign_in.id, factor, false).await
        }
        _ => Err(AppError::AuthFailed),
    }
}

async fn after_sign_up(app: &ClerkApp, client: String, sign_up: SignUpBody) -> Result<Step, AppError> {
    match (sign_up.status.as_str(), sign_up.created_session_id) {
        ("complete", Some(session)) => finish(app, client, &session).await,
        _ if !sign_up.missing_fields.is_empty() => Err(AppError::Auth(format!(
            "Per registrarti manca: {}.",
            sign_up.missing_fields.join(", ").replace('_', " ")
        ))),
        _ => {
            let mut client = client;
            let path = format!("/client/sign_ups/{}/prepare_verification", sign_up.id);
            call(app, reqwest::Method::POST, &path, &mut client, &[("strategy", "email_code")])
                .await
                .map_err(Failure::into_error)?;
            Ok(Step::Code(CodeStep {
                client,
                target: CodeTarget::SignUp(sign_up.id),
                sent_to: sign_up.email_address.unwrap_or_default(),
            }))
        }
    }
}

// ---------- Flows ----------

/// Sign-in with email or username and password.
pub async fn sign_in(app: ClerkApp, identifier: String, password: String) -> Result<Step, AppError> {
    let mut client = new_client(&app).await?;
    let form = [("strategy", "password"), ("identifier", identifier.trim()), ("password", password.as_str())];
    let envelope: Envelope<SignInBody> =
        call_as(&app, reqwest::Method::POST, "/client/sign_ins", &mut client, &form).await?;
    after_sign_in(&app, client, envelope.response).await
}

/// Sign-up: the email is confirmed with the code Clerk sends.
pub async fn sign_up(
    app: ClerkApp,
    email: String,
    username: String,
    password: String,
) -> Result<Step, AppError> {
    let mut client = new_client(&app).await?;
    let mut form = vec![("email_address", email.trim()), ("password", password.as_str())];
    if !username.trim().is_empty() {
        form.push(("username", username.trim()));
    }
    let envelope: Envelope<SignUpBody> =
        call_as(&app, reqwest::Method::POST, "/client/sign_ups", &mut client, &form).await?;
    after_sign_up(&app, client, envelope.response).await
}

/// The code typed by the user.
pub async fn submit_code(app: ClerkApp, step: CodeStep, code: String) -> Result<Step, AppError> {
    let mut client = step.client;
    let code = code.trim();
    match step.target {
        CodeTarget::SignUp(id) => {
            let path = format!("/client/sign_ups/{id}/attempt_verification");
            let form = [("strategy", "email_code"), ("code", code)];
            let envelope: Envelope<SignUpBody> =
                call_as(&app, reqwest::Method::POST, &path, &mut client, &form).await?;
            after_sign_up(&app, client, envelope.response).await
        }
        CodeTarget::FirstFactor { sign_in, strategy } => {
            let path = format!("/client/sign_ins/{sign_in}/attempt_first_factor");
            let form = [("strategy", strategy.as_str()), ("code", code)];
            let envelope: Envelope<SignInBody> =
                call_as(&app, reqwest::Method::POST, &path, &mut client, &form).await?;
            after_sign_in(&app, client, envelope.response).await
        }
        CodeTarget::SecondFactor { sign_in, strategy } => {
            let path = format!("/client/sign_ins/{sign_in}/attempt_second_factor");
            let form = [("strategy", strategy.as_str()), ("code", code)];
            let envelope: Envelope<SignInBody> =
                call_as(&app, reqwest::Method::POST, &path, &mut client, &form).await?;
            after_sign_in(&app, client, envelope.response).await
        }
    }
}

/// A new code for the same step.
pub async fn resend_code(app: ClerkApp, step: CodeStep) -> Result<CodeStep, AppError> {
    let mut client = step.client.clone();
    let (path, strategy) = match &step.target {
        CodeTarget::SignUp(id) => (format!("/client/sign_ups/{id}/prepare_verification"), "email_code"),
        CodeTarget::FirstFactor { sign_in, strategy } => {
            (format!("/client/sign_ins/{sign_in}/prepare_first_factor"), strategy.as_str())
        }
        CodeTarget::SecondFactor { sign_in, strategy } => {
            (format!("/client/sign_ins/{sign_in}/prepare_second_factor"), strategy.as_str())
        }
    };
    call(&app, reqwest::Method::POST, &path, &mut client, &[("strategy", strategy)])
        .await
        .map_err(Failure::into_error)?;
    Ok(CodeStep { client, ..step })
}

/// A sign-in with GitHub, Google or Discord halfway: the loopback port is already listening and Clerk has
/// already the attempt, only the user going through the browser is missing. `Debug` prints nothing: inside
/// there are the client token and the provider's address.
pub struct Oauth {
    listener: TcpListener,
    state: String,
    client: String,
    sign_in: String,
    url: Secret,
}

impl Oauth {
    /// The provider's address to open in the browser.
    pub fn url(&self) -> &Secret {
        &self.url
    }

    /// A sign-in halfway for the tests: the loopback sits on a free port chosen by the system and Clerk
    /// is not involved. Waiting for its return (`oauth_finish`) is not provided for.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn for_tests(url: &str) -> Result<Self, AppError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|_| AppError::AuthPortBusy(0))?;
        Ok(Self {
            listener,
            state: "stato-di-prova".into(),
            client: "client-di-prova".into(),
            sign_in: "si_prova".into(),
            url: Secret::new(url),
        })
    }
}

impl std::fmt::Debug for Oauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Oauth([redacted])")
    }
}

/// First part of the browser sign-in: it puts the loopback port listening (before opening
/// the browser, so the return finds no one) and asks Clerk for the provider's address. Opening it
/// is not our job: it is an effect of the core (`platform::open_url`).
pub async fn oauth_begin(app: &ClerkApp, provider: Provider) -> Result<Oauth, AppError> {
    let listener =
        TcpListener::bind(("127.0.0.1", app.port)).await.map_err(|_| AppError::AuthPortBusy(app.port))?;
    let state = random_state()?;
    let redirect = format!("http://127.0.0.1:{}/callback?state={state}", app.port);
    let mut client = new_client(app).await?;
    let form = [("strategy", provider.strategy()), ("redirect_url", redirect.as_str())];
    let envelope: Envelope<SignInBody> =
        call_as(app, reqwest::Method::POST, "/client/sign_ins", &mut client, &form).await?;
    let sign_in = envelope.response;
    let url = sign_in
        .first_factor_verification
        .and_then(|v| v.external_verification_redirect_url)
        .ok_or(AppError::AuthFailed)?;
    Ok(Oauth { listener, state, client, sign_in: sign_in.id, url: Secret::new(url) })
}

/// Second part: waits for the browser's return (at most [`LOGIN_TIMEOUT`]) and closes the sign-in. A
/// new account is registered on the fly (`transfer`).
pub async fn oauth_finish(app: ClerkApp, oauth: Oauth) -> Result<Step, AppError> {
    let Oauth { listener, state, mut client, sign_in, url: _ } = oauth;
    let nonce = tokio::time::timeout(LOGIN_TIMEOUT, receive_callback(&listener, &state))
        .await
        .map_err(|_| AppError::AuthCancelled)??;

    let path = format!("/client/sign_ins/{sign_in}");
    let query: Vec<(&str, &str)> =
        nonce.as_deref().map(|n| vec![("rotating_token_nonce", n)]).unwrap_or_default();
    let envelope: Envelope<SignInBody> =
        call_as(&app, reqwest::Method::GET, &path, &mut client, &query).await?;
    let sign_in = envelope.response;
    if sign_in.status == "complete" {
        let session = sign_in.created_session_id.ok_or(AppError::AuthFailed)?;
        return finish(&app, client, &session).await;
    }
    // The client may already have the session even if the attempt does not say so.
    if let Some(session) = envelope.client.and_then(|c| c.last_active_session_id) {
        return finish(&app, client, &session).await;
    }
    let transferable =
        sign_in.first_factor_verification.and_then(|v| v.status).is_some_and(|s| s == "transferable");
    if !transferable {
        return Err(AppError::AuthFailed);
    }
    let envelope: Envelope<SignUpBody> =
        call_as(&app, reqwest::Method::POST, "/client/sign_ups", &mut client, &[("transfer", "true")])
            .await?;
    after_sign_up(&app, client, envelope.response).await
}

/// At startup: the saved session, if Clerk still considers it active.
pub async fn restore(app: ClerkApp) -> Result<(Session, ClerkUser), AppError> {
    let stored = load_session(&app).ok_or(AppError::AuthExpired)?;
    let mut client = stored.client;
    let envelope: Envelope<ClientBody> =
        call_as(&app, reqwest::Method::GET, "/client", &mut client, &[]).await.map_err(|err| {
            if matches!(err, AppError::Auth(_) | AppError::AuthFailed) { AppError::AuthExpired } else { err }
        })?;
    let found = envelope
        .response
        .sessions
        .into_iter()
        .find(|s| s.id == stored.session && s.status == "active")
        .ok_or(AppError::AuthExpired)?;
    let user = found.user.map(ClerkUser::from).unwrap_or_default();
    let session = Session::new(app.clone(), client, stored.session);
    store(&app, &*session.state.lock().await).await;
    Ok((session, user))
}

fn random_state() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| AppError::AuthFailed)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Result of the return from the browser: the nonce with which Clerk renews the client (if there is one) when
/// `state` matches, otherwise an error.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<Option<String>, AppError> {
    let target = request_line
        .strip_prefix("GET ")
        .and_then(|rest| rest.split(' ').next())
        .ok_or(AppError::AuthFailed)?;
    let query = target.strip_prefix("/callback?").ok_or(AppError::AuthFailed)?;
    let mut nonce = None;
    let mut state = None;
    let mut denied = false;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "rotating_token_nonce" => nonce = decode(value).filter(|n| !n.is_empty()),
            "state" => state = decode(value),
            "error" | "__clerk_status" if value != "complete" && value != "verified" => denied = true,
            _ => {}
        }
    }
    // A different state is a callback we did not ask for: it is refused.
    if state.as_deref() != Some(expected_state) {
        return Err(AppError::AuthFailed);
    }
    if denied {
        return Err(AppError::AuthCancelled);
    }
    Ok(nonce)
}

/// Waits for the right callback: other requests (for example `favicon.ico`) get a `404`.
async fn receive_callback(listener: &TcpListener, state: &str) -> Result<Option<String>, AppError> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|_| AppError::AuthFailed)?;
        let mut buffer = vec![0u8; MAX_CALLBACK_BYTES];
        let mut read = 0;
        while read < buffer.len() && !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
            match socket.read(&mut buffer[read..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => read += n,
            }
        }
        let request = String::from_utf8_lossy(&buffer[..read]);
        let first_line = request.lines().next().unwrap_or_default();
        if !first_line.starts_with("GET /callback?") {
            let _ = socket
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }
        let result = parse_callback(first_line, state);
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CALLBACK_PAGE}",
            CALLBACK_PAGE.len()
        );
        let _ = socket.write_all(reply.as_bytes()).await;
        return result;
    }
}

// ---------- Session on disk ----------

#[derive(Serialize, Deserialize)]
struct StoredSession {
    client: String,
    session: String,
}

impl ClerkApp {
    /// There is a saved session to resume (the file exists). Synchronous read: it is done by whoever starts the
    /// core, once, before the first snapshot.
    pub fn has_stored_session(&self) -> bool {
        self.session_file.as_deref().is_some_and(Path::exists)
    }
}

/// The saved session, if there is one and it can be read. The file from the versions with the browser login (a
/// refresh token) is no longer read: one signs in once by hand.
fn load_session(app: &ClerkApp) -> Option<StoredSession> {
    let text = std::fs::read_to_string(app.session_file.as_deref()?).ok()?;
    serde_json::from_str::<StoredSession>(&text)
        .ok()
        .filter(|s| !s.client.is_empty() && !s.session.is_empty())
}

/// Saves the session; a write error only costs a new sign-in at the next startup.
async fn store(app: &ClerkApp, state: &SessionState) {
    let stored = StoredSession { client: state.client.clone(), session: state.session_id.clone() };
    let _ = write_session(app, &stored).await;
}

/// In a file readable only by the user (0600 on Unix; on Windows the profile folder is already
/// private). Without `session_file` nothing is written.
async fn write_session(app: &ClerkApp, stored: &StoredSession) -> Result<(), AppError> {
    let Some(path) = app.session_file.clone() else { return Ok(()) };
    let io = |_| AppError::Io(path.display().to_string());
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    let json = serde_json::to_vec(stored).map_err(|_| AppError::Io("session".into()))?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&path).await.map_err(io)?;
    file.write_all(&json).await.map_err(io)?;
    file.flush().await.map_err(io)
}

/// Removes the session from disk (and only from there: Clerk is not told).
pub async fn forget_session(app: &ClerkApp) -> Result<(), AppError> {
    let Some(path) = app.session_file.clone() else { return Ok(()) };
    match tokio::fs::remove_file(&path).await {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(AppError::Io(path.display().to_string()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_with_matching_state_gives_the_nonce() {
        let nonce = parse_callback("GET /callback?state=s1&rotating_token_nonce=ab%2Fc HTTP/1.1", "s1");
        assert_eq!(nonce, Ok(Some("ab/c".into())));
    }

    #[test]
    fn callback_without_nonce_is_still_a_return() {
        assert_eq!(parse_callback("GET /callback?state=s1 HTTP/1.1", "s1"), Ok(None));
    }

    #[test]
    fn callback_with_another_state_is_rejected() {
        assert_eq!(
            parse_callback("GET /callback?rotating_token_nonce=x&state=evil HTTP/1.1", "s1"),
            Err(AppError::AuthFailed)
        );
    }

    #[test]
    fn denied_login_is_a_cancellation() {
        let result = parse_callback("GET /callback?state=s1&error=access_denied HTTP/1.1", "s1");
        assert_eq!(result, Err(AppError::AuthCancelled));
    }

    #[test]
    fn clerk_errors_speak_italian_or_keep_clerks_words() {
        assert_eq!(
            clerk_message("form_password_incorrect", Some("Password is incorrect")),
            "Password errata."
        );
        assert_eq!(clerk_message("brand_new_code", Some("Something new")), "Something new");
        assert_eq!(clerk_message("brand_new_code", None), "Accesso non riuscito. Riprova.");
    }

    #[test]
    fn the_primary_email_is_the_one_shown() {
        let user = UserBody {
            username: Some(String::new()),
            first_name: Some("Psy".into()),
            last_name: None,
            image_url: None,
            email_addresses: vec![
                EmailBody { id: "a".into(), email_address: "old@example.com".into() },
                EmailBody { id: "b".into(), email_address: "main@example.com".into() },
            ],
            primary_email_address_id: Some("b".into()),
        };
        let user = ClerkUser::from(user);
        assert_eq!(
            (user.username, user.name, user.email.as_deref()),
            (None, Some("Psy".into()), Some("main@example.com"))
        );
    }

    /// Against a real development Clerk instance, with a test account (`…+clerk_test@…`, whose
    /// code is always `424242`): `PSYTIFY_LIVE_CLERK_ISSUER`, `PSYTIFY_LIVE_IDENTIFIER`,
    /// `PSYTIFY_LIVE_PASSWORD`, `PSYTIFY_LIVE_SESSION_DIR` (an empty folder where the
    /// session is written) and, to try the backend, `PSYTIFY_LIVE_API`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a Clerk development instance and a test account"]
    async fn live_password_sign_in_with_a_code() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        // The session goes in a folder of its own (`PSYTIFY_LIVE_SESSION_DIR`), never in the user's data.
        let session_file = Some(PathBuf::from(env("PSYTIFY_LIVE_SESSION_DIR")).join("session.json"));
        let app = ClerkApp { issuer: env("PSYTIFY_LIVE_CLERK_ISSUER"), port: 53682, session_file };
        let mut step = sign_in(app.clone(), env("PSYTIFY_LIVE_IDENTIFIER"), env("PSYTIFY_LIVE_PASSWORD"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        if let Step::Code(code) = step {
            step = submit_code(app.clone(), code, "424242".into()).await.unwrap_or_else(|e| panic!("{e}"));
        }
        let Step::Done(session, _) = step else { panic!("still waiting for a code") };
        let token = session.bearer().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(token.split('.').count(), 3);
        assert_eq!(session.bearer().await.ok(), Some(token.clone()), "the token is reused while fresh");
        let (restored, _) = restore(app).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(restored.bearer().await.is_ok());
        if let Ok(api) = std::env::var("PSYTIFY_LIVE_API") {
            let api = crate::api::ApiClient::new(&api).unwrap_or_else(|e| panic!("{e}"));
            let me = api.me(&token).await.unwrap_or_else(|e| panic!("{e}"));
            // No user data in the message: the tests' output ends up in the logs.
            assert!(me.user_id.starts_with("user_"), "the backend did not recognise the session");
        }
        session.sign_out().await.unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn steps_never_print_the_client_token() {
        let step = CodeStep {
            client: "secret-client".into(),
            target: CodeTarget::SignUp("sua_1".into()),
            sent_to: "p***@example.com".into(),
        };
        let session = Session::new(
            ClerkApp { issuer: "https://x.clerk.accounts.dev".into(), port: 53682, session_file: None },
            "secret-client".into(),
            "sess_1".into(),
        );
        assert!(!format!("{step:?} {session:?}").contains("secret"));
    }

    /// The session file sits where `ClerkApp` says (a core's folder, not one per process),
    /// is readable only by the user, is reread and forgotten; without a path nothing is written.
    #[tokio::test(flavor = "current_thread")]
    async fn the_session_file_belongs_to_the_core_that_names_it() {
        let dir = std::env::temp_dir().join(format!("psytify-auth-test-{}", uuid::Uuid::new_v4()));
        let app = ClerkApp {
            issuer: "https://x.clerk.accounts.dev".into(),
            port: 53682,
            session_file: Some(dir.join("session.json")),
        };
        assert!(!app.has_stored_session());
        let stored = StoredSession { client: "client-1".into(), session: "sess_1".into() };
        write_session(&app, &stored).await.unwrap();
        assert!(app.has_stored_session());
        let read = load_session(&app).unwrap();
        assert_eq!((read.client.as_str(), read.session.as_str()), ("client-1", "sess_1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("session.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user reads it");
        }
        forget_session(&app).await.unwrap();
        assert!(!app.has_stored_session());
        forget_session(&app).await.unwrap();

        let nowhere = ClerkApp { session_file: None, ..app };
        write_session(&nowhere, &stored).await.unwrap();
        assert!(load_session(&nowhere).is_none() && !nowhere.has_stored_session());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A file without a usable session (empty, or from the old login with the refresh token) is not valid.
    #[test]
    fn an_unusable_session_file_is_not_restored() {
        let dir = std::env::temp_dir().join(format!("psytify-auth-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.json");
        let app = ClerkApp {
            issuer: "https://x.clerk.accounts.dev".into(),
            port: 1,
            session_file: Some(file.clone()),
        };
        for text in ["", "{}", r#"{"refresh_token":"x"}"#, r#"{"client":"","session":"s"}"#] {
            std::fs::write(&file, text).unwrap();
            assert!(load_session(&app).is_none(), "{text}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn secrets_and_half_open_logins_never_print() {
        let secret = Secret::new("hunter2");
        assert_eq!(secret.expose(), "hunter2");
        assert!(!format!("{secret:?}").contains("hunter"));
    }
}
