//! Il menu del clic destro (sul telefono, una sheet dal basso).  Si apre dove sta il mouse, resta dentro la finestra e sale di qualche
//! pixel mentre compare; un clic fuori lo chiude. "Aggiungi a una playlist" e "Scarica in…" aprono un
//! secondo elenco accanto al primo.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, Point};
use uuid::Uuid;

use super::cover::{Art, cover, cover_round};
use super::icons::{Icon, icon};
use super::theme;
use super::{
    components::{art_of, divider_line},
    motion,
};
use crate::api::{DownloadQuality, Track};
use crate::app::is_phone;
use crate::app::{
    App, ContextMenu, Dialog, MenuTarget, Message, Screen, SettingsTab, SleepChoice, TrackSource,
};
use crate::config::FollowedArtist;

/// Distanza minima tra il menu e il bordo della finestra.
const EDGE: f32 = 8.0;
/// Quanto sale il menu mentre compare.
const RISE: f32 = 6.0;
/// Altezza massima dell'elenco delle playlist prima che scorra.
const FLYOUT_MAX: f32 = 8.0 * theme::MENU_ITEM;
const PADDING: f32 = 6.0;

/// Le misure di una voce: quelle del desktop o quelle, più grandi, della sheet del telefono.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    icon: f32,
    text: f32,
    height: f32,
    width: f32,
    gap: f32,
    pad: f32,
}

impl Metrics {
    const DESKTOP: Self = Self {
        icon: 16.0,
        text: theme::FS_BODY,
        height: theme::MENU_ITEM,
        width: theme::MENU_WIDTH,
        gap: 12.0,
        pad: 10.0,
    };

    /// La sheet occupa tutta la larghezza della finestra.
    fn sheet(width: f32) -> Self {
        Self { icon: 22.0, text: theme::FS_SHEET, height: theme::SHEET_ITEM, width, gap: 18.0, pad: 20.0 }
    }

    /// Larghezza che resta al testo: tolti i margini, l'icona e il segno a destra.
    fn label_width(self) -> f32 {
        self.width - 2.0 * self.pad - self.icon - self.gap - 40.0
    }
}

/// Il livello del menu, a tutta finestra; `None` se non ce n'è uno aperto.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let menu = app.menu.as_ref()?;
    if is_phone() {
        return Some(sheet(app, menu));
    }
    let items = entries(app, menu, Metrics::DESKTOP);
    let height = items.height();
    let shown = motion::appear(menu.since, theme::MOTION_FAST);

    // Le barre del sistema (telefono) non sono spazio per il menu.
    let (top, bottom) = app.insets.map_or((0.0, 0.0), |edges| (edges.top, edges.bottom));
    let origin = place(menu.at, theme::MENU_WIDTH, height, app.window_size, top, bottom);
    let mut layer = stack![
        mouse_area(Space::new(Length::Fill, Length::Fill))
            .on_press(Message::CloseMenu)
            .on_right_press(Message::CloseMenu),
        positioned(opaque(panel(items.body, shown)), Point::new(origin.x, origin.y + RISE * (1.0 - shown))),
    ];
    if let MenuTarget::Track { track, .. } = &menu.target {
        // Un solo elenco accanto al menu alla volta: playlist oppure qualità di download.
        let flyout = if menu.playlists {
            items.playlists_at.map(|index| {
                let rows = (app.playlists.len() as f32 + 1.0).min(FLYOUT_MAX / theme::MENU_ITEM);
                (playlists_list(app, track, Metrics::DESKTOP), rows, index)
            })
        } else if menu.downloads {
            items.downloads_at.map(|index| {
                (downloads_list(app, track, Metrics::DESKTOP), DownloadQuality::ALL.len() as f32, index)
            })
        } else {
            None
        };
        if let Some((list, rows, index)) = flyout {
            let list_height = rows * theme::MENU_ITEM + 2.0 * PADDING;
            // Accanto al menu, a destra o a sinistra; se non ci sta da nessuna parte (telefono), sopra.
            let side = if origin.x + 2.0 * theme::MENU_WIDTH + EDGE < app.window_size.width {
                origin.x + theme::MENU_WIDTH + 4.0
            } else if origin.x - theme::MENU_WIDTH - 4.0 >= EDGE {
                origin.x - theme::MENU_WIDTH - 4.0
            } else {
                origin.x + 2.0 * EDGE
            };
            let top = (origin.y + PADDING + index as f32 * theme::MENU_ITEM)
                .min(app.window_size.height - bottom - list_height - EDGE)
                .max(top + EDGE);
            layer = layer
                .push(positioned(opaque(panel(list, shown)), Point::new(side, top + RISE * (1.0 - shown))));
        }
    }
    Some(layer.into())
}

/// Dove mettere un menu largo `width` e alto `height` aperto con il mouse in `at`, dentro la finestra
/// e fuori dalle barre del sistema (`top` e `bottom`, zero sul desktop).
fn place(at: Point, width: f32, height: f32, window: iced::Size, top: f32, bottom: f32) -> Point {
    Point::new(
        at.x.min(window.width - width - EDGE).max(EDGE),
        at.y.min(window.height - bottom - height - EDGE).max(top + EDGE),
    )
}

fn positioned<'a>(content: Element<'a, Message>, at: Point) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding { top: at.y, left: at.x, ..Padding::ZERO })
        .into()
}

fn panel<'a>(content: Element<'a, Message>, shown: f32) -> Element<'a, Message> {
    container(content).width(theme::MENU_WIDTH).padding(PADDING).style(theme::menu_box(shown)).into()
}

/// Le voci del menu e la loro altezza, per posizionarlo.
struct Entries<'a> {
    body: Element<'a, Message>,
    /// Quante voci piene ha (i separatori contano meno).
    rows: usize,
    separators: usize,
    /// Posizione (in voci) di "Aggiungi a una playlist", per allineare l'elenco accanto.
    playlists_at: Option<usize>,
    /// Posizione (in voci) di "Scarica in…", per allineare l'elenco delle qualità.
    downloads_at: Option<usize>,
}

impl Entries<'_> {
    fn height(&self) -> f32 {
        self.rows as f32 * theme::MENU_ITEM + self.separators as f32 * SEPARATOR + 2.0 * PADDING
    }
}

const SEPARATOR: f32 = 9.0;

/// Costruisce le voci in ordine, tenendo il conto di quante sono.
struct Builder<'a> {
    m: Metrics,
    list: iced::widget::Column<'a, Message>,
    rows: usize,
    separators: usize,
    playlists_at: Option<usize>,
    downloads_at: Option<usize>,
}

impl<'a> Builder<'a> {
    fn new(m: Metrics) -> Self {
        Self { m, list: column![], rows: 0, separators: 0, playlists_at: None, downloads_at: None }
    }

    fn item(mut self, glyph: Icon, label: &str, message: Message) -> Self {
        self.list =
            self.list.push(item(self.m, glyph, label, Some(Message::Chosen(Box::new(message))), false, None));
        self.rows += 1;
        self
    }

    fn danger(mut self, glyph: Icon, label: &str, message: Message) -> Self {
        self.list =
            self.list.push(item(self.m, glyph, label, Some(Message::Chosen(Box::new(message))), true, None));
        self.rows += 1;
        self
    }

    fn separator(mut self) -> Self {
        self.list = self.list.push(divider_line());
        self.separators += 1;
        self
    }

    fn finish(self) -> Entries<'a> {
        Entries {
            body: self.list.into(),
            rows: self.rows,
            separators: self.separators,
            playlists_at: self.playlists_at,
            downloads_at: self.downloads_at,
        }
    }
}

fn entries<'a>(app: &'a App, menu: &ContextMenu, m: Metrics) -> Entries<'a> {
    match &menu.target {
        MenuTarget::Track { track, source } => track_entries(app, menu, track, *source, m),
        MenuTarget::Album(album) => {
            let saved = app.has_saved(album.id);
            let artist = FollowedArtist {
                id: album.artist.id,
                name: album.artist.name.clone(),
                cover_url: album.cover_url.clone(),
            };
            Builder::new(m)
                .item(Icon::Play, "Riproduci", Message::PlayAlbum(album.id))
                .item(Icon::Library, "Apri l'album", Message::Navigate(Screen::Album(album.id)))
                .separator()
                .item(
                    if saved { Icon::Check } else { Icon::Plus },
                    if saved { "Togli dalla libreria" } else { "Salva nella libreria" },
                    Message::ToggleSaveAlbum((**album).clone()),
                )
                .item(Icon::Account, "Vai all'artista", Message::Navigate(Screen::Artist(album.artist.id)))
                .item(
                    follow_icon(app, artist.id),
                    follow_label(app, artist.id),
                    Message::ToggleFollow(artist),
                )
                .finish()
        }
        MenuTarget::Artist(artist) => Builder::new(m)
            .item(Icon::Account, "Apri l'artista", Message::Navigate(Screen::Artist(artist.id)))
            .item(
                follow_icon(app, artist.id),
                follow_label(app, artist.id),
                Message::ToggleFollow((**artist).clone()),
            )
            .finish(),
        MenuTarget::Player => player_entries(app, menu, m),
        MenuTarget::Playlist(id) => {
            let name = app.playlist(*id).map(|p| p.name.clone()).unwrap_or_default();
            let mut builder = Builder::new(m);
            if let Some(playlist) = app.playlist(*id).filter(|p| !p.tracks.is_empty()) {
                let tracks: Arc<[Track]> = Arc::from(playlist.tracks.as_slice());
                builder = builder
                    .item(Icon::Play, "Riproduci", Message::PlayTracks(tracks.clone(), 0))
                    .item(Icon::Shuffle, "Riproduci in ordine casuale", Message::ShufflePlay(tracks))
                    .separator();
            }
            builder
                .item(Icon::Edit, "Rinomina", Message::OpenDialog(Dialog::Rename { id: *id, name }))
                .danger(Icon::Trash, "Elimina", Message::OpenDialog(Dialog::ConfirmDelete(*id)))
                .finish()
        }
    }
}

fn track_entries<'a>(
    app: &'a App,
    menu: &ContextMenu,
    track: &Track,
    source: TrackSource,
    m: Metrics,
) -> Entries<'a> {
    let liked = app.is_liked(track.id);
    let mut builder = Builder::new(m)
        .item(Icon::Play, "Riproduci", Message::PlayTracks(Arc::from([track.clone()]), 0))
        .item(Icon::Next, "Riproduci subito dopo", Message::EnqueueNext(track.clone()))
        .item(Icon::Queue, "Aggiungi alla coda", Message::Enqueue(track.clone()))
        .item(Icon::Radio, "Avvia radio", Message::StartRadio(Box::new(track.clone())))
        .separator()
        .item(
            if liked { Icon::HeartFill } else { Icon::Heart },
            if liked { "Non mi piace più" } else { "Mi piace" },
            Message::ToggleLike(track.clone()),
        );
    // Questa voce non chiude il menu: apre l'elenco delle playlist accanto.
    builder.playlists_at = Some(builder.rows);
    builder.list = builder.list.push(item(
        m,
        Icon::List,
        "Aggiungi a una playlist",
        Some(Message::MenuPlaylists(!menu.playlists)),
        false,
        Some(Icon::ChevronRight),
    ));
    builder.rows += 1;
    builder = match source {
        TrackSource::Playlist(id, index) => {
            builder.danger(Icon::Trash, "Togli da questa playlist", Message::RemoveFromPlaylist(id, index))
        }
        TrackSource::Queue(position) => {
            builder.danger(Icon::Close, "Togli dalla coda", Message::RemoveFromQueue(position))
        }
        TrackSource::Catalogue | TrackSource::Liked => builder,
    };
    builder = builder.separator().item(
        Icon::Account,
        "Vai all'artista",
        Message::Navigate(Screen::Artist(track.artist.id)),
    );
    if let Some(album) = &track.album {
        builder = builder.item(Icon::Library, "Vai all'album", Message::Navigate(Screen::Album(album.id)));
    }
    let artist = FollowedArtist {
        id: track.artist.id,
        name: track.artist.name.clone(),
        cover_url: track.cover_url.clone(),
    };
    builder = builder
        .item(follow_icon(app, artist.id), follow_label(app, artist.id), Message::ToggleFollow(artist))
        .item(
            Icon::Copy,
            "Copia titolo e artista",
            Message::CopyText(format!("{} — {}", track.title, track.artist.name)),
        );
    if track.downloadable && !app.downloading.contains(&track.id) {
        builder = builder.item(
            Icon::Download,
            &format!("Scarica · {}", app.download_quality.label()),
            Message::Download(track.clone()),
        );
        // Questa voce non chiude il menu: apre l'elenco delle qualità accanto.
        builder.downloads_at = Some(builder.rows);
        builder.list = builder.list.push(item(
            m,
            Icon::Download,
            "Scarica in…",
            Some(Message::MenuDownloads(!menu.downloads)),
            false,
            Some(Icon::ChevronRight),
        ));
        builder.rows += 1;
    }
    builder.finish()
}

/// Il menu del player: vista immersiva, equalizzatore, riproduzione automatica e timer di spegnimento.
fn player_entries<'a>(app: &'a App, menu: &ContextMenu, m: Metrics) -> Entries<'a> {
    let mut builder = Builder::new(m)
        .item(Icon::Expand, "Vista immersiva", Message::ToggleFocus)
        .item(Icon::Equalizer, "Equalizzatore", Message::OpenSettingsTab(SettingsTab::Sound))
        .separator();
    // Questa voce non chiude il menu: accende o spegne e mostra la spunta.
    builder.list = builder.list.push(item(
        m,
        Icon::Radio,
        "Autoplay",
        Some(Message::SetAutoplay(!app.autoplay)),
        false,
        app.autoplay.then_some(Icon::Check),
    ));
    builder.rows += 1;
    builder = builder.separator();
    if is_phone() {
        // Sul telefono i timer stanno in un elenco a parte: una sola riga che dice cosa è attivo.
        let label = match app.sleep {
            Some(timer) => format!("Si ferma tra {}", timer.remaining_label(std::time::Instant::now())),
            None => "Timer di spegnimento".to_string(),
        };
        builder.list = builder.list.push(item(
            m,
            Icon::Moon,
            &label,
            Some(Message::MenuTimers(!menu.timers)),
            false,
            Some(Icon::ChevronRight),
        ));
        builder.rows += 1;
        return builder.finish();
    }
    if let Some(timer) = app.sleep {
        let label = format!("Si ferma tra {}", timer.remaining_label(std::time::Instant::now()));
        builder.list = builder.list.push(item(m, Icon::Moon, &label, None, false, Some(Icon::Check)));
        builder.rows += 1;
    }
    for choice in SleepChoice::ALL {
        let label = match choice {
            SleepChoice::EndOfTrack => "Timer: fine del brano".to_string(),
            other => format!("Timer: {}", other.label()),
        };
        builder = builder.item(Icon::Moon, &label, Message::SetSleep(Some(choice)));
    }
    if app.sleep.is_some() {
        builder = builder.separator().danger(Icon::Close, "Disattiva il timer", Message::SetSleep(None));
    }
    builder.finish()
}

fn follow_icon(app: &App, artist: Uuid) -> Icon {
    if app.is_following(artist) { Icon::Check } else { Icon::UserPlus }
}

fn follow_label(app: &App, artist: Uuid) -> &'static str {
    if app.is_following(artist) { "Smetti di seguire" } else { "Segui l'artista" }
}

/// L'elenco accanto al menu: «Nuova playlist» e le playlist esistenti, con un segno su quelle che
/// hanno già il brano.
fn playlists_list<'a>(app: &'a App, track: &Track, m: Metrics) -> Element<'a, Message> {
    let new = item(
        m,
        Icon::Plus,
        "Nuova playlist",
        Some(Message::OpenDialog(Dialog::NewPlaylist {
            name: app.suggested_playlist_name(),
            seed: vec![track.clone()],
        })),
        false,
        None,
    );
    let existing = app.playlists.iter().map(|playlist| {
        let has = playlist.contains(track.id);
        item(
            m,
            Icon::List,
            &playlist.name,
            Some(Message::Chosen(Box::new(Message::AddToPlaylist(playlist.id, track.clone())))),
            false,
            has.then_some(Icon::Check),
        )
    });
    let list = column(existing);
    column![new, scrollable(list).height(Length::Shrink).style(theme::scroll)].max_width(m.width).into()
}

/// L'elenco accanto al menu con le qualità di download, con la spunta su quella delle impostazioni.
fn downloads_list<'a>(app: &'a App, track: &Track, m: Metrics) -> Element<'a, Message> {
    let rows = DownloadQuality::ALL.map(|quality| {
        item(
            m,
            Icon::Download,
            &format!("{} · {}", quality.label(), quality.detail()),
            Some(Message::Chosen(Box::new(Message::DownloadAs(track.clone(), quality)))),
            false,
            (quality == app.download_quality).then_some(Icon::Check),
        )
    });
    column(rows).max_width(m.width).into()
}

/// Una voce: icona, etichetta e, a destra, un segno facoltativo (freccia o spunta).
fn item<'a>(
    m: Metrics,
    glyph: Icon,
    label: &str,
    message: Option<Message>,
    danger: bool,
    trailing: Option<Icon>,
) -> Element<'a, Message> {
    let tint = if danger { theme::NEGATIVE } else { theme::MUTED };
    let mut content = row![
        icon(glyph, m.icon, tint),
        text(super::components::fit(label, m.label_width(), m.text)).size(m.text).font(theme::FONT_MEDIUM),
        Space::with_width(Length::Fill),
    ]
    .spacing(m.gap)
    .align_y(Alignment::Center);
    if let Some(trailing) = trailing {
        content = content.push(icon(trailing, m.icon.clamp(14.0, 16.0), theme::MUTED));
    }
    button(super::components::centered(content))
        .width(Length::Fill)
        .height(m.height)
        .padding([0.0, m.pad])
        .style(theme::menu_item(danger))
        .on_press_maybe(message)
        .into()
}

// ---------- Sheet del telefono ----------

/// Altezza massima della sheet, in parti della finestra: sopra resta il velo da toccare per chiudere.
const SHEET_MAX: f32 = 0.78;
const HEADER_COVER: f32 = 52.0;

/// La sheet dal basso: maniglia, intestazione del bersaglio (copertina e nome) e le voci; «Aggiungi a
/// una playlist», «Scarica in…» e i timer sostituiscono le voci con il loro elenco e un tasto indietro.
fn sheet<'a>(app: &'a App, menu: &'a ContextMenu) -> Element<'a, Message> {
    let shown = motion::appear(menu.since, theme::MOTION_FAST);
    let (top, bottom) = app.insets.map_or((0.0, 0.0), |edges| (edges.top, edges.bottom));
    let m = Metrics::sheet(app.window_size.width);

    let body: Element<'_, Message> = match &menu.target {
        MenuTarget::Track { track, .. } if menu.playlists => {
            sub(m, "Aggiungi a una playlist", playlists_list(app, track, m), Message::MenuPlaylists)
        }
        MenuTarget::Track { track, .. } if menu.downloads => {
            sub(m, "Scarica in…", downloads_list(app, track, m), Message::MenuDownloads)
        }
        MenuTarget::Player if menu.timers => {
            sub(m, "Timer di spegnimento", timers_list(app, m), Message::MenuTimers)
        }
        _ => entries(app, menu, m).body,
    };
    let max_height = (app.window_size.height - top - bottom) * SHEET_MAX;
    let panel = container(
        container(
            column![
                container(Space::new(36.0, 4.0)).style(theme::bar(theme::FAINT, 2.0)),
                header(app, menu, m),
                scrollable(body).height(Length::Shrink).style(theme::scroll),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .max_height(max_height),
    )
    .width(Length::Fill)
    .padding(Padding { top: 10.0, bottom: bottom + 10.0, left: 0.0, right: 0.0 })
    .style(theme::sheet(shown));

    stack![
        mouse_area(container(Space::new(Length::Fill, Length::Fill)).style(theme::scrim(shown)))
            .on_press(Message::CloseMenu)
            .on_right_press(Message::CloseMenu),
        column![Space::with_height(Length::Fill), opaque(panel)],
    ]
    .into()
}

/// Copertina, nome e riga di dettaglio di ciò su cui si è aperto il menu.
fn header<'a>(app: &'a App, menu: &'a ContextMenu, m: Metrics) -> Element<'a, Message> {
    let (art, round, title, detail): (Option<Art<'a>>, bool, String, String) = match &menu.target {
        MenuTarget::Track { track, .. } => {
            (Some(art_of(track)), false, track.title.clone(), track.artist.name.clone())
        }
        MenuTarget::Album(album) => (
            Some(Art { id: album.id, title: &album.title, url: album.cover_url.as_deref() }),
            false,
            album.title.clone(),
            album.artist.name.clone(),
        ),
        MenuTarget::Artist(artist) => (
            Some(Art { id: artist.id, title: &artist.name, url: artist.cover_url.as_deref() }),
            true,
            artist.name.clone(),
            "Artista".to_string(),
        ),
        MenuTarget::Playlist(id) => match app.playlist(*id) {
            Some(playlist) => (
                Some(Art::drawn(playlist.id, &playlist.name)),
                false,
                playlist.name.clone(),
                format!("Playlist · {} brani", playlist.tracks.len()),
            ),
            None => (None, false, String::new(), String::new()),
        },
        MenuTarget::Player => match app.queue.current() {
            Some(track) => (Some(art_of(track)), false, track.title.clone(), track.artist.name.clone()),
            None => (None, false, "In riproduzione".to_string(), String::new()),
        },
    };
    let text_width = m.width - 2.0 * m.pad - HEADER_COVER - 14.0;
    let mut content = row![].spacing(14).align_y(Alignment::Center);
    if let Some(art) = art {
        content = content.push(if round {
            cover_round(&app.covers, art, HEADER_COVER)
        } else {
            cover(&app.covers, art, HEADER_COVER)
        });
    }
    content = content.push(
        column![
            text(super::components::fit(&title, text_width, theme::FS_SHEET))
                .size(theme::FS_SHEET)
                .font(theme::FONT_SEMIBOLD),
            text(super::components::fit(&detail, text_width, theme::FS_BODY))
                .size(theme::FS_BODY)
                .color(theme::MUTED),
        ]
        .spacing(2),
    );
    column![container(content).padding([4.0, m.pad]).width(Length::Fill), divider_line()].spacing(10).into()
}

/// Un sotto-elenco con la riga per tornare indietro in cima.
fn sub<'a>(
    m: Metrics,
    title: &str,
    list: Element<'a, Message>,
    flag: fn(bool) -> Message,
) -> Element<'a, Message> {
    column![back_row(m, title, flag(false)), list].into()
}

/// La riga in cima a un sotto-elenco: freccia e titolo, tocca per tornare alle voci.
fn back_row<'a>(m: Metrics, title: &str, back: Message) -> Element<'a, Message> {
    item(m, Icon::ChevronLeft, title, Some(back), false, None)
}

/// I timer di spegnimento della sheet del player, con la spunta su quello attivo.
fn timers_list<'a>(app: &'a App, m: Metrics) -> Element<'a, Message> {
    let mut list = column![];
    for choice in SleepChoice::ALL {
        let label = match choice {
            SleepChoice::EndOfTrack => "Fine del brano".to_string(),
            other => other.label(),
        };
        list = list.push(item(
            m,
            Icon::Moon,
            &label,
            Some(Message::Chosen(Box::new(Message::SetSleep(Some(choice))))),
            false,
            None,
        ));
    }
    if let Some(timer) = app.sleep {
        let label = format!("Disattiva · si ferma tra {}", timer.remaining_label(std::time::Instant::now()));
        list = list.push(item(
            m,
            Icon::Close,
            &label,
            Some(Message::Chosen(Box::new(Message::SetSleep(None)))),
            true,
            None,
        ));
    }
    list.max_width(m.width).into()
}
