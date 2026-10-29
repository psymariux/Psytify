//! Errori dell'applicazione. Sono `Clone` perché viaggiano dentro i `Message` di Iced, e i
//! messaggi non contengono mai URL firmati, token o percorsi dello storage.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("Server non raggiungibile. Controlla la connessione.")]
    Network,
    #[error("Il server non ha risposto in tempo.")]
    Timeout,
    #[error("{message}")]
    Api { status: u16, code: String, message: String },
    #[error("Il collegamento al file è scaduto.")]
    UrlExpired,
    #[error("Il server ha inviato una risposta non valida.")]
    InvalidResponse,
    #[error("Indirizzo del server non valido: {0}")]
    InvalidBaseUrl(String),
    #[error("Nessuna uscita audio disponibile.")]
    AudioDevice,
    #[error("Il file audio non è riproducibile.")]
    AudioDecode,
    #[error("Impossibile spostarsi in questo punto del brano.")]
    Seek,
    #[error("Impossibile scrivere {0}.")]
    Io(String),
    #[error("Accesso non riuscito. Riprova.")]
    AuthFailed,
    #[error("Accesso annullato.")]
    AuthCancelled,
    #[error("La sessione è scaduta: accedi di nuovo.")]
    AuthExpired,
    #[error("La porta {0} è occupata: chiudi l'altro programma o cambia oauth_port.")]
    AuthPortBusy(u16),
    #[error("Il login non è configurato: imposta clerk_issuer e clerk_client_id.")]
    AuthNotConfigured,
    #[cfg(feature = "video")]
    #[error("Videoclip non disponibile.")]
    Video,
}

impl AppError {
    /// Il server non conosce più il contenuto richiesto (`404`).
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404, .. })
    }

    /// Traduce un errore di reqwest senza conservarne il testo, che può contenere l'URL.
    pub fn from_http(err: &reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_decode() {
            Self::InvalidResponse
        } else {
            Self::Network
        }
    }
}
