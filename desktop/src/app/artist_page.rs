//! La pagina di un artista, come da `PSYTIFY_ART_DIRECTION.md` (Artist) e con le stesse sezioni, nello
//! stesso ordine, del client Iced (`client/src/ui/pages/artist.rs`): la testa con la foto nel cerchio
//! pixel, il nome, album, singoli e fan; Riproduci, Segui / Segui già e Radio. Poi, dal catalogo
//! ufficiale («Altro di …», `ArtistExtras`): *Popolari* (5, «Mostra altri» li mostra tutti) e
//! *Discografia* (schede Tutto, Album, Singoli quando ci sono entrambi; una copertina apre l'album);
//! poi i brani già nel catalogo del server, *Altri artisti nel catalogo*, *Informazioni* e i *Crediti*.
//!
//! La pagina la tiene il catalogo del core (`CatalogView::artist`, LRU di 40, riletta in silenzio dopo
//! 90 s) e «Altro di …» il dominio esterno (`ExternalView::artists`, chiesto dal core da sé quando la
//! pagina è pronta). Tutto sta in una sola lista virtualizzata: la griglia della discografia è fatta di
//! righe di copertine, quante ne stanno nella larghezza, e si costruiscono solo quelle visibili.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, canvas, div, list,
};
use psytify_core::api::{ArtistPage as Artist, SourceItem, SourcePlaylist, Track};
use psytify_core::catalog::{Load, portrait_url};
use psytify_core::command::{CatalogCmd, DownloadsCmd, ExternalCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::FollowedArtist;
use psytify_core::feed::{self, ArtistCard};
use psytify_core::snapshot::{ArtistExtras, CatalogView, DownloadsView, ExternalView, LibraryView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY};
use psytify_design::tokens::size::cover::COVER_XS;
use psytify_design::tokens::size::{TRACK_INDEX_W, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META, TYPE_NUMERIC};
use uuid::Uuid;

use super::album_page::{credit_line, credits, failure, skeleton_page};
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::search_page::source_data;
use super::track_list::{
    RowText, batch_label, download_all_label, download_command, download_state, now_playing, plays,
    with_links,
};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::{PlayState, Skeleton};
use crate::components::navigation::{Tab, Tabs};
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{note, section_heading};
use crate::components::source_row::{SourceData, SourceRow};
use crate::components::tile::AlbumTile;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Brani in *Popolari*; «Mostra altri» li mostra tutti.
const POPULAR: usize = 5;
/// Altri artisti proposti in fondo (al più, e solo quelli che stanno in una riga).
const RELATED: usize = 12;
/// Il lato delle copertine della griglia e dei ritratti degli altri artisti.
const TILE: f32 = 160.0;
/// Lo spazio fra due copertine della griglia e fra due righe.
const TILE_GAP: f32 = SPACE_5;
/// Righe dello scheletro mentre «Altro di …» arriva.
const EXTRAS_SKELETON: usize = 6;

/// Come si filtra la discografia (lo stesso del client Iced, `client/src/app/artists.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Discography {
    #[default]
    All,
    Albums,
    Singles,
}

impl Discography {
    const ALL: [Self; 3] = [Self::All, Self::Albums, Self::Singles];

    fn label(self) -> &'static str {
        match self {
            Self::All => "Tutto",
            Self::Albums => "Album",
            Self::Singles => "Singoli",
        }
    }

    fn keeps(self, release: &SourcePlaylist) -> bool {
        match self {
            Self::All => true,
            Self::Albums => release.kind == "album",
            Self::Singles => release.kind != "album",
        }
    }
}

/// Le sezioni della pagina, nell'ordine del client Iced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Popular,
    Discography,
    /// I brani del server: «Già nel catalogo del server» se c'è anche il catalogo ufficiale, «Brani» se no.
    Server,
    Related,
    About,
    Credits,
}

/// Una riga dopo la testa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    /// Le schede della discografia (solo con album e singoli insieme).
    Tabs,
    Popular(usize),
    /// Una riga della griglia: le uscite da `start`, al più `columns`.
    Releases(usize),
    /// «Altro di …» sta arrivando: righe vuote alla misura vera.
    ExtrasLoading,
    ExtrasFailed,
    /// Né catalogo ufficiale né brani del server.
    Empty,
    Track(usize),
    Related,
    About,
    Credit(usize),
}

/// Che cosa c'è da mostrare: i conteggi, già calcolati. Puro, perché `plan` si possa provare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shape {
    /// `None` finché «Altro di …» non è pronto o è fallito.
    extras: Option<ExtrasShape>,
    extras_failed: bool,
    tracks: usize,
    related: usize,
    credits: usize,
    columns: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ExtrasShape {
    /// Le righe di *Popolari* da mostrare (5 o tutte).
    popular: usize,
    /// Le uscite dopo il filtro.
    releases: usize,
    /// Le schede servono (ci sono album e singoli).
    tabs: bool,
}

/// Le righe della pagina. Pura.
fn plan(shape: Shape) -> Vec<Row> {
    let mut rows = Vec::new();
    match shape.extras {
        Some(extras) => {
            if extras.popular > 0 {
                rows.push(Row::Heading(Section::Popular));
                rows.extend((0..extras.popular).map(Row::Popular));
            }
            if extras.releases > 0 || extras.tabs {
                rows.push(Row::Heading(Section::Discography));
                if extras.tabs {
                    rows.push(Row::Tabs);
                }
                let columns = shape.columns.max(1);
                rows.extend((0..extras.releases).step_by(columns).map(Row::Releases));
            }
            if extras.popular == 0 && extras.releases == 0 && !extras.tabs && shape.tracks == 0 {
                rows.push(Row::Empty);
            }
        }
        None if shape.extras_failed => {
            if shape.tracks == 0 {
                rows.push(Row::ExtrasFailed);
            }
        }
        None => rows.extend((0..EXTRAS_SKELETON).map(|_| Row::ExtrasLoading)),
    }
    if shape.tracks > 0 {
        rows.push(Row::Heading(Section::Server));
        rows.extend((0..shape.tracks).map(Row::Track));
    }
    if shape.related > 0 {
        rows.push(Row::Heading(Section::Related));
        rows.push(Row::Related);
    }
    if shape.tracks > 0 {
        rows.push(Row::Heading(Section::About));
        rows.push(Row::About);
        if shape.credits > 0 {
            rows.push(Row::Heading(Section::Credits));
            rows.extend((0..shape.credits).map(Row::Credit));
        }
    }
    rows
}

/// Quante copertine di lato `TILE` stanno in una riga larga `width` (con i margini della pagina).
fn columns(width: f32) -> usize {
    let inner = (width - 2.0 * SPACE_6).max(TILE);
    // Il conto è su numeri piccoli e positivi: il troncamento è voluto.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fit = ((inner + TILE_GAP) / (TILE + TILE_GAP)).floor() as usize;
    fit.max(1)
}

/// «4 album · 6 singoli ed EP · 12.400 fan» dal catalogo ufficiale; «N brani» finché non c'è.
fn meta(extras: Option<&ArtistExtras>, tracks: usize) -> String {
    let Some(extras) = extras else { return format::tracks(tracks) };
    let mut parts = Vec::new();
    let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
    if albums > 0 {
        parts.push(format!("{albums} album"));
    }
    let others = extras.albums.len() - albums;
    if others > 0 {
        parts.push(format!("{others} singoli ed EP"));
    }
    if extras.artist.fans > 0 {
        parts.push(format::fans(extras.artist.fans));
    }
    if parts.is_empty() { format::tracks(tracks) } else { parts.join(" · ") }
}

/// «2024 · Album» sotto una copertina della discografia.
fn release_subtitle(release: &SourcePlaylist) -> SharedString {
    match release.released_on.as_deref().and_then(|date| date.get(..4)) {
        Some(year) => format!("{year} · {}", release.kind_label()).into(),
        None => release.kind_label().into(),
    }
}

/// Cosa mostra la pagina.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Loading,
    Failed(SharedString),
    Ready,
}

/// Una copertina della discografia, con i testi pronti.
struct Release {
    playlist: SourcePlaylist,
    subtitle: SharedString,
}

pub struct ArtistPage {
    core: Entity<CoreModel>,
    id: Option<Uuid>,
    shown: Shown,
    artist: Option<Arc<Artist>>,
    extras: Option<ArtistExtras>,
    extras_failed: bool,
    tracks: Arc<[Track]>,
    texts: Vec<RowText>,
    popular: Vec<SourceData>,
    releases: Vec<Release>,
    related: Vec<ArtistCard>,
    credits: Vec<SharedString>,
    about: Vec<SharedString>,
    filter: Discography,
    more_popular: bool,
    columns: usize,
    rows: Vec<Row>,
    meta: SharedString,
    portrait: Option<SharedString>,
    following: bool,
    radio: Option<Track>,
    /// «Scarica tutto» dei brani del server, solo quando il catalogo ufficiale non dà i Popolari (come la
    /// barra del client Iced, che allora suona i brani del server); e il conteggio del lotto in corso.
    download_label: Option<&'static str>,
    batch: Option<SharedString>,
    liked: Arc<HashSet<Uuid>>,
    library: Arc<LibraryView>,
    downloads: Arc<DownloadsView>,
    external: Arc<ExternalView>,
    now: Option<(Uuid, PlayState)>,
    list: ListState,
    /// Il posto dell'equalizzatore del brano in corso: lo dipinge `Shell` (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for ArtistPage {}

impl ArtistPage {
    /// Il posto dell'equalizzatore di questa vista, per lo strato di `Shell`.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        Self {
            id: None,
            shown: Shown::Loading,
            artist: None,
            extras: None,
            extras_failed: false,
            tracks: Arc::from(Vec::new()),
            texts: Vec::new(),
            popular: Vec::new(),
            releases: Vec::new(),
            related: Vec::new(),
            credits: Vec::new(),
            about: Vec::new(),
            filter: Discography::All,
            more_popular: false,
            columns: 4,
            rows: Vec::new(),
            meta: SharedString::default(),
            portrait: None,
            following: false,
            radio: None,
            download_label: None,
            batch: batch_label(&snapshot.downloads),
            liked: core.read(cx).liked(),
            library: Arc::clone(&snapshot.library),
            downloads: Arc::clone(&snapshot.downloads),
            external: Arc::clone(&snapshot.external),
            now: now_playing(&snapshot.playback),
            list: new_list(0),
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        }
    }

    /// Mostra l'artista `id`: il core lo serve dalla cache se c'è, altrimenti lo carica. Un altro artista
    /// rimette a posto il filtro e «Mostra altri», come nel client Iced.
    pub fn show(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.core.read(cx).catalog(CatalogCmd::Ensure(Resource::Artist(id)));
        if self.id == Some(id) {
            return;
        }
        self.id = Some(id);
        self.artist = None;
        self.extras = None;
        self.extras_failed = false;
        self.filter = Discography::All;
        self.more_popular = false;
        let snapshot = self.core.read(cx).snapshot();
        self.take(&snapshot.catalog);
        self.take_extras(&snapshot.external);
        self.rebuild();
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut rebuild = false;
        let mut dirty = false;
        if changed.catalog {
            rebuild |= self.take(&snapshot.catalog);
        }
        if changed.external && !Arc::ptr_eq(&snapshot.external, &self.external) {
            self.external = Arc::clone(&snapshot.external);
            rebuild |= self.take_extras(&snapshot.external);
            // «Apro…» sulle righe di *Popolari* mentre il core importa.
            dirty = true;
        }
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            let feed_changed = !Arc::ptr_eq(&snapshot.library.feed, &self.library.feed)
                || snapshot.library.stats.len() != self.library.stats.len();
            self.library = Arc::clone(&snapshot.library);
            self.liked = core.read(cx).liked();
            rebuild |= feed_changed;
            let following = self.is_following();
            if following != self.following {
                self.following = following;
                remeasure_header(&self.list);
            }
            dirty = true;
        }
        if changed.downloads && !Arc::ptr_eq(&snapshot.downloads, &self.downloads) {
            self.downloads = Arc::clone(&snapshot.downloads);
            let batch = batch_label(&self.downloads);
            if batch != self.batch {
                self.batch = batch;
                remeasure_header(&self.list);
            }
            dirty = true;
        }
        if changed.playback {
            let now = now_playing(&snapshot.playback);
            if now != self.now {
                self.now = now;
                dirty = true;
            }
        }
        if rebuild {
            self.rebuild();
        }
        if rebuild || dirty {
            cx.notify();
        }
    }

    fn is_following(&self) -> bool {
        self.id.is_some_and(|id| self.library.followed.iter().any(|artist| artist.id == id))
    }

    /// Prende dal catalogo la pagina di questo artista; `true` se è cambiata.
    fn take(&mut self, catalog: &CatalogView) -> bool {
        let Some(id) = self.id else { return false };
        let (shown, artist) = match &catalog.artist {
            Some((open, Load::Ready(page))) if *open == id => (Shown::Ready, Some(Arc::clone(page))),
            Some((open, Load::Failed(error))) if *open == id => {
                (Shown::Failed(error.to_string().into()), None)
            }
            _ => (Shown::Loading, None),
        };
        let same = match (&artist, &self.artist) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        if same && shown == self.shown {
            return false;
        }
        self.shown = shown;
        self.artist = artist;
        true
    }

    /// Prende «Altro di …» di questo artista; `true` se è cambiato.
    fn take_extras(&mut self, external: &ExternalView) -> bool {
        let Some(id) = self.id else { return false };
        let (extras, failed) = match external.artist(id) {
            Some(Load::Ready(extras)) => (Some(extras), false),
            Some(Load::Failed(_)) => (None, true),
            _ => (None, false),
        };
        let same = match (extras, &self.extras) {
            (Some(new), Some(old)) => {
                Arc::ptr_eq(&new.top, &old.top) && Arc::ptr_eq(&new.albums, &old.albums)
            }
            (None, None) => true,
            _ => false,
        };
        if same && failed == self.extras_failed {
            return false;
        }
        self.extras = extras.cloned();
        self.extras_failed = failed;
        true
    }

    /// Rifà testi e righe: quando cambiano la pagina, «Altro di …», il filtro, le colonne o il feed.
    fn rebuild(&mut self) {
        let Some(artist) = self.artist.clone() else {
            self.rows.clear();
            set_rows(&self.list, 0);
            return;
        };
        self.tracks = Arc::from(artist.tracks.as_slice());
        self.texts = self.tracks.iter().map(RowText::of).collect();
        self.portrait = portrait_url(&artist)
            .or_else(|| self.extras.as_ref().and_then(|e| e.artist.image_url.as_deref()))
            .map(|url| SharedString::from(url.to_owned()));
        self.meta = meta(self.extras.as_ref(), self.tracks.len()).into();
        self.following = self.is_following();
        // La radio parte dal brano più ascoltato di questo artista già nel catalogo del server.
        self.radio = feed::popular(&self.tracks, self.library.stats.as_slice(), 1).into_iter().next();
        let has_top = self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty());
        self.download_label = if has_top { None } else { download_all_label(&self.tracks) };
        let extras_shape = self.extras.as_ref().map(|extras| {
            self.popular = extras
                .top
                .iter()
                .take(if self.more_popular { extras.top.len() } else { POPULAR })
                .enumerate()
                .map(|(position, item)| source_data(position, item))
                .collect();
            let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
            let singles = extras.albums.len() - albums;
            let tabs = albums > 0 && singles > 0;
            // Con una sola specie di uscite le schede non servono e il filtro non conta.
            let filter = if tabs { self.filter } else { Discography::All };
            self.releases = extras
                .albums
                .iter()
                .filter(|release| filter.keeps(release))
                .map(|release| Release { playlist: release.clone(), subtitle: release_subtitle(release) })
                .collect();
            ExtrasShape { popular: self.popular.len(), releases: self.releases.len(), tabs }
        });
        if extras_shape.is_none() {
            self.popular.clear();
            self.releases.clear();
        }
        self.related = self
            .library
            .feed
            .artists
            .iter()
            .filter(|card| card.artist.id != artist.id)
            .take(RELATED.min(self.columns))
            .cloned()
            .collect();
        self.credits = credits(&self.tracks);
        self.about = self.about_lines(&artist);
        self.rows = plan(Shape {
            extras: extras_shape,
            extras_failed: self.extras_failed,
            tracks: self.tracks.len(),
            related: self.related.len(),
            credits: self.credits.len(),
            columns: self.columns,
        });
        set_rows(&self.list, self.rows.len());
        remeasure_header(&self.list);
    }

    /// Le righe di *Informazioni*: brani e durata, i tuoi ascolti, le licenze.
    fn about_lines(&self, artist: &Artist) -> Vec<SharedString> {
        let total: u64 = self.tracks.iter().map(|t| t.duration_ms).sum();
        let mut lines = vec![SharedString::from(format!(
            "{} · {}",
            format::tracks(self.tracks.len()),
            format::total(total)
        ))];
        let plays: u32 = self
            .library
            .stats
            .iter()
            .filter(|stat| stat.track.artist.id == artist.id)
            .map(|stat| stat.plays)
            .sum();
        if plays > 0 {
            lines.push(format!("Li hai ascoltati {plays} volte").into());
        }
        let licences = format::licence_summary(self.tracks.iter().map(|t| t.licence.label()));
        lines.push(format!("Licenze: {licences}").into());
        lines
    }

    /// La larghezza della lista è cambiata: se cambia il numero di copertine per riga si rifà la griglia.
    fn set_columns(&mut self, columns: usize, cx: &mut Context<Self>) {
        if columns != self.columns {
            self.columns = columns;
            self.rebuild();
            cx.notify();
        }
    }

    fn set_filter(&mut self, filter: Discography, cx: &mut Context<Self>) {
        if filter != self.filter {
            self.filter = filter;
            self.rebuild();
            cx.notify();
        }
    }

    /// Riproduci: i brani più ascoltati dal catalogo ufficiale (li importa il server, uno alla volta);
    /// finché non arrivano, i brani del server.
    fn play(&self, start: usize, cx: &gpui::App) {
        let Some(id) = self.id else { return };
        let core = self.core.read(cx);
        if self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty()) {
            core.external(ExternalCmd::PlayArtistTop { artist: id, start, video: false });
        } else if !self.tracks.is_empty() {
            core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start: 0 });
        }
    }

    fn play_track(&self, start: usize, cx: &gpui::App) {
        let core = self.core.read(cx);
        match (self.tracks.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start }),
        }
    }

    fn item(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.artist.clone().map(|artist| self.header(&artist, cx).into_any_element()),
            Some(row) => self.row(row, cx),
        };
        element.unwrap_or_else(|| div().into_any_element())
    }

    fn header(&self, artist: &Artist, cx: &mut Context<Self>) -> PageHeader {
        let mut header =
            PageHeader::new(HeaderArt::Round(self.portrait.clone()), "Artista", artist.name.clone())
                .meta(self.meta.clone());
        let can_play =
            self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty()) || !self.tracks.is_empty();
        if can_play {
            header = header.action(
                Button::new("artist-play", "Riproduci")
                    .primary()
                    .icon(sprite::ICON_PLAY)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.play(0, cx))),
            );
        }
        let followed = FollowedArtist {
            id: artist.id,
            name: artist.name.clone(),
            cover_url: self.portrait.as_ref().map(ToString::to_string),
        };
        let mut follow = Button::new("artist-follow", if self.following { "Segui già" } else { "Segui" })
            .secondary()
            .selected(self.following);
        if !self.following {
            follow = follow.icon(sprite::ICON_USER_PLUS);
        }
        header = header.action(follow.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(PageEvent::ToggleFollow(Box::new(followed.clone())));
        })));
        if let Some(track) = self.radio.clone() {
            header = header.action(
                Button::new("artist-radio", "Radio").secondary().icon(sprite::ICON_RADIO).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).playback(PlaybackCmd::StartRadio(Box::new(track.clone())));
                    }),
                ),
            );
        }
        match (&self.batch, self.download_label) {
            // Mentre il lotto scarica, al posto dell'icona c'è il conteggio.
            (Some(count), Some(_)) => {
                header = header
                    .action(div().type_role(TYPE_NUMERIC).text_color(col(TEXT_PRIMARY)).child(count.clone()))
            }
            (None, Some(label)) => {
                header = header.action(
                    IconButton::new("artist-download", sprite::ICON_DOWNLOAD).tooltip(label, None).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).downloads(DownloadsCmd::DownloadAll(Arc::clone(&this.tracks)));
                        }),
                    ),
                )
            }
            _ => {}
        }
        header
    }

    fn heading(&self, section: Section, cx: &mut Context<Self>) -> AnyElement {
        let title: SharedString = match section {
            Section::Popular => "Popolari".into(),
            Section::Discography => "Discografia".into(),
            Section::Server if self.extras.is_some() => "Già nel catalogo del server".into(),
            Section::Server => "Brani".into(),
            Section::Related => "Altri artisti nel catalogo".into(),
            Section::About => "Informazioni".into(),
            Section::Credits => "Crediti".into(),
        };
        let heading = section_heading(title, SharedString::default());
        let more = self.extras.as_ref().is_some_and(|extras| extras.top.len() > POPULAR);
        if section == Section::Popular && more {
            let label = if self.more_popular { "Mostra meno" } else { "Mostra altri" };
            return heading
                .child(div().flex_1())
                .child(Button::new("artist-more", label).ghost().on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.more_popular = !this.more_popular;
                        this.rebuild();
                        cx.notify();
                    },
                )))
                .into_any_element();
        }
        heading.into_any_element()
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match *self.rows.get(ix)? {
            Row::Heading(section) => self.heading(section, cx),
            Row::Tabs => {
                let selected = Discography::ALL.iter().position(|f| *f == self.filter).unwrap_or(0);
                let tabs = Discography::ALL
                    .iter()
                    .map(|filter| {
                        Tab::new(SharedString::from(format!("disc-{}", filter.label())), filter.label())
                    })
                    .collect();
                let entity = cx.entity().downgrade();
                div()
                    .px(dp(SPACE_6))
                    .pb(dp(SPACE_5))
                    .child(Tabs::new(tabs, selected).on_select(move |ix, _, cx| {
                        let filter = Discography::ALL.get(ix).copied().unwrap_or_default();
                        entity.update(cx, |this, cx| this.set_filter(filter, cx)).ok();
                    }))
                    .into_any_element()
            }
            Row::Popular(position) => {
                let item = self.extras.as_ref()?.top.get(position)?;
                SourceRow::new(("artist-popular", position), self.popular.get(position)?.clone())
                    .busy(self.importing(item))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if plays(event) {
                            this.play(position, cx);
                        }
                    }))
                    .into_any_element()
            }
            Row::Releases(start) => self.releases_row(start, cx),
            Row::ExtrasLoading => div()
                .h(dp(TRACK_ROW_H))
                .flex()
                .items_center()
                .gap(dp(SPACE_3))
                .px(dp(SPACE_3))
                .child(div().w(dp(TRACK_INDEX_W)).flex_none())
                .child(Skeleton::new(COVER_XS, COVER_XS))
                .child(Skeleton::new(240.0, 12.0))
                .into_any_element(),
            Row::ExtrasFailed => note(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_ERROR))
                    .child("Il catalogo ufficiale non risponde: riprova più tardi."),
            )
            .into_any_element(),
            Row::Empty => note(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Il catalogo non ha brani di questo artista."),
            )
            .into_any_element(),
            Row::Track(position) => self.track_row(position, cx)?.into_any_element(),
            Row::Related => self.related_row(cx),
            Row::About => self.about_card(cx),
            Row::Credit(position) => {
                credit_line(self.credits.get(position)?.clone(), position + 1 == self.credits.len())
                    .into_any_element()
            }
        })
    }

    fn importing(&self, item: &SourceItem) -> bool {
        self.external.importing(item.source, &item.id)
    }

    /// Una riga di copertine della discografia: un clic apre l'album (la pagina della fonte).
    fn releases_row(&self, start: usize, cx: &mut Context<Self>) -> AnyElement {
        let end = (start + self.columns.max(1)).min(self.releases.len());
        let tiles = (start..end).filter_map(|position| {
            let release = self.releases.get(position)?;
            let playlist = release.playlist.clone();
            Some(
                AlbumTile::new(
                    ("artist-release", position),
                    release.playlist.title.clone(),
                    release.subtitle.clone(),
                )
                .side(TILE)
                .cover_url(release.playlist.cover_url.clone().map(SharedString::from))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(PageEvent::OpenSource(Box::new(playlist.clone())));
                })),
            )
        });
        div().flex().gap(dp(TILE_GAP)).px(dp(SPACE_6)).pb(dp(TILE_GAP)).children(tiles).into_any_element()
    }

    /// Una riga di altri artisti del catalogo: un clic apre il loro profilo.
    fn related_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let tiles = self.related.iter().enumerate().map(|(position, card)| {
            let id = card.artist.id;
            AlbumTile::new(("artist-related", position), card.artist.name.clone(), "Artista")
                .side(TILE)
                .round()
                .cover_url(card.cover_url.clone().map(SharedString::from))
                .on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Artist(id)))),
                )
        });
        div().flex().gap(dp(TILE_GAP)).px(dp(SPACE_6)).pb(dp(TILE_GAP)).children(tiles).into_any_element()
    }

    /// *Informazioni*: i dati e «Cerca altro di …», sulla tela (niente schede, come da direzione artistica).
    fn about_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let name = self.artist.as_ref().map(|artist| artist.name.clone()).unwrap_or_default();
        let lines = self.about.iter().enumerate().map(|(ix, line)| {
            div()
                .type_role(TYPE_META)
                .text_color(col(if ix == 0 { TEXT_PRIMARY } else { TEXT_SECONDARY }))
                .child(line.clone())
        });
        div()
            .px(dp(SPACE_6))
            .pb(dp(SPACE_6))
            .child(
                div().flex().flex_col().gap(dp(SPACE_3)).children(lines).child(
                    div().pt(dp(SPACE_3)).flex().child(
                        Button::new("artist-search", format!("Cerca altro di {name}"))
                            .secondary()
                            .icon(sprite::ICON_SEARCH)
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(PageEvent::Search(name.clone().into()));
                            })),
                    ),
                ),
            )
            .into_any_element()
    }

    fn track_row(&self, position: usize, cx: &mut Context<Self>) -> Option<TrackRow> {
        let track = self.tracks.get(position)?;
        let text = self.texts.get(position)?;
        let data = TrackData {
            index: u32::try_from(position + 1).unwrap_or(u32::MAX),
            title: text.title.clone(),
            artist: text.artist.clone(),
            album: text.album.clone(),
            duration: text.duration.clone(),
            liked: self.liked.contains(&track.id),
            download: download_state(&self.downloads, track),
            available: true,
            cover_url: text.cover_url.clone(),
        };
        let liked_track = track.clone();
        let download_track = track.clone();
        let menu_track = track.clone();
        let mut row = TrackRow::new(("artist-row", position), data)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if plays(event) {
                    this.play_track(position, cx);
                }
            }))
            .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).library(LibraryCmd::ToggleLike(liked_track.clone()));
            }))
            .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).downloads(download_command(&this.downloads, &download_track));
            }))
            .on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                    track: menu_track.clone(),
                    at: event.position(),
                    from_playlist: None,
                })));
            }));
        row = with_links(row, track, cx);
        if let Some((id, state)) = self.now
            && id == track.id
        {
            row = row.playing_at(state, &self.spot);
        }
        Some(row)
    }

    fn retry(&self, cx: &gpui::App) {
        if let Some(id) = self.id {
            self.core.read(cx).catalog(CatalogCmd::Retry(Resource::Artist(id)));
        }
    }
}

impl Render for ArtistPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Il posto si segna di nuovo nel paint di questo fotogramma, se la riga in corso è visibile.
        self.spot.clear();
        let page = div().relative().size_full().bg(col(SURFACE_CANVAS));
        match &self.shown {
            Shown::Loading => page.child(skeleton_page(true)),
            Shown::Failed(error) => page.child(failure(
                "artist-retry",
                error.clone(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.retry(cx)),
            )),
            Shown::Ready => {
                // Quante copertine stanno in una riga lo dice la larghezza vera, letta a ogni disegno:
                // se cambia, la griglia si rifà al giro dopo (un solo fotogramma).
                let entity = cx.entity().downgrade();
                let current = self.columns;
                let measure = canvas(
                    move |bounds, _, cx| {
                        let fit = columns(f32::from(bounds.size.width));
                        if fit != current {
                            cx.defer(move |cx| {
                                entity.update(cx, |this, cx| this.set_columns(fit, cx)).ok();
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();
                page.child(measure).child(
                    list(
                        self.list.clone(),
                        cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
                    )
                    .size_full(),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Shape {
        Shape { columns: 4, ..Shape::default() }
    }

    #[test]
    fn the_sections_follow_the_iced_order() {
        let rows = plan(Shape {
            extras: Some(ExtrasShape { popular: 2, releases: 5, tabs: true }),
            tracks: 1,
            related: 3,
            credits: 1,
            ..shape()
        });
        assert_eq!(
            rows,
            vec![
                Row::Heading(Section::Popular),
                Row::Popular(0),
                Row::Popular(1),
                Row::Heading(Section::Discography),
                Row::Tabs,
                Row::Releases(0),
                Row::Releases(4),
                Row::Heading(Section::Server),
                Row::Track(0),
                Row::Heading(Section::Related),
                Row::Related,
                Row::Heading(Section::About),
                Row::About,
                Row::Heading(Section::Credits),
                Row::Credit(0),
            ]
        );
    }

    #[test]
    fn while_the_official_catalogue_loads_there_are_placeholder_rows() {
        let rows = plan(Shape { tracks: 1, ..shape() });
        assert_eq!(rows.iter().filter(|row| **row == Row::ExtrasLoading).count(), EXTRAS_SKELETON);
        assert!(rows.contains(&Row::Track(0)));
    }

    #[test]
    fn a_failure_shows_only_when_there_is_nothing_else() {
        assert_eq!(plan(Shape { extras_failed: true, ..shape() }), vec![Row::ExtrasFailed]);
        let rows = plan(Shape { extras_failed: true, tracks: 1, ..shape() });
        assert!(!rows.contains(&Row::ExtrasFailed));
    }

    #[test]
    fn an_artist_with_nothing_says_so() {
        let rows = plan(Shape { extras: Some(ExtrasShape::default()), ..shape() });
        assert_eq!(rows, vec![Row::Empty]);
    }

    #[test]
    fn the_grid_fits_whole_tiles_in_the_width() {
        assert_eq!(columns(2.0 * SPACE_6 + TILE), 1);
        assert_eq!(columns(2.0 * SPACE_6 + 4.0 * TILE + 3.0 * TILE_GAP), 4);
        assert_eq!(columns(2.0 * SPACE_6 + 4.0 * TILE + 3.0 * TILE_GAP - 1.0), 3);
        assert_eq!(columns(10.0), 1);
    }

    #[test]
    fn the_filter_splits_albums_from_singles_and_eps() {
        let release = |kind: &str| SourcePlaylist {
            source: psytify_core::api::SourceKind::Deezer,
            id: "1".into(),
            title: "X".into(),
            owner: "Y".into(),
            cover_url: None,
            kind: kind.into(),
            released_on: Some("2021-03-04".into()),
        };
        assert!(Discography::Albums.keeps(&release("album")));
        assert!(!Discography::Albums.keeps(&release("ep")));
        assert!(Discography::Singles.keeps(&release("single")));
        assert!(Discography::All.keeps(&release("ep")));
        assert_eq!(release_subtitle(&release("album")), SharedString::from("2021 · Album"));
    }
}
