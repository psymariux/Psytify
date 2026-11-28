//! Where a page is: what the app keeps in its back stack. Identifiers cross as strings; a route
//! also has a text key ([`route_key`]) so the app can save its back stack across a process death.

use psytify_core::api::{SourceKind, SourcePlaylist};
use uuid::Uuid;

/// The external sources a page can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Source {
    YouTube,
    SoundCloud,
    Bandcamp,
    /// The official catalogue (Deezer): the server finds the audio when a track is played.
    Official,
}

impl Source {
    pub(crate) fn of(kind: SourceKind) -> Self {
        match kind {
            SourceKind::YouTube => Self::YouTube,
            SourceKind::SoundCloud => Self::SoundCloud,
            SourceKind::Bandcamp => Self::Bandcamp,
            SourceKind::Deezer => Self::Official,
        }
    }

    pub(crate) fn kind(self) -> SourceKind {
        match self {
            Self::YouTube => SourceKind::YouTube,
            Self::SoundCloud => SourceKind::SoundCloud,
            Self::Bandcamp => SourceKind::Bandcamp,
            Self::Official => SourceKind::Deezer,
        }
    }
}

/// An album or a playlist of an external source, as the search or an artist page found it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SourceRef {
    pub source: Source,
    pub id: String,
    pub title: String,
    pub owner: String,
    pub cover_url: Option<String>,
    /// `album`, `single`, `ep` or `playlist` (official catalogue only).
    pub kind: String,
    pub released_on: Option<String>,
}

impl SourceRef {
    pub(crate) fn of(playlist: &SourcePlaylist) -> Self {
        Self {
            source: Source::of(playlist.source),
            id: playlist.id.clone(),
            title: playlist.title.clone(),
            owner: playlist.owner.clone(),
            cover_url: playlist.cover_url.clone(),
            kind: playlist.kind.clone(),
            released_on: playlist.released_on.clone(),
        }
    }

    pub(crate) fn playlist(&self) -> SourcePlaylist {
        SourcePlaylist {
            source: self.source.kind(),
            id: self.id.clone(),
            title: self.title.clone(),
            owner: self.owner.clone(),
            cover_url: self.cover_url.clone(),
            kind: self.kind.clone(),
            released_on: self.released_on.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Route {
    Search,
    Library,
    Album {
        id: String,
    },
    Artist {
        id: String,
    },
    /// An album or playlist of the official catalogue, Bandcamp or YouTube.
    Source {
        source: SourceRef,
    },
    /// «Brani che ti piacciono».
    Liked,
    Playlist {
        id: String,
    },
    /// A profile, by username.
    Profile {
        handle: String,
    },
    /// «Scaricati».
    Downloads,
    /// «Amici»: search by username, requests, friends.
    Friends,
    /// «Jam»: listening together.
    Jam,
    /// A friend's public playlist, from their profile.
    FriendPlaylist {
        handle: String,
        id: String,
    },
}

impl Route {
    pub(crate) fn album(id: Uuid) -> Self {
        Self::Album { id: id.to_string() }
    }

    pub(crate) fn artist(id: Uuid) -> Self {
        Self::Artist { id: id.to_string() }
    }
}

/// Separates the fields of a source route in its key: not a character of a title or an id.
const SEP: char = '\u{1f}';

/// The route as text, for the app's saved state.
#[uniffi::export]
pub fn route_key(route: &Route) -> String {
    match route {
        Route::Search => "search".to_owned(),
        Route::Library => "library".to_owned(),
        Route::Liked => "liked".to_owned(),
        Route::Downloads => "downloads".to_owned(),
        Route::Friends => "friends".to_owned(),
        Route::Jam => "jam".to_owned(),
        Route::FriendPlaylist { handle, id } => format!("public:{handle}{SEP}{id}"),
        Route::Album { id } => format!("album:{id}"),
        Route::Artist { id } => format!("artist:{id}"),
        Route::Playlist { id } => format!("playlist:{id}"),
        Route::Profile { handle } => format!("profile:{handle}"),
        Route::Source { source } => {
            let fields = [
                source.source.kind().as_str(),
                &source.id,
                &source.title,
                &source.owner,
                source.cover_url.as_deref().unwrap_or(""),
                &source.kind,
                source.released_on.as_deref().unwrap_or(""),
            ];
            format!("source:{}", fields.join(&SEP.to_string()))
        }
    }
}

/// The route of a key written by [`route_key`]; `None` for anything else.
#[uniffi::export]
pub fn route_of_key(key: &str) -> Option<Route> {
    let (kind, rest) = key.split_once(':').unwrap_or((key, ""));
    let some = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    Some(match kind {
        "search" => Route::Search,
        "library" => Route::Library,
        "liked" => Route::Liked,
        "downloads" => Route::Downloads,
        "friends" => Route::Friends,
        "jam" => Route::Jam,
        "public" => {
            let (handle, id) = rest.split_once(SEP)?;
            Route::FriendPlaylist { handle: handle.to_owned(), id: id.to_owned() }
        }
        "album" => Route::Album { id: rest.to_owned() },
        "artist" => Route::Artist { id: rest.to_owned() },
        "playlist" => Route::Playlist { id: rest.to_owned() },
        "profile" => Route::Profile { handle: rest.to_owned() },
        "source" => {
            let fields: Vec<&str> = rest.split(SEP).collect();
            let [source, id, title, owner, cover, kind, released] = fields.as_slice() else { return None };
            let source = [Source::YouTube, Source::SoundCloud, Source::Bandcamp, Source::Official]
                .into_iter()
                .find(|known| known.kind().as_str() == *source)?;
            Route::Source {
                source: SourceRef {
                    source,
                    id: (*id).to_owned(),
                    title: (*title).to_owned(),
                    owner: (*owner).to_owned(),
                    cover_url: some(cover),
                    kind: (*kind).to_owned(),
                    released_on: some(released),
                },
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_comes_back_from_its_key() {
        let source = SourceRef {
            source: Source::Official,
            id: "album/123".into(),
            title: "Discovery: live".into(),
            owner: "Daft Punk".into(),
            cover_url: Some("https://cdn.example.test/c.jpg".into()),
            kind: "album".into(),
            released_on: None,
        };
        let routes = [
            Route::Search,
            Route::Library,
            Route::Liked,
            Route::Downloads,
            Route::Friends,
            Route::Jam,
            Route::FriendPlaylist { handle: "luca".into(), id: Uuid::from_u128(5).to_string() },
            Route::Profile { handle: "luca".into() },
            Route::album(Uuid::from_u128(7)),
            Route::artist(Uuid::from_u128(8)),
            Route::Playlist { id: Uuid::from_u128(9).to_string() },
            Route::Source { source },
        ];
        for route in routes {
            assert_eq!(route_of_key(&route_key(&route)), Some(route.clone()), "{route:?}");
        }
        assert_eq!(route_of_key("nowhere"), None);
        assert_eq!(route_of_key("source:youtube"), None);
    }
}
