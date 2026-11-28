//! Catalog helpers needed by both the logic and the views: they live here because the core
//! does not depend on the interface.

use crate::api::ArtistPage;
use crate::error::AppError;

/// State of a piece of data loaded from the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(AppError),
}

// By hand: `derive(Default)` would require `T: Default`, and `Arc<Album>` & co. are not.
#[allow(clippy::derivable_impls)]
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self::Idle
    }
}

impl<T> Load<T> {
    /// Nothing to show and nothing on the way: to ask for, or to retry after an error.
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

    /// The outcome of a request as a state.
    pub fn of(result: Result<T, AppError>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Failed(err),
        }
    }
}

/// The search text as the backend expects it: no control characters, from 1 to 100 characters.
pub fn clean_query(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let trimmed: String = cleaned.trim().chars().take(MAX_QUERY_CHARS).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Maximum characters of a search.
pub const MAX_QUERY_CHARS: usize = 100;

/// The artist image is their official photo; without it, the cover of their first album with
/// an image, or of one of their tracks.
pub fn portrait_url(page: &ArtistPage) -> Option<&str> {
    page.image_url
        .as_deref()
        .or_else(|| page.albums.iter().find_map(|a| a.cover_url.as_deref()))
        .or_else(|| page.tracks.iter().find_map(|t| t.cover_url.as_deref()))
}
