//! Il menu del clic destro. Si apre dove sta il mouse, resta dentro la finestra e sale di qualche
//! pixel mentre compare; un clic fuori lo chiude. "Aggiungi a una playlist" e "Scarica in…" aprono un
//! secondo elenco accanto al primo.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, Point};
use uuid::Uuid;

use super::icons::{Icon, icon};
use super::theme;
use super::{components::divider_line, motion};
use crate::api::{DownloadQuality, Track};
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

/// Il livello del menu, a tutta finestra; `None` se non ce n'è uno aperto.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let menu = app.menu.as_ref()?;
    let items = entries(app, menu);
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
                (playlists_list(app, track), rows, index)
            })
        } else if menu.downloads {
            items
                .downloads_at
                .map(|index| (downloads_list(app, track), DownloadQuality::ALL.len() as f32, index))
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
    list: iced::widget::Column<'a, Message>,
    rows: usize,
    separators: usize,
    playlists_at: Option<usize>,
    downloads_at: Option<usize>,
}

impl<'a> Builder<'a> {
    fn new() -> Self {
        Self { list: column![], rows: 0, separators: 0, playlists_at: None, downloads_at: None }
    }

    fn item(mut self, glyph: Icon, label: &str, message: Message) -> Self {
        self.list = self.list.push(item(glyph, label, Some(Message::Chosen(Box::new(message))), false, None));
        self.rows += 1;
        self
    }

    fn danger(mut self, glyph: Icon, label: &str, message: Message) -> Self {
        self.list = self.list.push(item(glyph, label, Some(Message::Chosen(Box::new(message))), true, None));
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

fn entries<'a>(app: &'a App, menu: &ContextMenu) -> Entries<'a> {
    match &menu.target {
        MenuTarget::Track { track, source } => track_entries(app, menu, track, *source),
        MenuTarget::Album(album) => {
            let saved = app.has_saved(album.id);
            let artist = FollowedArtist {
                id: album.artist.id,
                name: album.artist.name.clone(),
                cover_url: album.cover_url.clone(),
            };
            Builder::new()
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
        MenuTarget::Artist(artist) => Builder::new()
            .item(Icon::Account, "Apri l'artista", Message::Navigate(Screen::Artist(artist.id)))
            .item(
                follow_icon(app, artist.id),
                follow_label(app, artist.id),
                Message::ToggleFollow((**artist).clone()),
            )
            .finish(),
        MenuTarget::Player => player_entries(app),
        MenuTarget::Playlist(id) => {
            let name = app.playlist(*id).map(|p| p.name.clone()).unwrap_or_default();
            let mut builder = Builder::new();
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

fn track_entries<'a>(app: &'a App, menu: &ContextMenu, track: &Track, source: TrackSource) -> Entries<'a> {
    let liked = app.is_liked(track.id);
    let mut builder = Builder::new()
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
fn player_entries(app: &App) -> Entries<'_> {
    let mut builder = Builder::new()
        .item(Icon::Expand, "Vista immersiva", Message::ToggleFocus)
        .item(Icon::Equalizer, "Equalizzatore", Message::OpenSettingsTab(SettingsTab::Sound))
        .separator();
    // Questa voce non chiude il menu: accende o spegne e mostra la spunta.
    builder.list = builder.list.push(item(
        Icon::Radio,
        "Autoplay",
        Some(Message::SetAutoplay(!app.autoplay)),
        false,
        app.autoplay.then_some(Icon::Check),
    ));
    builder.rows += 1;
    builder = builder.separator();
    if let Some(timer) = app.sleep {
        let label = format!("Si ferma tra {}", timer.remaining_label(std::time::Instant::now()));
        builder.list = builder.list.push(item(Icon::Moon, &label, None, false, Some(Icon::Check)));
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
fn playlists_list<'a>(app: &'a App, track: &Track) -> Element<'a, Message> {
    let new = item(
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
            Icon::List,
            &playlist.name,
            Some(Message::Chosen(Box::new(Message::AddToPlaylist(playlist.id, track.clone())))),
            false,
            has.then_some(Icon::Check),
        )
    });
    let list = column(existing);
    column![new, scrollable(list).height(Length::Shrink).style(theme::scroll)]
        .max_width(theme::MENU_WIDTH)
        .into()
}

/// L'elenco accanto al menu con le qualità di download, con la spunta su quella delle impostazioni.
fn downloads_list<'a>(app: &'a App, track: &Track) -> Element<'a, Message> {
    let rows = DownloadQuality::ALL.map(|quality| {
        item(
            Icon::Download,
            &format!("{} · {}", quality.label(), quality.detail()),
            Some(Message::Chosen(Box::new(Message::DownloadAs(track.clone(), quality)))),
            false,
            (quality == app.download_quality).then_some(Icon::Check),
        )
    });
    column(rows).max_width(theme::MENU_WIDTH).into()
}

/// Una voce: icona, etichetta e, a destra, un segno facoltativo (freccia o spunta).
fn item<'a>(
    glyph: Icon,
    label: &str,
    message: Option<Message>,
    danger: bool,
    trailing: Option<Icon>,
) -> Element<'a, Message> {
    let tint = if danger { theme::NEGATIVE } else { theme::MUTED };
    let mut content = row![
        icon(glyph, 16.0, tint),
        text(super::components::fit(
            label,
            theme::MENU_WIDTH - 2.0 * PADDING - 20.0 - 16.0 - 12.0 - 30.0,
            theme::FS_BODY
        ))
        .size(theme::FS_BODY)
        .font(theme::FONT_MEDIUM),
        Space::with_width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(trailing) = trailing {
        content = content.push(icon(trailing, 14.0, theme::MUTED));
    }
    button(super::components::centered(content))
        .width(Length::Fill)
        .height(theme::MENU_ITEM)
        .padding([0, 10])
        .style(theme::menu_item(danger))
        .on_press_maybe(message)
        .into()
}
