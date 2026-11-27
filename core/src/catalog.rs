//! Aiuti del catalogo che servono sia alla logica sia alle viste: stanno qui perché il core non
//! dipende dall'interfaccia.

use crate::api::ArtistPage;
use crate::error::AppError;

/// Stato di un dato caricato dalla rete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(AppError),
}

// A mano: `derive(Default)` pretenderebbe `T: Default`, e `Arc<Album>` & co. non lo sono.
#[allow(clippy::derivable_impls)]
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self::Idle
    }
}

impl<T> Load<T> {
    /// Non c'è niente da mostrare e niente in arrivo: da chiedere, o da riprovare dopo un errore.
    pub fn needs_load(&self) -> bool {
        matches!(self, Self::Idle | Self::Failed(_))
    }

    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// L'esito di una richiesta come stato.
    pub fn of(result: Result<T, AppError>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Failed(err),
        }
    }
}

/// Il testo della ricerca com'è per il backend: senza caratteri di controllo, da 1 a 100 caratteri.
pub fn clean_query(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let trimmed: String = cleaned.trim().chars().take(MAX_QUERY_CHARS).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Caratteri di una ricerca, al massimo.
pub const MAX_QUERY_CHARS: usize = 100;

/// L'immagine dell'artista è la sua foto ufficiale; senza, la copertina del suo primo album con
/// un'immagine, o di un suo brano.
pub fn portrait_url(page: &ArtistPage) -> Option<&str> {
    page.image_url
        .as_deref()
        .or_else(|| page.albums.iter().find_map(|a| a.cover_url.as_deref()))
        .or_else(|| page.tracks.iter().find_map(|t| t.cover_url.as_deref()))
}
