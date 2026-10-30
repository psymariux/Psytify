//! Componenti condivisi: testi, pulsanti, righe di brani, tessere, scheletri e stati vuoti.

use std::sync::Arc;
use std::time::Duration;

use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, stack, text, tooltip};
use iced::{Alignment, Color, Element, Font, Length, Padding, alignment};
use uuid::Uuid;

use super::cover::{Art, cover, cover_round};
use super::icons::{Icon, icon};
use super::motion::{self, fade};
use super::theme;
use crate::api::{AlbumSummary, ArtistSummary, Track};
use crate::app::{App, Hovered, Load, MenuTarget, Message, Screen, TrackSource};
use crate::error::AppError;
use crate::player::seek::format_time;

// ---------- Testi ----------

pub fn h1<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(theme::FS_PAGE).font(theme::FONT_SEMIBOLD).color(theme::TEXT)
}

pub fn h2<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(theme::FS_SECTION).font(theme::FONT_SEMIBOLD).color(theme::TEXT)
}

pub fn body<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(theme::FS_BODY).color(theme::TEXT)
}

pub fn muted<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(theme::FS_BODY).color(theme::MUTED)
}

pub fn small<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(theme::FS_SMALL).color(theme::FAINT)
}

/// Margine sulla larghezza misurata: il testo non deve mai andare a capo dentro `line`.
const FIT_SAFETY: f32 = 1.04;
/// Larghezza di "…" in em.
const ELLIPSIS_EM: f32 = 0.8;

/// Taglia `content` con "…" perché stia in `width` pixel al corpo `size`, misurando le lettere
/// con le larghezze vere di Inter (`lyrics::char_em`).
pub fn fit(content: &str, width: f32, size: f32) -> String {
    let budget = width.max(0.0) / (size * FIT_SAFETY);
    if super::lyrics::em_width(content) <= budget {
        return content.to_string();
    }
    let room = budget - ELLIPSIS_EM;
    let mut used = 0.0;
    let mut end = 0;
    for (at, letter) in content.char_indices() {
        used += super::lyrics::char_em(letter);
        if used > room {
            break;
        }
        end = at + letter.len_utf8();
    }
    let mut cut = content[..end].trim_end().to_string();
    cut.push('…');
    cut
}

/// Stile di una riga di testo.
#[derive(Debug, Clone, Copy)]
pub struct Line {
    pub size: f32,
    pub font: Font,
    /// `None` eredita il colore dal contenitore (per esempio dallo stato di un pulsante).
    pub color: Option<Color>,
}

pub const TITLE_LINE: Line =
    Line { size: theme::FS_BODY, font: theme::FONT_MEDIUM, color: Some(theme::TEXT) };
pub const PLAYING_LINE: Line =
    Line { size: theme::FS_BODY, font: theme::FONT_MEDIUM, color: Some(theme::ACCENT) };
pub const BODY_LINE: Line = Line { size: theme::FS_BODY, font: theme::FONT, color: Some(theme::TEXT) };
pub const MUTED_LINE: Line = Line { size: theme::FS_BODY, font: theme::FONT, color: Some(theme::MUTED) };
pub const SUB_LINE: Line = Line { size: theme::FS_SMALL, font: theme::FONT, color: Some(theme::MUTED) };
pub const META_LINE: Line = Line { size: theme::FS_SMALL, font: theme::FONT, color: Some(theme::FAINT) };
pub const H1_LINE: Line =
    Line { size: theme::FS_TITLE, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };
pub const H2_LINE: Line =
    Line { size: theme::FS_SECTION, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };
pub const NAV_LINE: Line = Line { size: theme::FS_BODY, font: theme::FONT, color: None };

/// Un testo che va a capo su al massimo `lines` righe, tagliato con "…" se non ci sta: per i
/// titoli grandi, che su una riga sola diventerebbero «Blindin…».
pub fn wrapped<'a>(content: &str, width: f32, style: Line, lines: u16) -> Element<'a, Message> {
    let lines = f32::from(lines.max(1));
    // Le righe a capo sprecano la fine della riga: un po' meno spazio della somma delle righe.
    let label = text(fit(content, width * lines * 0.88, style.size)).size(style.size).font(style.font);
    let label = match style.color {
        Some(color) => label.color(color),
        None => label,
    };
    container(label)
        .width(Length::Fill)
        .max_width(width.max(1.0))
        .max_height((style.size * 1.3 * lines).ceil())
        .clip(true)
        .into()
}

/// Una riga di testo che non va mai a capo.
///
/// Iced 0.13 ignora `Wrapping::None` (iced_graphics 0.13.0 non lo passa a cosmic-text), quindi
/// tagliamo noi con "…" sulla larghezza stimata e fissiamo l'altezza a una riga: se la stima
/// sbaglia, l'eventuale seconda riga resta nascosta dal clip.
pub fn line<'a>(content: &str, width: f32, style: Line) -> Element<'a, Message> {
    line_aligned(content, width, style, alignment::Horizontal::Left)
}

/// Come `line`, con il testo allineato: al centro sotto le copertine tonde.
pub fn line_aligned<'a>(
    content: &str,
    width: f32,
    style: Line,
    align: alignment::Horizontal,
) -> Element<'a, Message> {
    let label = text(fit(content, width, style.size))
        .size(style.size)
        .font(style.font)
        .width(Length::Fill)
        .align_x(align);
    let label = match style.color {
        Some(color) => label.color(color),
        None => label,
    };
    container(label)
        .width(Length::Fill)
        .max_width(width.max(1.0))
        .height(Length::Fixed((style.size * 1.3).ceil()))
        .clip(true)
        .into()
}

// ---------- Pulsanti ----------

/// Contenuto di un pulsante-riga ad altezza fissa, centrato in verticale.
pub fn centered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).height(Length::Fill).align_y(alignment::Vertical::Center).into()
}

fn labelled<'a>(label: &'a str, leading: Option<Icon>, color: Color) -> Element<'a, Message> {
    let mut content = row![].spacing(8).align_y(Alignment::Center);
    if let Some(leading) = leading {
        content = content.push(icon(leading, 16.0, color));
    }
    centered(content.push(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM)))
}

pub fn primary_button<'a>(label: &'a str, leading: Option<Icon>, on_press: Message) -> Element<'a, Message> {
    button(labelled(label, leading, theme::ON_PRIMARY))
        .height(theme::CONTROL_HEIGHT)
        .padding([0, 18])
        .style(theme::primary)
        .on_press(on_press)
        .into()
}

pub fn outlined_button<'a>(
    label: &'a str,
    leading: Option<Icon>,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(labelled(label, leading, theme::TEXT))
        .height(theme::CONTROL_HEIGHT)
        .padding([0, 16])
        .style(theme::outlined)
        .on_press_maybe(on_press)
        .into()
}

/// Come `outlined_button`, con l'etichetta di una stringa costruita al momento.
/// Come `outlined_button`, solo con l'icona (e il nome nel suggerimento): dove lo spazio è poco.
pub fn outlined_icon<'a>(glyph: Icon, label: &'a str, on_press: Option<Message>) -> Element<'a, Message> {
    let control = button(container(icon(glyph, 16.0, theme::TEXT)).center(Length::Fill))
        .width(theme::CONTROL_HEIGHT)
        .height(theme::CONTROL_HEIGHT)
        .padding(0)
        .style(theme::outlined)
        .on_press_maybe(on_press);
    tooltip(control, tip(label), tooltip::Position::Top).gap(4).into()
}

pub fn outlined_button_owned<'a>(
    label: String,
    leading: Option<Icon>,
    on_press: Message,
) -> Element<'a, Message> {
    let mut content = row![].spacing(8).align_y(Alignment::Center);
    if let Some(leading) = leading {
        content = content.push(icon(leading, 16.0, theme::TEXT));
    }
    button(centered(content.push(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM))))
        .height(theme::CONTROL_HEIGHT)
        .padding([0, 16])
        .style(theme::outlined)
        .on_press(on_press)
        .into()
}

pub fn text_button<'a>(label: &'a str, on_press: Message) -> Element<'a, Message> {
    // Un'etichetta di pulsante non va mai a capo («Ripro / va» in uno spazio stretto).
    button(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM).wrapping(text::Wrapping::None))
        .padding([4, 0])
        .style(theme::text_action)
        .on_press(on_press)
        .into()
}

/// Pulsante a icona con il suo nome in un tooltip (Iced 0.13 non espone nomi accessibili).
pub fn icon_button<'a>(
    glyph: Icon,
    label: &'a str,
    on: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let tint = if on { theme::TEXT } else { theme::MUTED };
    let control =
        button(icon(glyph, 18.0, tint)).padding(8).style(theme::icon_button(on)).on_press_maybe(on_press);
    tooltip(control, tip(label), tooltip::Position::Top).gap(4).into()
}

/// Come `icon_button`, ma acceso col verde del marchio: casuale e ripeti quando sono attivi.
pub fn toggle_button<'a>(
    glyph: Icon,
    label: &'a str,
    on: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let tint = if on { theme::ACCENT } else { theme::MUTED };
    let control =
        button(icon(glyph, 18.0, tint)).padding(8).style(theme::icon_button(on)).on_press_maybe(on_press);
    tooltip(control, tip(label), tooltip::Position::Top).gap(4).into()
}

/// Larghezza occupata da `icon_button`: icona 18 + padding 8 per lato.
pub const ICON_BUTTON: f32 = 34.0;

/// Riga del titolo di una pagina: il nome (h1), sotto una riga sui dati, a destra le azioni.
pub fn page_header<'a>(
    title: &'a str,
    subline: String,
    actions: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let heading = column![h1(title), muted(subline)].spacing(6).width(Length::Fill);
    let mut header = row![heading].align_y(Alignment::Center).spacing(16);
    if let Some(actions) = actions {
        header = header.push(actions);
    }
    header.into()
}

/// Lato del grande Play tondo di una pagina.
const PLAY_FAB: f32 = 56.0;
/// Lato della copertina nella testa di una pagina.
pub const HERO_COVER: f32 = 188.0;
/// Spazio minimo per il titolo accanto alla copertina della testa; sotto, la testa si impila.
const HERO_TEXT_MIN: f32 = 280.0;
pub const HERO_LINE: Line =
    Line { size: theme::FS_HERO, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };

/// Il grande Play verde di una pagina: avvia `play`.
fn play_fab<'a>(play: Message) -> Element<'a, Message> {
    let fab = button(container(icon(Icon::Play, 22.0, theme::ON_PRIMARY)).center(Length::Fill))
        .width(PLAY_FAB)
        .height(PLAY_FAB)
        .padding(0)
        .style(theme::play_fab)
        .on_press(play);
    tooltip(fab, tip("Riproduci"), tooltip::Position::Top).gap(4).into()
}

/// Riproduci (tondo, verde) e Casuale: il lavoro più frequente della pagina, a un clic.
pub fn play_controls<'a>(play: Message, shuffle: Message) -> Element<'a, Message> {
    row![play_fab(play), icon_button(Icon::Shuffle, "Riproduci in ordine casuale", false, Some(shuffle))]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// La barra sotto la testa di una pagina di brani: Play, Casuale, Scarica e le azioni `extra`. Sul
/// telefono, come in Spotify: a sinistra le azioni, a destra Casuale e il Play grande.
pub fn play_bar<'a>(
    app: &App,
    tracks: &Arc<[Track]>,
    extra: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let phone = crate::app::is_phone();
    let mut bar = row![].spacing(10).align_y(Alignment::Center);
    let mut shuffle = (!tracks.is_empty()).then(|| {
        icon_button(
            Icon::Shuffle,
            "Riproduci in ordine casuale",
            false,
            Some(Message::ShufflePlay(tracks.clone())),
        )
    });
    if !phone && !tracks.is_empty() {
        bar = bar.push(play_fab(Message::PlayTracks(tracks.clone(), 0)));
        bar = bar.extend(shuffle.take());
    }
    if !tracks.is_empty() {
        let downloadable = tracks.iter().filter(|t| t.downloadable).count();
        // Il download dell'intera lista: sul telefono c'è sempre, e se nessun brano si può scaricare lo
        // dice invece di sparire.
        if downloadable > 0 || phone {
            let label =
                if downloadable == tracks.len() { "Scarica tutto" } else { "Scarica i brani disponibili" };
            bar = bar.push(match app.download_progress() {
                // Mentre il lotto scarica, al posto dell'icona c'è il conteggio.
                Some((done, total)) => container(
                    text(format!("{done}/{total}"))
                        .size(theme::FS_SMALL)
                        .font(theme::FONT_MEDIUM)
                        .color(theme::ACCENT),
                )
                .height(ICON_BUTTON)
                .padding([0, 6])
                .align_y(alignment::Vertical::Center)
                .into(),
                None => icon_button(Icon::Download, label, false, Some(Message::DownloadAll(tracks.clone()))),
            });
        }
    }
    bar = extra.into_iter().fold(bar, iced::widget::Row::push);
    if phone && !tracks.is_empty() {
        bar = bar
            .push(Space::with_width(Length::Fill))
            .extend(shuffle)
            .push(play_fab(Message::PlayTracks(tracks.clone(), 0)))
            .width(Length::Fill);
    }
    bar.into()
}

/// La testa di una pagina: copertina grande con l'ombra, genere, titolo enorme e riga dei dati.
pub fn hero<'a>(
    art: Element<'a, Message>,
    kicker: &'a str,
    title: &str,
    meta: &str,
    width: f32,
) -> Element<'a, Message> {
    // Stretta (il telefono), la copertina affiancata lascerebbe al titolo una parola: si impila,
    // e il titolo va a capo invece di tagliarsi.
    if width < HERO_COVER + 24.0 + HERO_TEXT_MIN {
        let mut heading = column![].spacing(8);
        // Sul telefono il genere lo dice già la pagina: il titolo resta solo, più grande.
        if !crate::app::is_phone() {
            heading = heading
                .push(text(kicker).size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(theme::TEXT));
        }
        let size = if crate::app::is_phone() { theme::FS_PHONE_TITLE } else { theme::FS_PAGE };
        return column![
            container(container(art).style(theme::lifted)).center_x(Length::Fill),
            heading
                .push(text(title.to_owned()).size(size).font(theme::FONT_SEMIBOLD).color(theme::TEXT))
                .push(line(meta, width, SUB_LINE)),
        ]
        .spacing(20)
        .into();
    }
    let text_width = (width - HERO_COVER - 24.0).max(200.0);
    row![
        container(art).style(theme::lifted),
        column![
            text(kicker).size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(theme::TEXT),
            line(title, text_width, HERO_LINE),
            line(meta, text_width, SUB_LINE),
        ]
        .spacing(10)
        .width(Length::Fill),
    ]
    .spacing(24)
    .align_y(Alignment::End)
    .into()
}

pub(super) fn tip<'a>(label: &'a str) -> Element<'a, Message> {
    container(small(label).color(theme::TEXT)).padding([4, 8]).style(theme::panel).into()
}

/// Filtro a pillola con il suo conteggio: "Album 12".
pub fn pill<'a>(label: &'a str, count: String, selected: bool, on_press: Message) -> Element<'a, Message> {
    let count_color = if selected { theme::CONTROL } else { theme::FAINT };
    button(centered(
        row![
            text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM),
            text(count).size(theme::FS_BODY).color(count_color),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ))
    .height(theme::PILL_HEIGHT)
    .padding([0, 14])
    .style(theme::pill(selected))
    .on_press(on_press)
    .into()
}

/// Linea sottile tra gruppi di voci di un menu, con un po' d'aria sopra e sotto.
pub fn divider_line<'a>() -> Element<'a, Message> {
    container(container(Space::new(Length::Fill, Length::Fixed(1.0))).style(theme::bar(theme::HOVER, 0.0)))
        .padding([4, 6])
        .into()
}

// ---------- Stati ----------

pub fn skeleton<'a>(width: Length, height: f32) -> Element<'a, Message> {
    container(Space::new(width, Length::Fixed(height))).style(theme::skeleton).into()
}

/// Righe scheletro alla stessa altezza delle righe vere: niente salto quando arrivano i dati.
/// `kind` dice se la riga vera ha la copertina (le liste miste).
pub fn skeleton_rows<'a>(count: usize, height: f32, kind: TableKind) -> Element<'a, Message> {
    column((0..count).map(|_| {
        let mut cells = row![skeleton(Length::Fixed(24.0), 12.0)].spacing(16).align_y(Alignment::Center);
        if kind == TableKind::Mixed {
            cells = cells.push(skeleton(Length::Fixed(theme::ROW_COVER), theme::ROW_COVER));
        }
        container(
            cells
                .push(
                    column![skeleton(Length::FillPortion(3), 14.0), skeleton(Length::FillPortion(2), 12.0)]
                        .spacing(8)
                        .width(Length::FillPortion(5)),
                )
                .push(skeleton(Length::FillPortion(3), 12.0))
                .push(skeleton(Length::Fixed(40.0), 12.0)),
        )
        .height(height)
        .padding([0, 12])
        .align_y(alignment::Vertical::Center)
        .into()
    }))
    .into()
}

pub fn skeleton_tiles<'a>(count: usize) -> Element<'a, Message> {
    row((0..count).map(|_| {
        column![
            skeleton(Length::Fixed(TILE_COVER), TILE_COVER),
            skeleton(Length::Fixed(110.0), 14.0),
            skeleton(Length::Fixed(80.0), 12.0)
        ]
        .spacing(8)
        .padding(8)
        .into()
    }))
    .spacing(6)
    .into()
}

/// Errore dentro il blocco che è fallito, con "Riprova".
pub fn error_block<'a>(what: &'a str, err: &AppError) -> Element<'a, Message> {
    container(
        row![
            container(icon(Icon::Close, 14.0, theme::NEGATIVE)).padding(4),
            column![body(format!("Impossibile caricare {what}.")), small(err.to_string())]
                .spacing(2)
                .width(Length::Fill),
            text_button("Riprova", Message::Retry),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([12, 16])
    .width(Length::Fill)
    .style(theme::notice_box)
    .into()
}

pub fn empty_block<'a>(message: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(muted(message)).padding([12, 16]).width(Length::Fill).style(theme::notice_box).into()
}

/// Stato vuoto di una lista: che cosa manca, perché, e l'azione che la riempie.
pub fn empty_state<'a>(
    title: &'a str,
    sentence: &'a str,
    action: Option<(&'a str, Message)>,
) -> Element<'a, Message> {
    let mut content = column![h2(title), muted(sentence)].spacing(6);
    if let Some((label, on_press)) = action {
        content = content.push(Space::with_height(10)).push(outlined_button(label, None, Some(on_press)));
    }
    container(content).padding(theme::PAD).width(Length::Fill).style(theme::empty_card).into()
}

/// Sezione con titolo, azione a destra facoltativa e contenuto.
pub fn section<'a>(
    title: &'a str,
    trailing: Option<Element<'a, Message>>,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut header = row![h2(title), Space::with_width(Length::Fill)].align_y(Alignment::Center);
    if let Some(trailing) = trailing {
        header = header.push(trailing);
    }
    column![header, content].spacing(12).into()
}

/// Stato di un caricamento: scheletro, errore, oppure il contenuto.
pub fn loaded<'a, T>(
    load: &'a Load<T>,
    what: &'a str,
    placeholder: impl FnOnce() -> Element<'a, Message>,
    content: impl FnOnce(&'a T) -> Element<'a, Message>,
) -> Element<'a, Message> {
    match load {
        Load::Idle | Load::Loading => placeholder(),
        Load::Failed(err) => error_block(what, err),
        Load::Ready(value) => content(value),
    }
}

// ---------- Brani ----------

pub fn duration_label(ms: u64) -> String {
    format_time(std::time::Duration::from_millis(ms))
}

/// Durata complessiva in parole: "41 min" o "1 h 12 min".
pub fn total_label(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

pub fn count_label(n: usize) -> String {
    if n == 1 { "1 brano".into() } else { format!("{n} brani") }
}

/// Copertina di un brano: l'immagine se c'è; altrimenti colore e iniziale del suo album, oppure
/// i suoi se è un singolo.
pub fn art_of(track: &Track) -> Art<'_> {
    let url = track.cover_url.as_deref();
    match &track.album {
        Some(album) => Art { id: album.id, title: &album.title, url },
        None => Art { id: track.id, title: &track.title, url },
    }
}

pub fn album_art(album: &AlbumSummary) -> Art<'_> {
    Art { id: album.id, title: &album.title, url: album.cover_url.as_deref() }
}

pub fn artist_art(artist: &ArtistSummary) -> Art<'_> {
    Art { id: artist.id, title: &artist.name, url: artist.cover_url.as_deref() }
}

const ACTIONS_WIDTH: f32 = 3.0 * ICON_BUTTON + 4.0;
const NUMBER_WIDTH: f32 = 28.0;
const DURATION_WIDTH: f32 = 56.0;
const ROW_PADDING: f32 = 12.0;
const CELL_GAP: f32 = 16.0;
pub const TILE_COVER: f32 = 148.0;
/// Sotto questa larghezza per titolo e album, la colonna Album entra nella seconda riga: un album
/// tagliato a otto lettere non dice niente.
const ALBUM_COLUMN_MIN: f32 = 520.0;
/// Sotto questa larghezza la tabella diventa una lista da telefono: niente intestazione né
/// durata, solo cuore e «altro» a destra, così al titolo resta lo spazio.
const COMPACT_TABLE_BELOW: f32 = 480.0;
const COMPACT_ACTIONS_WIDTH: f32 = 2.0 * ICON_BUTTON + 2.0;
/// Lato del tasto Play che spunta sulla copertina.
pub(super) const FAB: f32 = 44.0;

/// Da dove viene una lista di brani: cambia le voci del menu contestuale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Catalogue,
    Playlist(Uuid),
    Liked,
}

impl Origin {
    fn source(self, index: usize) -> TrackSource {
        match self {
            Self::Catalogue => TrackSource::Catalogue,
            Self::Playlist(id) => TrackSource::Playlist(id, index),
            Self::Liked => TrackSource::Liked,
        }
    }
}

/// Il mouse è sull'elemento `id`.
pub(super) fn is_hot(app: &App, id: Uuid) -> bool {
    app.hovered.is_some_and(|(hovered, _)| hovered.id == id)
}

/// Da quanto il mouse è sull'elemento `id`, da 0 a 1; 0 se non c'è.
pub(super) fn hot_progress(app: &App, id: Uuid) -> f32 {
    match app.hovered {
        Some((hovered, since)) if hovered.id == id => motion::appear(since, theme::MOTION_FAST),
        _ => 0.0,
    }
}

pub(super) fn faded(style: Line, progress: f32) -> Line {
    Line { color: style.color.map(|color| fade(color, progress)), ..style }
}

/// Un elemento sul quale si ferma il mouse: ne mostra i controlli e, se il mouse resta, ne prepara
/// l'audio. Il clic destro apre il menu.
pub fn hoverable<'a>(
    id: Uuid,
    track: bool,
    menu: Option<MenuTarget>,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let area = mouse_area(content)
        .on_enter(Message::Hover(Some(Hovered { id, track })))
        .on_exit(Message::HoverEnd(id));
    match menu {
        Some(target) => area.on_right_press(Message::OpenMenu(target)).into(),
        None => area.into(),
    }
}

/// Il cuore di "mi piace": pieno e verde se piace, vuoto se no. Appena premuto fa un rimbalzo.
/// Se non piace compare solo con `visible` (il mouse è sulla riga).
pub fn heart<'a>(app: &App, track: &Track, visible: bool) -> Element<'a, Message> {
    let liked = app.is_liked(track.id);
    if !liked && !visible {
        return Space::with_width(Length::Fixed(ICON_BUTTON)).into();
    }
    let pop = match app.like_pop {
        Some((id, since)) if id == track.id => motion::pop(since, theme::MOTION_PANEL),
        _ => 0.0,
    };
    let (glyph, tint, label) = if liked {
        (Icon::HeartFill, theme::ACCENT, "Non mi piace più")
    } else {
        (Icon::Heart, theme::MUTED, "Mi piace")
    };
    let face = container(icon(glyph, 18.0 * (1.0 + 0.45 * pop), tint)).center(Length::Fill);
    let control = button(face)
        .width(ICON_BUTTON)
        .height(ICON_BUTTON)
        .padding(0)
        .style(theme::icon_button(liked))
        .on_press(Message::ToggleLike(track.clone()));
    tooltip(control, tip(label), tooltip::Position::Top).gap(4).into()
}

/// Barre dell'equalizzatore: salgono e scendono mentre il brano suona, ferme in pausa.
pub fn playing_bars<'a>(color: Color, moving: bool) -> Element<'a, Message> {
    const MAX: f32 = 14.0;
    let bars = (0..3).map(|lane| {
        let level = if moving { motion::wave(lane, Duration::from_millis(820)) } else { 0.25 };
        container(Space::new(Length::Fixed(3.0), Length::Fixed(3.0 + (MAX - 3.0) * level)))
            .style(theme::bar(color, 1.5))
            .into()
    });
    container(row(bars).spacing(2).align_y(Alignment::End))
        .height(MAX)
        .align_y(alignment::Vertical::Bottom)
        .into()
}

/// Azioni a destra di una riga: mi piace, scarica (solo se il brano lo consente) e altro. A
/// riposo restano solo il cuore di chi piace e lo stato di un download in corso.
fn row_actions<'a>(app: &App, track: &Track, source: TrackSource, hot: bool) -> Element<'a, Message> {
    let download: Element<'a, Message> = if app.downloading.contains(&track.id) {
        container(small("…")).width(ICON_BUTTON).align_x(alignment::Horizontal::Center).into()
    } else if track.downloadable && hot {
        icon_button(Icon::Download, "Scarica", false, Some(Message::Download(track.clone())))
    } else {
        // Nascosto, non disabilitato: lo spazio resta per allineare le colonne.
        Space::with_width(Length::Fixed(ICON_BUTTON)).into()
    };
    let more: Element<'a, Message> = if hot {
        let target = MenuTarget::track(track, source);
        icon_button(Icon::More, "Altre azioni", false, Some(Message::OpenMenu(target)))
    } else {
        Space::with_width(Length::Fixed(ICON_BUTTON)).into()
    };
    row![heart(app, track, hot), download, more].spacing(2).width(Length::Fixed(ACTIONS_WIDTH)).into()
}

/// Azioni di una riga stretta: cuore e altro. Sul telefono non c'è il passaggio del mouse, quindi
/// restano sempre visibili; il download sta nel menu.
fn compact_actions<'a>(app: &App, track: &Track, source: TrackSource, hot: bool) -> Element<'a, Message> {
    let visible = hot || crate::app::is_phone();
    let more: Element<'a, Message> = if visible {
        let target = MenuTarget::track(track, source);
        icon_button(Icon::More, "Altre azioni", false, Some(Message::OpenMenu(target)))
    } else {
        Space::with_width(Length::Fixed(ICON_BUTTON)).into()
    };
    row![heart(app, track, visible), more].spacing(2).width(Length::Fixed(COMPACT_ACTIONS_WIDTH)).into()
}

/// La cella del numero: il numero, e al passaggio del mouse Play; sul brano in corso le barre.
fn number_cell<'a>(
    app: &App,
    number: String,
    playing: bool,
    hot: bool,
    progress: f32,
) -> Element<'a, Message> {
    let content: Element<'a, Message> = match (playing, hot) {
        (_, true) => {
            let glyph = if playing && app.playback.is_playing() { Icon::Pause } else { Icon::Play };
            icon(glyph, 14.0, theme::TEXT).into()
        }
        (true, false) => playing_bars(theme::ACCENT, app.playback.is_playing()),
        (false, false) => text(number).size(theme::FS_SMALL).color(fade(theme::FAINT, progress)).into(),
    };
    container(content).width(NUMBER_WIDTH).align_x(alignment::Horizontal::Right).into()
}

fn table_header<'a>(cover: bool, show_album: bool) -> Element<'a, Message> {
    let mut header = row![container(small("#")).width(NUMBER_WIDTH).align_x(alignment::Horizontal::Right)]
        .spacing(CELL_GAP);
    if cover {
        header = header.push(Space::with_width(theme::ROW_COVER));
    }
    header = header.push(container(small("Titolo")).width(Length::FillPortion(5)));
    if show_album {
        header = header.push(container(small("Album")).width(Length::FillPortion(3)));
    }
    header
        .push(container(small("Durata")).width(DURATION_WIDTH).align_x(alignment::Horizontal::Right))
        .push(Space::with_width(ACTIONS_WIDTH))
        .padding([0.0, ROW_PADDING])
        .into()
}

/// Tipo di tabella: un album numera con `track_no` e non ripete l'album; una lista mista
/// (libreria, playlist) numera per posizione e mostra la colonna Album.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Album,
    Mixed,
}

/// Cosa succede al clic su una riga: sul brano in corso fa pausa o riprende, sugli altri parte.
fn row_press(tracks: &Arc<[Track]>, index: usize, playing: bool) -> Message {
    if playing { Message::TogglePlay } else { Message::PlayTracks(tracks.clone(), index) }
}

/// Tabella dei brani: #, copertina (liste miste), titolo e artista, album, durata. Un clic
/// riproduce dalla riga scelta. `width` è la larghezza disponibile, usata per tagliare i testi
/// lunghi e per decidere se l'album ha una colonna sua.
pub fn track_table<'a>(
    app: &App,
    tracks: &Arc<[Track]>,
    kind: TableKind,
    width: f32,
    origin: Origin,
) -> Element<'a, Message> {
    let mixed = kind == TableKind::Mixed;
    let compact = width < COMPACT_TABLE_BELOW;
    let cover_width = if mixed { theme::ROW_COVER + CELL_GAP } else { 0.0 };
    // Stretta, una lista mista perde il numero (c'è la copertina) e ogni lista perde la durata.
    let numbered = !(compact && mixed);
    let flexible = if compact {
        width
            - 2.0 * ROW_PADDING
            - if numbered { NUMBER_WIDTH + CELL_GAP } else { 0.0 }
            - cover_width
            - COMPACT_ACTIONS_WIDTH
            - CELL_GAP
    } else {
        width
            - 2.0 * ROW_PADDING
            - NUMBER_WIDTH
            - cover_width
            - DURATION_WIDTH
            - ACTIONS_WIDTH
            - CELL_GAP * 4.0
    };
    // La colonna dell'identità (il titolo) ha sempre più spazio: l'album prende 3/8 solo se avanza.
    let show_album = mixed && !compact && flexible >= ALBUM_COLUMN_MIN;
    let (title_width, album_width) = if show_album {
        let title = (flexible - CELL_GAP) * 5.0 / 8.0;
        (title, flexible - CELL_GAP - title)
    } else {
        (flexible, 0.0)
    };

    let entered = app.page_motion.elapsed();
    let rows = tracks.iter().enumerate().map(|(index, track)| {
        let playing = app.is_current(track.id);
        let hot = is_hot(app, track.id);
        let progress = motion::stagger(entered, index);
        let number = match (kind, track.track_no) {
            (TableKind::Album, Some(n)) => n.to_string(),
            _ => (index + 1).to_string(),
        };
        // Senza colonna Album, l'album sta nella seconda riga accanto all'artista.
        let subline = match (&track.album, mixed && !show_album) {
            (Some(album), true) => format!("{} · {}", track.artist.name, album.title),
            (None, true) => format!("{} · Singolo", track.artist.name),
            (_, false) => track.artist.name.clone(),
        };
        let mut cells = row![].spacing(CELL_GAP).align_y(Alignment::Center);
        if numbered {
            cells = cells.push(number_cell(app, number, playing, hot, progress));
        }
        if mixed {
            cells = cells.push(cover(&app.covers, art_of(track), theme::ROW_COVER));
        }
        let title_style = if playing { PLAYING_LINE } else { TITLE_LINE };
        cells = cells.push(
            column![
                line(&track.title, title_width, faded(title_style, progress)),
                button(line(&subline, title_width, faded(SUB_LINE, progress)))
                    .padding(0)
                    .style(theme::text_action)
                    .on_press(Message::Navigate(Screen::Artist(track.artist.id))),
            ]
            .spacing(4)
            .width(Length::FillPortion(5)),
        );
        if show_album {
            let album: Element<'a, Message> = match &track.album {
                Some(album) => button(line(&album.title, album_width, faded(MUTED_LINE, progress)))
                    .padding(0)
                    .style(theme::text_action)
                    .on_press(Message::Navigate(Screen::Album(album.id)))
                    .into(),
                None => small("Singolo").into(),
            };
            cells = cells.push(container(album).width(Length::FillPortion(3)));
        }
        let source = origin.source(index);
        if compact {
            cells = cells.push(compact_actions(app, track, source, hot));
        } else {
            cells = cells
                .push(
                    container(
                        text(duration_label(track.duration_ms))
                            .size(theme::FS_BODY)
                            .color(fade(theme::MUTED, progress)),
                    )
                    .width(DURATION_WIDTH)
                    .align_x(alignment::Horizontal::Right),
                )
                .push(row_actions(app, track, source, hot));
        }
        let item = button(centered(cells))
            .height(theme::ROW_PLAIN)
            .width(Length::Fill)
            .padding([0.0, ROW_PADDING])
            .style(theme::row(playing))
            .on_press(row_press(tracks, index, playing));
        hoverable(track.id, true, Some(MenuTarget::track(track, source)), item.into())
    });
    if compact {
        return column(rows).into();
    }
    column![table_header(mixed, show_album), Space::with_height(4), column(rows)].into()
}

/// Righe con copertina (ricerca): media da 50px su un passo di 64px.
/// Stretta (telefono), come la tabella: niente durata e solo cuore e altro.
pub fn track_rows<'a>(app: &App, tracks: &Arc<[Track]>, width: f32) -> Element<'a, Message> {
    let compact = width < COMPACT_TABLE_BELOW;
    let text_width = if compact {
        width - 2.0 * ROW_PADDING - 50.0 - COMPACT_ACTIONS_WIDTH - CELL_GAP * 2.0
    } else {
        width - 2.0 * ROW_PADDING - 50.0 - DURATION_WIDTH - ACTIONS_WIDTH - CELL_GAP * 3.0
    };
    let entered = app.page_motion.elapsed();
    column(tracks.iter().enumerate().map(|(index, track)| {
        let playing = app.is_current(track.id);
        let hot = is_hot(app, track.id);
        let progress = motion::stagger(entered, index);
        let subtitle = match &track.album {
            Some(album) => format!("{} · {}", track.artist.name, album.title),
            None => format!("{} · Singolo", track.artist.name),
        };
        let source = TrackSource::Catalogue;
        let mut cells = row![
            cover(&app.covers, art_of(track), 50.0),
            column![
                line(
                    &track.title,
                    text_width,
                    faded(if playing { PLAYING_LINE } else { TITLE_LINE }, progress)
                ),
                line(&subtitle, text_width, faded(SUB_LINE, progress)),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(CELL_GAP)
        .align_y(Alignment::Center);
        cells = if compact {
            cells.push(compact_actions(app, track, source, hot))
        } else {
            cells
                .push(
                    container(muted(duration_label(track.duration_ms)))
                        .width(DURATION_WIDTH)
                        .align_x(alignment::Horizontal::Right),
                )
                .push(row_actions(app, track, source, hot))
        };
        let item = button(centered(cells))
            .height(theme::ROW_MEDIA)
            .width(Length::Fill)
            .padding(Padding::from([0.0, ROW_PADDING]))
            .style(theme::row(playing))
            .on_press(row_press(tracks, index, playing));
        hoverable(track.id, true, Some(MenuTarget::track(track, source)), item.into())
    }))
    .into()
}

/// Ciò che serve a disegnare una tessera.
pub(super) struct Tile<'a> {
    id: Uuid,
    art: Art<'a>,
    title: &'a str,
    subtitle: &'a str,
    on_press: Message,
    /// Menu del clic destro, se la tessera ne ha uno.
    menu: Option<MenuTarget>,
    /// Il tasto Play sulla copertina (le tessere dei brani; gli album si aprono).
    play_badge: bool,
    /// Se c'è, il tasto Play della copertina è un pulsante vero che manda questo messaggio, e il
    /// clic sul resto della tessera fa altro (apre l'album).
    play: Option<Message>,
    /// Copertina tonda (artisti).
    round: bool,
    /// Etichetta sull'angolo della copertina («Nuovo»).
    badge: Option<&'a str>,
}

impl<'a> Tile<'a> {
    pub(super) fn new(id: Uuid, art: Art<'a>, title: &'a str, subtitle: &'a str, on_press: Message) -> Self {
        Self {
            id,
            art,
            title,
            subtitle,
            on_press,
            menu: None,
            play_badge: false,
            play: None,
            round: false,
            badge: None,
        }
    }

    pub(super) fn menu(mut self, menu: MenuTarget) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Il Play compare sulla copertina; il clic sulla tessera lo fa partire.
    pub(super) fn play_badge(mut self) -> Self {
        self.play_badge = true;
        self
    }

    /// Il Play compare sulla copertina ed è un pulsante a sé.
    pub(super) fn play_button(mut self, play: Message) -> Self {
        self.play_badge = true;
        self.play = Some(play);
        self
    }

    pub(super) fn round(mut self) -> Self {
        self.round = true;
        self
    }

    pub(super) fn badge(mut self, label: &'a str) -> Self {
        self.badge = Some(label);
        self
    }
}

/// La copertina con, al passaggio del mouse, il tasto Play tondo che sale dal basso.
fn tile_cover<'a>(app: &App, spec: &Tile<'_>, side: f32) -> Element<'a, Message> {
    let base: Element<'a, Message> = if spec.round {
        cover_round(&app.covers, spec.art, side)
    } else {
        cover(&app.covers, spec.art, side)
    };
    let progress = hot_progress(app, spec.id);
    let mut layers = stack![base];
    if let Some(label) = spec.badge {
        layers = layers.push(
            container(
                container(
                    text(label.to_string())
                        .size(theme::FS_SMALL)
                        .font(theme::FONT_SEMIBOLD)
                        .color(theme::ON_PRIMARY),
                )
                .padding([2, 8])
                .style(theme::badge),
            )
            .padding(8),
        );
    }
    if spec.play_badge && progress > 0.0 {
        let face = container(icon(Icon::Play, 18.0, fade(theme::ON_PRIMARY, progress))).center(FAB);
        let play: Element<'a, Message> = match &spec.play {
            Some(message) => button(face)
                .width(FAB)
                .height(FAB)
                .padding(0)
                .style(theme::fab_button(progress))
                .on_press(message.clone())
                .into(),
            None => container(face).style(theme::fab(progress)).into(),
        };
        layers = layers.push(
            container(play)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Right)
                .align_y(alignment::Vertical::Bottom)
                .padding(Padding { right: 8.0, bottom: 8.0 - 6.0 * (1.0 - progress), ..Padding::ZERO }),
        );
    }
    layers.into()
}

/// Una tessera: copertina, titolo e riga secondaria.
fn tile<'a>(app: &App, spec: Tile<'_>) -> Element<'a, Message> {
    sized_tile(app, spec, TILE_COVER)
}

/// Tessera con la copertina di lato `side`; il padding di 8 è il fondo che compare al passaggio.
pub(super) fn sized_tile<'a>(app: &App, spec: Tile<'_>, side: f32) -> Element<'a, Message> {
    let align = if spec.round { alignment::Horizontal::Center } else { alignment::Horizontal::Left };
    let item = button(
        column![
            tile_cover(app, &spec, side),
            line_aligned(spec.title, side, TITLE_LINE, align),
            line_aligned(spec.subtitle, side, SUB_LINE, align)
        ]
        .spacing(6)
        .width(side),
    )
    .padding(8)
    .style(theme::tile)
    .on_press(spec.on_press);
    hoverable(spec.id, spec.play_badge, spec.menu, item.into())
}

/// Scorciatoia della Home: copertina a filo, titolo e, al passaggio del mouse, il Play verde.
pub fn shortcut<'a>(
    app: &App,
    id: Uuid,
    art: Element<'a, Message>,
    title: &str,
    width: f32,
    on_press: Message,
    menu: Option<MenuTarget>,
) -> Element<'a, Message> {
    let progress = hot_progress(app, id);
    // Niente spazio elastico accanto al titolo: si dividerebbero la riga e il titolo resterebbe
    // tagliato a metà. Il titolo occupa ciò che resta; il Play, quando compare, se ne prende un pezzo.
    let play = if progress > 0.0 { 16.0 + FAB } else { 0.0 };
    // Sul telefono la cella è stretta: il titolo va a capo su due righe invece di tagliarsi.
    let room = width - 56.0 - 16.0 - 12.0 - play;
    let label = if crate::app::is_phone() {
        wrapped(title, room, TITLE_LINE, 2)
    } else {
        line(title, room, TITLE_LINE)
    };
    let mut cells = row![art, label].spacing(16).align_y(Alignment::Center);
    if progress > 0.0 {
        cells = cells.push(
            container(icon(Icon::Play, 18.0, fade(theme::ON_PRIMARY, progress)))
                .center(FAB)
                .style(theme::fab(progress)),
        );
    }
    let item = button(cells.padding(Padding { right: 12.0, ..Padding::ZERO }))
        .height(56)
        .width(Length::Fill)
        .padding(0)
        .style(theme::shortcut)
        .on_press(on_press);
    hoverable(id, false, menu, item.into())
}

pub(super) fn scroll_row<'a>(tiles: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    // Sul telefono la fila si scorre col dito: la barra non serve e sembrerebbe un carosello.
    let bar = if crate::app::is_phone() { 0.0 } else { 4.0 };
    let gap = if crate::app::is_phone() { 0.0 } else { 12.0 };
    scrollable(container(row(tiles).spacing(6)).padding(Padding::default().bottom(gap)))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(bar).scroller_width(bar),
        ))
        .style(theme::scroll)
        .into()
}

/// Fila orizzontale di tessere: copertina, titolo, artista.
pub fn tile_row<'a>(app: &App, tracks: &Arc<[Track]>) -> Element<'a, Message> {
    scroll_row((0..tracks.len()).map(|index| track_tile(app, tracks, index, TILE_COVER)).collect())
}

/// Una tessera di brano: il clic lo fa partire dentro la sua lista.
pub fn track_tile<'a>(app: &App, tracks: &Arc<[Track]>, index: usize, side: f32) -> Element<'a, Message> {
    let track = &tracks[index];
    let spec = Tile::new(
        track.id,
        art_of(track),
        &track.title,
        &track.artist.name,
        Message::PlayTracks(tracks.clone(), index),
    )
    .menu(MenuTarget::track(track, TrackSource::Catalogue))
    .play_badge();
    sized_tile(app, spec, side)
}

fn album_spec(album: &AlbumSummary) -> Tile<'_> {
    Tile::new(
        album.id,
        album_art(album),
        &album.title,
        &album.artist.name,
        Message::Navigate(Screen::Album(album.id)),
    )
    .menu(MenuTarget::album(album))
    .play_button(Message::PlayAlbum(album.id))
}

pub fn album_tile<'a>(app: &App, album: &AlbumSummary) -> Element<'a, Message> {
    tile(app, album_spec(album))
}

/// Fila orizzontale di album: copertina, titolo, artista. Un clic apre l'album.
pub fn album_tile_row<'a>(app: &App, albums: &[AlbumSummary]) -> Element<'a, Message> {
    scroll_row(albums.iter().map(|album| album_tile(app, album)).collect())
}

/// Fila orizzontale di artisti: immagine, nome, "Artista". Un clic apre la pagina dell'artista.
pub fn artist_tile_row<'a>(app: &App, artists: &[ArtistSummary]) -> Element<'a, Message> {
    scroll_row(
        artists
            .iter()
            .map(|artist| {
                let spec = Tile::new(
                    artist.id,
                    artist_art(artist),
                    &artist.name,
                    "Artista",
                    Message::Navigate(Screen::Artist(artist.id)),
                )
                .menu(MenuTarget::artist(artist.id, &artist.name, artist.cover_url.as_deref()))
                .round();
                tile(app, spec)
            })
            .collect(),
    )
}

/// Colonne e lato delle copertine di una griglia larga `width`: quante tessere da almeno
/// `ALBUM_TILE_MIN` ci stanno, poi le copertine crescono finché la griglia tocca i due bordi.
/// La somma chiude: `columns * (side + 16) + (columns - 1) * GRID_GAP == width`.
pub fn grid_columns(width: f32) -> (usize, f32) {
    let pad = 16.0;
    let fits = ((width + theme::GRID_GAP) / (theme::ALBUM_TILE_MIN + pad + theme::GRID_GAP)).floor();
    let columns = (fits as usize).max(2);
    let side = (width - theme::GRID_GAP * (columns as f32 - 1.0)) / columns as f32 - pad;
    (columns, side.floor().max(1.0))
}

/// Griglia degli album (libreria) sui due bordi della pagina; l'ultima fila parte da sinistra.
pub fn album_grid<'a>(app: &App, albums: &[AlbumSummary], width: f32) -> Element<'a, Message> {
    let (columns, side) = grid_columns(width);
    column(albums.chunks(columns).map(|chunk| {
        row(chunk.iter().map(|album| sized_tile(app, album_spec(album), side)))
            .spacing(theme::GRID_GAP)
            .into()
    }))
    .spacing(theme::GRID_GAP)
    .into()
}

/// Scheletro della griglia: le stesse colonne e la stessa altezza delle tessere vere.
pub fn skeleton_grid<'a>(width: f32, rows: usize) -> Element<'a, Message> {
    let (columns, side) = grid_columns(width);
    column((0..rows).map(|_| {
        row((0..columns).map(|_| {
            column![
                skeleton(Length::Fixed(side), side),
                skeleton(Length::Fixed(side * 0.7), 14.0),
                skeleton(Length::Fixed(side * 0.5), 12.0)
            ]
            .spacing(8)
            .padding(8)
            .into()
        }))
        .spacing(theme::GRID_GAP)
        .into()
    }))
    .spacing(theme::GRID_GAP)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_album_grid_closes_on_both_edges() {
        for width in [500.0, 731.0, 1024.0, 1400.0] {
            let (columns, side) = grid_columns(width);
            let used = columns as f32 * (side + 16.0) + (columns as f32 - 1.0) * theme::GRID_GAP;
            assert!((width - used) >= 0.0 && (width - used) < columns as f32, "{width}: {used}");
        }
    }

    #[test]
    fn album_covers_never_shrink_below_the_minimum_when_two_fit() {
        let (columns, side) = grid_columns(731.0);
        assert!(side >= theme::ALBUM_TILE_MIN && columns == 3, "{columns} × {side}");
    }

    #[test]
    fn total_rounds_to_minutes() {
        assert_eq!(total_label(2_470_000), "41 min");
    }

    #[test]
    fn total_over_an_hour_shows_hours() {
        assert_eq!(total_label(4_320_000), "1 h 12 min");
    }

    #[test]
    fn single_track_is_singular() {
        assert_eq!(count_label(1), "1 brano");
    }

    #[test]
    fn short_text_is_not_cut() {
        assert_eq!(fit("Sale", 200.0, 14.0), "Sale");
    }

    #[test]
    fn long_text_ends_with_ellipsis_within_budget() {
        let cut = fit("Una passeggiata lunghissima lungo il porto vecchio", 100.0, 14.0);
        assert!(cut.ends_with('…') && super::super::lyrics::em_width(&cut) * 14.0 <= 100.0, "{cut}");
    }

    #[test]
    fn a_title_that_fits_by_real_width_is_not_cut() {
        // «One Step Closer» a 15 px misura circa 118 px: la vecchia stima (0,56 em) ne voleva 126.
        assert_eq!(fit("One Step Closer", 125.0, 15.0), "One Step Closer");
    }

    #[test]
    fn ellipsis_does_not_follow_a_space() {
        assert!(!fit("ab cd ef gh ij kl", 47.0, 14.0).contains(" …"));
    }

    #[test]
    fn zero_width_still_shows_one_character() {
        assert_eq!(fit("Fondale", 0.0, 14.0), "…");
    }
}
