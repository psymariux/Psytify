//! Login facoltativo con Clerk come provider OAuth 2.0, tutto in Rust: il client apre la pagina
//! di login nel browser, riceve il codice su `http://127.0.0.1:<porta>/callback` e lo scambia con
//! PKCE (nessun client secret: l'app desktop è un client pubblico).
//!
//! Il token di accesso resta in memoria; su disco va solo il refresh token, in
//! `<config_dir>/psytify/session.json`, leggibile solo dall'utente. I token non finiscono mai nei
//! log né nei messaggi d'errore.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::AppError;

/// Tempo concesso all'utente per completare il login nel browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// La richiesta di callback del browser è una riga GET con qualche header.
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
const SCOPES: &str = "openid profile email offline_access";
const CALLBACK_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Psytify</title>\
<body style=\"background:#000;color:#fff;font-family:sans-serif;padding:48px\">\
<p>Accesso completato. Puoi tornare a Psytify.</p></body>";

/// Dove sta l'applicazione OAuth su Clerk. Niente di segreto: issuer e client ID sono pubblici.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClerkApp {
    /// Frontend API dell'istanza, per esempio `https://your-instance.clerk.accounts.dev`.
    pub issuer: String,
    pub client_id: String,
    /// Porta di loopback registrata come redirect URI su Clerk.
    pub port: u16,
}

impl ClerkApp {
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }
}

/// Token di Clerk. `Debug` non li stampa.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens([redacted])")
    }
}

/// base64url senza padding (RFC 4648 §5), come vogliono PKCE e i parametri OAuth.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// `code_challenge` S256: base64url(SHA-256(verifier)).
pub fn code_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn random_token(len: usize) -> Result<String, AppError> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|_| AppError::AuthFailed)?;
    Ok(base64url(&bytes))
}

/// Codifica per query string e form: restano in chiaro solo i caratteri non riservati.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect::<Vec<_>>().join("&")
}

/// URL della pagina di login di Clerk con PKCE S256.
pub fn authorize_url(app: &ClerkApp, challenge: &str, state: &str) -> String {
    let query = form(&[
        ("response_type", "code"),
        ("client_id", &app.client_id),
        ("redirect_uri", &app.redirect_uri()),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ]);
    format!("{}/oauth/authorize?{query}", app.issuer)
}

/// Esito della callback: il codice se `state` coincide, altrimenti un errore.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<String, AppError> {
    let target = request_line
        .strip_prefix("GET ")
        .and_then(|rest| rest.split(' ').next())
        .ok_or(AppError::AuthFailed)?;
    let query = target.strip_prefix("/callback?").ok_or(AppError::AuthFailed)?;
    let mut code = None;
    let mut state = None;
    let mut denied = false;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = decode(value),
            "state" => state = decode(value),
            "error" => denied = true,
            _ => {}
        }
    }
    if denied {
        return Err(AppError::AuthCancelled);
    }
    // Uno state diverso è una callback che non abbiamo chiesto noi: si rifiuta.
    if state.as_deref() != Some(expected_state) {
        return Err(AppError::AuthFailed);
    }
    code.filter(|c| !c.is_empty()).ok_or(AppError::AuthFailed)
}

fn http() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("psytify/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AppError::Network)
}

async fn token_request(app: &ClerkApp, body: String) -> Result<Tokens, AppError> {
    let response = http()?
        .post(format!("{}/oauth/token", app.issuer))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| AppError::from_http(&e))?;
    match response.status().as_u16() {
        200 => response.json::<Tokens>().await.map_err(|_| AppError::InvalidResponse),
        // Codice o refresh token non più validi: serve un nuovo login.
        400 | 401 => Err(AppError::AuthExpired),
        _ => Err(AppError::AuthFailed),
    }
}

/// Login completo: browser, callback sul loopback, scambio del codice.
pub async fn login(app: ClerkApp) -> Result<Tokens, AppError> {
    let verifier = random_token(32)?;
    let state = random_token(16)?;
    let listener =
        TcpListener::bind(("127.0.0.1", app.port)).await.map_err(|_| AppError::AuthPortBusy(app.port))?;
    if !crate::open_url(&authorize_url(&app, &code_challenge(&verifier), &state)) {
        return Err(AppError::AuthFailed);
    }

    let code = tokio::time::timeout(LOGIN_TIMEOUT, receive_code(&listener, &state))
        .await
        .map_err(|_| AppError::AuthCancelled)??;
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &app.redirect_uri()),
        ("client_id", &app.client_id),
        ("code_verifier", &verifier),
    ]);
    token_request(&app, body).await
}

/// Aspetta la callback giusta: altre richieste (per esempio `favicon.ico`) ricevono `404`.
async fn receive_code(listener: &TcpListener, state: &str) -> Result<String, AppError> {
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

/// Nuovo token di accesso dal refresh token salvato.
pub async fn refresh(app: ClerkApp, refresh_token: String) -> Result<Tokens, AppError> {
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
        ("client_id", &app.client_id),
    ]);
    let mut tokens = token_request(&app, body).await?;
    // Se Clerk non ruota il refresh token, si tiene quello vecchio.
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token);
    }
    Ok(tokens)
}

// ---------- Sessione su disco ----------

#[derive(Serialize, Deserialize)]
struct StoredSession {
    refresh_token: String,
}

pub fn session_path() -> Option<PathBuf> {
    crate::config::dir().map(|dir| dir.join("session.json"))
}

/// Refresh token salvato, se c'è e si legge.
pub fn load_refresh_token() -> Option<String> {
    let text = std::fs::read_to_string(session_path()?).ok()?;
    serde_json::from_str::<StoredSession>(&text).ok().map(|s| s.refresh_token).filter(|t| !t.is_empty())
}

/// Salva il refresh token in un file leggibile solo dall'utente (0600 su Unix; su Windows la
/// cartella del profilo è già privata).
pub async fn save_refresh_token(token: String) -> Result<(), AppError> {
    let path = session_path().ok_or_else(|| AppError::Io("session".into()))?;
    let io = |_| AppError::Io(path.display().to_string());
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    let json = serde_json::to_vec(&StoredSession { refresh_token: token })
        .map_err(|_| AppError::Io("session".into()))?;
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

    fn app() -> ClerkApp {
        ClerkApp { issuer: "https://x.clerk.accounts.dev".into(), client_id: "abc_1".into(), port: 53682 }
    }

    #[test]
    fn base64url_matches_rfc_4648_vectors() {
        let cases =
            [("", ""), ("f", "Zg"), ("fo", "Zm8"), ("foo", "Zm9v"), ("foob", "Zm9vYg"), ("fooba", "Zm9vYmE")];
        for (input, expected) in cases {
            assert_eq!(base64url(input.as_bytes()), expected, "{input}");
        }
    }

    #[test]
    fn base64url_uses_the_url_safe_alphabet() {
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn code_challenge_matches_rfc_7636_example() {
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_carries_pkce_and_loopback_redirect() {
        let url = authorize_url(&app(), "CHALLENGE", "STATE");
        assert!(
            url.starts_with(
                "https://x.clerk.accounts.dev/oauth/authorize?response_type=code&client_id=abc_1"
            )
        );
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback"), "{url}");
        assert!(url.contains("code_challenge=CHALLENGE&code_challenge_method=S256&state=STATE"), "{url}");
    }

    #[test]
    fn callback_with_matching_state_gives_the_code() {
        let code = parse_callback("GET /callback?code=ab%2Fc&state=s1 HTTP/1.1", "s1");
        assert_eq!(code, Ok("ab/c".into()));
    }

    #[test]
    fn callback_with_another_state_is_rejected() {
        assert_eq!(
            parse_callback("GET /callback?code=x&state=evil HTTP/1.1", "s1"),
            Err(AppError::AuthFailed)
        );
    }

    #[test]
    fn denied_login_is_a_cancellation() {
        let result = parse_callback("GET /callback?error=access_denied&state=s1 HTTP/1.1", "s1");
        assert_eq!(result, Err(AppError::AuthCancelled));
    }

    #[test]
    fn tokens_are_never_printed() {
        let tokens =
            Tokens { access_token: "secret-access".into(), refresh_token: Some("secret-refresh".into()) };
        assert!(!format!("{tokens:?}").contains("secret"));
    }
}
