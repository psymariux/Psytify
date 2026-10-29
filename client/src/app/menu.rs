//! Lo stato di ciò che galleggia sopra la pagina: il menu contestuale (clic destro) e le finestre
//! di dialogo (nuova playlist, rinomina, elimina). Qui c'è solo l'insieme dei dati; come si
//! disegnano sta in `ui::context_menu` e `ui::dialog`.

use std::time::Instant;

use iced::Point;
use uuid::Uuid;

use crate::api::{AlbumSummary, Track};
use crate::config::{FollowedArtist, SavedAlbum};

/// Da dove viene il brano su cui si è fatto clic: cambia le voci del menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSource {
    /// Catalogo, ricerca, libreria: nessuna voce in più.
    Catalogue,
    /// Un brano della coda, alla sua posizione nell'ordine di ascolto.
    Queue(usize),
    /// Una playlist locale, e la posizione del brano al suo interno.
    Playlist(Uuid, usize),
    /// I brani che piacciono.
    Liked,
}

/// Su che cosa si è aperto il menu.
#[derive(Debug, Clone)]
pub enum MenuTarget {
    Track {
        track: Box<Track>,
        source: TrackSource,
    },
    /// Una tessera di album: si apre, si salva, si va all'artista.
    Album(Box<SavedAlbum>),
    /// Un artista (tessera tonda): si apre o si segue.
    Artist(Box<FollowedArtist>),
    /// Una playlist nella barra laterale.
    Playlist(Uuid),
    /// Il menu del player: vista immersiva, equalizzatore, riproduzione automatica e timer.
    Player,
}

impl MenuTarget {
    /// Il menu di un brano.
    pub fn track(track: &Track, source: TrackSource) -> Self {
        Self::Track { track: Box::new(track.clone()), source }
    }

    pub fn album(album: &AlbumSummary) -> Self {
        Self::Album(Box::new(SavedAlbum {
            id: album.id,
            title: album.title.clone(),
            artist: album.artist.clone(),
            cover_url: album.cover_url.clone(),
        }))
    }

    pub fn artist(id: Uuid, name: &str, cover_url: Option<&str>) -> Self {
        Self::Artist(Box::new(FollowedArtist {
            id,
            name: name.to_string(),
            cover_url: cover_url.map(str::to_string),
        }))
    }
}

/// Il menu aperto, con il punto del clic nella finestra e l'istante da cui compare.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub target: MenuTarget,
    pub at: Point,
    pub since: Instant,
    /// L'elenco delle playlist è aperto accanto al menu ("Aggiungi a una playlist").
    pub playlists: bool,
    /// L'elenco delle qualità di download è aperto accanto al menu ("Scarica in…").
    pub downloads: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    /// Nuova playlist: il nome digitato e i brani con cui nasce (uno solo dal menu di un brano,
    /// tutta la coda dal pannello, nessuno dalla barra laterale).
    NewPlaylist {
        name: String,
        seed: Vec<Track>,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    ConfirmDelete(Uuid),
}

/// La finestra di dialogo aperta, con l'istante da cui compare.
#[derive(Debug, Clone)]
pub struct OpenDialog {
    pub dialog: Dialog,
    pub since: Instant,
}
