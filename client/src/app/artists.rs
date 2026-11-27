//! La pagina artista: le scelte di vista (filtro della discografia, «Popolari» esteso). Le pagine in
//! memoria, i caricamenti in anticipo e le riletture in silenzio sono del catalogo del core
//! (`core/src/state/catalog.rs`, LRU di 40 pagine), che pubblica la pagina aperta; qui si apre un
//! artista e si scelgono le copertine da scaricare per le sue pagine.

use iced::Task;
use psytify_core::command::{CatalogCmd, Resource};
use uuid::Uuid;

use super::{App, Message};
use crate::api::ArtistPage;

/// Come si filtra la discografia nella pagina di un artista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Discography {
    #[default]
    All,
    Albums,
    Singles,
}

impl Discography {
    pub const ALL: [Self; 3] = [Self::All, Self::Albums, Self::Singles];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Tutto",
            Self::Albums => "Album",
            Self::Singles => "Singoli",
        }
    }
}

/// Le scelte di vista della pagina artista.
#[derive(Debug, Default)]
pub struct ArtistsState {
    pub filter: Discography,
    /// «Popolari» mostra dieci brani invece di cinque.
    pub more_popular: bool,
}

impl App {
    /// Apre la pagina di un artista: il core la serve dalla cache se c'è (e la rilegge in silenzio se è
    /// vecchia), altrimenti la carica. Un'altra pagina artista rimette a posto filtro e «mostra altri».
    pub(super) fn open_artist(&mut self, id: Uuid) -> Task<Message> {
        if !matches!(&self.artist, Some((open, _)) if *open == id) {
            self.artists.filter = Discography::All;
            self.artists.more_popular = false;
        }
        self.send_catalog(CatalogCmd::Ensure(Resource::Artist(id)))
    }
}

/// Le copertine di una pagina artista, nell'ordine in cui si vedono: la foto, gli album, i brani.
pub(super) fn page_covers(page: &ArtistPage, limit: usize) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for url in page
        .image_url
        .as_deref()
        .into_iter()
        .chain(page.albums.iter().filter_map(|a| a.cover_url.as_deref()))
        .chain(page.tracks.iter().filter_map(|t| t.cover_url.as_deref()))
    {
        if !urls.iter().any(|known| known == url) {
            urls.push(url.to_owned());
        }
        if urls.len() == limit {
            break;
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Track;
    use crate::api::models::fixtures::track as fixture;

    fn page(n: u128) -> ArtistPage {
        ArtistPage {
            id: Uuid::from_u128(n),
            name: format!("Artista {n}"),
            image_url: None,
            albums: Vec::new(),
            tracks: Vec::new(),
        }
    }

    fn track(n: u128, cover: Option<&str>) -> Track {
        Track { cover_url: cover.map(str::to_owned), ..fixture(n) }
    }

    #[test]
    fn page_covers_are_unique_and_capped() {
        let mut p = page(1);
        p.tracks = vec![track(1, Some("a")), track(2, Some("a")), track(3, Some("b")), track(4, None)];
        assert_eq!(page_covers(&p, usize::MAX), vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(page_covers(&p, 1), vec!["a".to_owned()]);
    }
}
