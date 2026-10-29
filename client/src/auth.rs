//! Login facoltativo con Clerk, dentro l'app e tutto in Rust: la schermata «Accedi a Psytify»
//! parla con la Frontend API di Clerk in modalità nativa (`_is_native=1`), come le app mobili.
//!
//! - Email o nome utente e password; se Clerk chiede una conferma (dispositivo nuovo, secondo
//!   fattore) manda un codice via email che si scrive nell'app.
//! - Registrazione con email, nome utente e password, confermata dal codice via email.
//! - GitHub, Google e Discord passano dal browser e tornano all'app su
//!   `http://127.0.0.1:<porta>/callback`.
//!
//! Clerk riconosce l'app da un token del client (header `Authorization` senza schema), che può
//! cambiare a ogni risposta. Il token del client e l'id della sessione stanno in
//! `<config_dir>/psytify/session.json`, leggibile solo dall'utente; il token di sessione da
//! mandare al backend dura un minuto, resta in memoria e si rinnova da sé (`Session::bearer`).
//! Nessun token finisce nei log o nei messaggi d'errore.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::error::AppError;

/// Tempo concesso all'utente per completare il login nel browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// La richiesta di callback del browser è una riga GET con qualche header.
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
/// Il token di sessione vale un minuto: lo si rinnova un po' prima.
const SESSION_TOKEN_REUSE: Duration = Duration::from_secs(45);
const CALLBACK_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Psytify</title>\
<body style=\"background:#000;color:#fff;font-family:sans-serif;padding:48px\">\
<p style=\"color:#c6f432;font-weight:600\">Psytify</p>\
<p>Accesso completato. Puoi tornare all'app.</p></body>";

/// L'istanza Clerk. Niente di segreto: la Frontend API è pubblica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClerkApp {
    /// Frontend API dell'istanza, per esempio `https://your-instance.clerk.accounts.dev`.
    pub issuer: String,
    /// Porta di loopback per il ritorno dal browser (GitHub, Google, Discord).
    pub port: u16,
}

impl ClerkApp {
    fn url(&self, path: &str) -> String {
        format!("{}/v1{path}?_is_native=1", self.issuer)
    }
}

/// Gli accessi che passano dal browser.
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

/// L'utente di Clerk, quanto serve per proporre il profilo di Psytify.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClerkUser {
    pub username: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub image_url: Option<String>,
}

/// Una sessione di Clerk. Si clona a basso costo; `Debug` non stampa i token.
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

    /// Il token da mandare al backend come `Authorization: Bearer`. Rinnovato quando scade;
    /// `AuthExpired` se Clerk ha chiuso la sessione.
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
            store(&state).await;
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

    /// Chiude la sessione su Clerk e dimentica quella su disco. Un errore di rete non impedisce di
    /// uscire.
    pub async fn sign_out(self) -> Result<(), AppError> {
        {
            let state = self.state.lock().await;
            let path = format!("/client/sessions/{}/remove", state.session_id);
            let mut client = state.client.clone();
            let _ = call(&self.app, reqwest::Method::POST, &path, &mut client, &[]).await;
        }
        forget_session().await
    }
}

/// Un passo del login: fatto, oppure serve un codice arrivato via email.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Done(Session, ClerkUser),
    Code(CodeStep),
}

/// Il codice da chiedere all'utente: dove l'ha ricevuto e a quale tentativo appartiene.
#[derive(Clone, PartialEq, Eq)]
pub struct CodeStep {
    client: String,
    target: CodeTarget,
    /// L'indirizzo, in parte nascosto da Clerk, a cui è arrivato il codice.
    pub sent_to: String,
}

impl std::fmt::Debug for CodeStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeStep").field("sent_to", &self.sent_to).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodeTarget {
    SignUp(String),
    FirstFactor { sign_in: String, strategy: String },
    SecondFactor { sign_in: String, strategy: String },
}

// ---------- Risposte di Clerk ----------

/// Clerk manda `null` dove una lista è vuota (i fattori di un accesso già completo).
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

// ---------- Errori ----------

/// Un errore della Frontend API, prima di diventare un `AppError` da mostrare.
enum Failure {
    /// Clerk ha risposto con un errore: il messaggio è già in italiano.
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

/// I codici d'errore di Clerk più comuni, in italiano; gli altri con il testo di Clerk.
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

/// Una chiamata alla Frontend API. `client` è il token del client: si manda e, se Clerk ne
/// restituisce uno nuovo, si aggiorna.
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

/// Un client nuovo: il punto di partenza di ogni accesso.
async fn new_client(app: &ClerkApp) -> Result<String, AppError> {
    let mut client = String::new();
    call(app, reqwest::Method::POST, "/client", &mut client, &[]).await.map_err(Failure::into_error)?;
    if client.is_empty() { Err(AppError::AuthFailed) } else { Ok(client) }
}

/// La sessione appena creata, con l'utente: la si salva su disco.
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
    store(&*session.state.lock().await).await;
    Ok(Step::Done(session, user))
}

/// Il fattore con un codice (prima l'email, poi il telefono) fra quelli che Clerk propone.
fn code_factor(factors: &[FactorBody]) -> Option<&FactorBody> {
    factors
        .iter()
        .find(|f| f.strategy == "email_code")
        .or_else(|| factors.iter().find(|f| f.strategy == "phone_code"))
}

/// Prepara il codice per il fattore indicato e restituisce il passo da mostrare.
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

/// Dove porta lo stato di un tentativo di accesso.
async fn after_sign_in(app: &ClerkApp, client: String, sign_in: SignInBody) -> Result<Step, AppError> {
    match sign_in.status.as_str() {
        "complete" => {
            let session = sign_in.created_session_id.ok_or(AppError::AuthFailed)?;
            finish(app, client, &session).await
        }
        // Dispositivo nuovo o secondo fattore: Clerk manda un codice.
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

// ---------- Flussi ----------

/// Accesso con email o nome utente e password.
pub async fn sign_in(app: ClerkApp, identifier: String, password: String) -> Result<Step, AppError> {
    let mut client = new_client(&app).await?;
    let form = [("strategy", "password"), ("identifier", identifier.trim()), ("password", password.as_str())];
    let envelope: Envelope<SignInBody> =
        call_as(&app, reqwest::Method::POST, "/client/sign_ins", &mut client, &form).await?;
    after_sign_in(&app, client, envelope.response).await
}

/// Registrazione: l'email si conferma con il codice che Clerk manda.
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

/// Il codice scritto dall'utente.
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

/// Un nuovo codice per lo stesso passo.
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

/// Accesso con GitHub, Google o Discord: il browser apre il provider e torna sul loopback. Un
/// account nuovo viene registrato al volo (`transfer`).
pub async fn sign_in_with(app: ClerkApp, provider: Provider) -> Result<Step, AppError> {
    let listener =
        TcpListener::bind(("127.0.0.1", app.port)).await.map_err(|_| AppError::AuthPortBusy(app.port))?;
    let state = random_state()?;
    let redirect = format!("http://127.0.0.1:{}/callback?state={state}", app.port);
    let mut client = new_client(&app).await?;
    let form = [("strategy", provider.strategy()), ("redirect_url", redirect.as_str())];
    let envelope: Envelope<SignInBody> =
        call_as(&app, reqwest::Method::POST, "/client/sign_ins", &mut client, &form).await?;
    let sign_in = envelope.response;
    let url = sign_in
        .first_factor_verification
        .and_then(|v| v.external_verification_redirect_url)
        .ok_or(AppError::AuthFailed)?;
    if !crate::open_url(&url) {
        return Err(AppError::AuthFailed);
    }
    let nonce = tokio::time::timeout(LOGIN_TIMEOUT, receive_callback(&listener, &state))
        .await
        .map_err(|_| AppError::AuthCancelled)??;

    let path = format!("/client/sign_ins/{}", sign_in.id);
    let query: Vec<(&str, &str)> =
        nonce.as_deref().map(|n| vec![("rotating_token_nonce", n)]).unwrap_or_default();
    let envelope: Envelope<SignInBody> =
        call_as(&app, reqwest::Method::GET, &path, &mut client, &query).await?;
    let sign_in = envelope.response;
    if sign_in.status == "complete" {
        let session = sign_in.created_session_id.ok_or(AppError::AuthFailed)?;
        return finish(&app, client, &session).await;
    }
    // Il client può già avere la sessione anche se il tentativo non lo dice.
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

/// All'avvio: la sessione salvata, se Clerk la considera ancora attiva.
pub async fn restore(app: ClerkApp) -> Result<(Session, ClerkUser), AppError> {
    let stored = load_session().ok_or(AppError::AuthExpired)?;
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
    let session = Session::new(app, client, stored.session);
    store(&*session.state.lock().await).await;
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

/// Esito del ritorno dal browser: il nonce con cui Clerk rinnova il client (se c'è) quando lo
/// `state` coincide, altrimenti un errore.
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
    // Uno state diverso è una callback che non abbiamo chiesto noi: si rifiuta.
    if state.as_deref() != Some(expected_state) {
        return Err(AppError::AuthFailed);
    }
    if denied {
        return Err(AppError::AuthCancelled);
    }
    Ok(nonce)
}

/// Aspetta la callback giusta: altre richieste (per esempio `favicon.ico`) ricevono `404`.
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

// ---------- Sessione su disco ----------

#[derive(Serialize, Deserialize)]
struct StoredSession {
    client: String,
    session: String,
}

pub fn session_path() -> Option<PathBuf> {
    crate::config::dir().map(|dir| dir.join("session.json"))
}

/// La sessione salvata, se c'è e si legge. Il file delle versioni con il login nel browser (un
/// refresh token) non si legge più: si rientra una volta a mano.
fn load_session() -> Option<StoredSession> {
    let text = std::fs::read_to_string(session_path()?).ok()?;
    serde_json::from_str::<StoredSession>(&text)
        .ok()
        .filter(|s| !s.client.is_empty() && !s.session.is_empty())
}

/// Salva la sessione; un errore di scrittura costa solo un nuovo accesso al prossimo avvio.
async fn store(state: &SessionState) {
    let stored = StoredSession { client: state.client.clone(), session: state.session_id.clone() };
    let _ = write_session(&stored).await;
}

/// In un file leggibile solo dall'utente (0600 su Unix; su Windows la cartella del profilo è già
/// privata).
async fn write_session(stored: &StoredSession) -> Result<(), AppError> {
    let path = session_path().ok_or_else(|| AppError::Io("session".into()))?;
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

pub async fn forget_session() -> Result<(), AppError> {
    let Some(path) = session_path() else { return Ok(()) };
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

    /// Contro un'istanza Clerk di sviluppo vera, con un account di test (`…+clerk_test@…`, il cui
    /// codice è sempre `424242`): `PSYTIFY_LIVE_CLERK_ISSUER`, `PSYTIFY_LIVE_IDENTIFIER`,
    /// `PSYTIFY_LIVE_PASSWORD` e, per provare il backend, `PSYTIFY_LIVE_API`. Scrive la sessione
    /// nella cartella di configurazione: lanciarlo con un `XDG_CONFIG_HOME` a parte.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a Clerk development instance and a test account"]
    async fn live_password_sign_in_with_a_code() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let app = ClerkApp { issuer: env("PSYTIFY_LIVE_CLERK_ISSUER"), port: 53682 };
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
            // Niente dati dell'utente nel messaggio: l'output dei test finisce nei log.
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
            ClerkApp { issuer: "https://x.clerk.accounts.dev".into(), port: 53682 },
            "secret-client".into(),
            "sess_1".into(),
        );
        assert!(!format!("{step:?} {session:?}").contains("secret"));
    }
}
