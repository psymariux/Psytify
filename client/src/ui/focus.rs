//! Le due viste a tutto schermo di «In riproduzione». Si aprono da tasti diversi e si chiudono con
//! Esc o con la X.
//!
//! - **Testo** (tasto F): come Lyrics Plus di Spicetify o il testo di Apple Music. Dietro, la
//!   copertina sfocata che si muove piano; davanti un blocco centrato: la copertina (che respira con
//!   il Play) con l'avanzamento, il titolo e i controlli, e accanto il testo sincronizzato di
//!   `ui/lyrics.rs`, che scivola fino alla riga in corso e la accende parola per parola.
//! - **Videoclip** (tasto V): il video riempie la finestra, il testo sincronizzato sta sopra come
//!   sottotitolo (si nasconde con il pulsante) e sotto ci sono i controlli.

use std::time::Duration;

use iced::widget::{Space, column, container, mouse_area, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, alignment};

use super::components::{
    ICON_BUTTON, Line, MUTED_LINE, SUB_LINE, art_of, heart, icon_button, line, line_aligned, small,
    toggle_button,
};
use super::cover::{backdrop, cover};
use super::icons::Icon;
use super::motion::{self, fade};
use super::pages::video;
use super::{lyrics as ui_lyrics, now_playing, theme, top_bar};
use crate::api::Track;
use crate::app::{App, Load, Message};

const TITLE_LINE: Line = Line { size: 18.0, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };
const ARTIST_LINE: Line = Line { size: 14.0, font: theme::FONT, color: Some(theme::MUTED) };
const STAGE_TITLE: Line = Line { size: 24.0, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };
const STAGE_ARTIST: Line = Line { size: 15.0, font: theme::FONT_MEDIUM, color: Some(theme::MUTED) };
/// Lato della copertina nell'intestazione della vista del videoclip.
const HEADER_COVER: f32 = 48.0;
/// Margine dai bordi e fra la colonna della copertina e il testo; larghezza massima di un verso.
const MARGIN: f32 = 64.0;
const LYRICS_MAX_WIDTH: f32 = 760.0;
/// Larghezza minima della colonna della copertina: ci devono stare i controlli.
const LEFT_MIN: f32 = 300.0;
/// Spazio sotto la copertina per avanzamento, titolo, artista e controlli.
const UNDER_COVER: f32 = 190.0;
/// Lato massimo della copertina accanto al testo, e da sola (senza testo).
const COVER_MAX: f32 = 640.0;
const COVER_ALONE_MAX: f32 = 760.0;

/// Il livello a tutta finestra; `None` se la vista non è aperta o non c'è un brano.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    if !app.focus {
        return None;
    }
    let track = app.queue.current()?;
    Some(if app.focus_video && app.video_available() {
        video_stage(app, track)
    } else {
        lyrics_stage(app, track)
    })
}

/// Chiude la vista; i pulsanti per passare all'altra stanno a sinistra della X.
fn top_bar<'a>(app: &'a App, lead: Element<'a, Message>) -> Element<'a, Message> {
    let mut actions = row![].spacing(4).align_y(Alignment::Center);
    if app.video_available() {
        actions = actions.push(toggle_button(
            Icon::Video,
            if app.focus_video { "Torna al testo" } else { "Videoclip a tutto schermo (V)" },
            app.focus_video,
            Some(Message::ToggleVideoFocus),
        ));
    }
    if app.focus_video {
        actions = actions.push(toggle_button(
            Icon::Lyrics,
            if app.video_lyrics { "Nascondi il testo" } else { "Mostra il testo" },
            app.video_lyrics,
            Some(Message::ToggleVideoLyrics),
        ));
        actions = actions.push(toggle_button(
            Icon::Note,
            "Solo testo a tutto schermo",
            false,
            Some(Message::ToggleFocus),
        ));
    }
    mouse_area(
        row![
            lead,
            Space::with_width(Length::Fill),
            actions,
            icon_button(Icon::Close, "Chiudi (Esc)", false, Some(Message::ToggleFocus)),
            top_bar::window_controls(app),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .height(theme::TOP_BAR_HEIGHT)
        .padding([0, 12]),
    )
    .on_press(Message::TitleBarPressed)
    .into()
}

/// Titolo e artista del brano, con la copertina piccola e il cuore.
fn heading<'a>(app: &'a App, track: &'a Track, width: f32) -> Element<'a, Message> {
    row![
        cover(&app.covers, art_of(track), HEADER_COVER),
        column![line(&track.title, width, TITLE_LINE), line(&track.artist.name, width, ARTIST_LINE)]
            .spacing(2),
        heart(app, track, true),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn controls(app: &App) -> Element<'_, Message> {
    container(now_playing::transport(app))
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .padding([0, 56])
        .into()
}

// ---------- Testo ----------

fn lyrics_stage<'a>(app: &'a App, track: &'a Track) -> Element<'a, Message> {
    let window = app.window_size;
    let stage_height = (window.height - theme::TOP_BAR_HEIGHT - MARGIN / 2.0).max(240.0);
    let has_text = match &app.lyrics {
        Load::Idle | Load::Loading => true,
        Load::Failed(_) => false,
        Load::Ready(text) => text.is_synced() || text.plain.is_some(),
    };
    // La copertina e il testo formano un blocco solo, centrato nella finestra: niente vuoto a destra
    // sugli schermi larghi. Senza testo la copertina sta al centro, più grande.
    let body: Element<'a, Message> = if has_text {
        let side = (stage_height - UNDER_COVER).min(window.width * 0.36).clamp(140.0, COVER_MAX);
        let left_width = side.max(LEFT_MIN);
        let width = (window.width - left_width - 3.0 * MARGIN).clamp(240.0, LYRICS_MAX_WIDTH);
        let left = now_playing_column(app, track, side, left_width);
        container(
            row![left, lyrics_view(app, width, stage_height)].spacing(MARGIN).align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .into()
    } else {
        let side = (stage_height - UNDER_COVER).min(window.width * 0.5).clamp(140.0, COVER_ALONE_MAX);
        let left = now_playing_column(app, track, side, side.max(LEFT_MIN));
        container(left).width(Length::Fill).height(Length::Fill).center(Length::Fill).into()
    };
    let credit = small("Testi da LRCLIB").color(theme::MUTED);
    let page = column![top_bar(app, credit.into()), body, Space::with_height(MARGIN / 2.0)]
        .width(Length::Fill)
        .height(Length::Fill);

    // Dietro, la copertina sfocata che cambia piano colore; finché non arriva, la sua tinta.
    let phase = 0.5 + 0.5 * motion::drift(Duration::from_secs(24));
    match backdrop(&app.covers, art_of(track), phase) {
        Some(ground) => {
            stack![ground, container(Space::new(Length::Fill, Length::Fill)).style(theme::veil), page]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        None => {
            let tint = app.covers.tint_of(art_of(track));
            let drift = if app.playback.is_playing() { motion::drift(Duration::from_secs(14)) } else { 0.0 };
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::aurora_full(tint, drift))
                .into()
        }
    }
}

/// La copertina grande: respira con il Play (in pausa scende un po', al Play torna con la molla).
/// Il posto resta quello della copertina intera, così il resto della colonna non si muove.
pub fn breathing_cover<'a>(app: &'a App, track: &'a Track, side: f32) -> Element<'a, Message> {
    let shown = side * app.cover_motion.value();
    container(container(cover(&app.covers, art_of(track), shown)).style(theme::lifted))
        .width(side)
        .height(side)
        .center(side)
        .into()
}

/// La colonna della copertina: copertina, avanzamento, titolo con il cuore, artista e album,
/// controlli.
fn now_playing_column<'a>(app: &'a App, track: &'a Track, side: f32, width: f32) -> Element<'a, Message> {
    let mut detail = track.artist.name.clone();
    if let Some(album) = &track.album {
        detail = format!("{detail} · {}", album.title);
    }
    let title_width = width - 2.0 * ICON_BUTTON;
    let title = row![
        Space::with_width(ICON_BUTTON),
        line_aligned(&track.title, title_width, STAGE_TITLE, alignment::Horizontal::Center),
        heart(app, track, true),
    ]
    .align_y(Alignment::Center);
    column![
        breathing_cover(app, track, side),
        container(now_playing::progress(app)).width(side.max(LEFT_MIN - 40.0)),
        column![title, line_aligned(&detail, width, STAGE_ARTIST, alignment::Horizontal::Center)]
            .spacing(2)
            .align_x(Alignment::Center),
        now_playing::controls(app),
    ]
    .spacing(14)
    .width(width)
    .align_x(Alignment::Center)
    .into()
}

/// Il testo: sincronizzato (scorre da solo), semplice (si scorre a mano) o in arrivo.
fn lyrics_view<'a>(app: &'a App, width: f32, height: f32) -> Element<'a, Message> {
    match &app.lyrics {
        Load::Ready(shown) if shown.is_synced() => {
            let look = if width < 520.0 { ui_lyrics::STAGE_NARROW } else { ui_lyrics::STAGE };
            ui_lyrics::synced(app, shown, look, width, height)
        }
        Load::Ready(shown) => match &shown.plain {
            Some(plain) => {
                let verses = plain.lines().map(|verse| {
                    text(verse.to_owned())
                        .size(26.0)
                        .font(theme::FONT_SEMIBOLD)
                        .color(fade(theme::TEXT, 0.72))
                        .into()
                });
                scrollable(column(verses).spacing(10).width(width)).height(height).style(theme::scroll).into()
            }
            None => line("Nessun testo per questo brano.", width, SUB_LINE),
        },
        Load::Idle | Load::Loading | Load::Failed(_) => {
            container(line("Caricamento del testo…", width, MUTED_LINE))
                .width(width)
                .height(height)
                .center_y(height)
                .into()
        }
    }
}

// ---------- Videoclip ----------

fn video_stage<'a>(app: &'a App, track: &'a Track) -> Element<'a, Message> {
    let window = app.window_size;
    let captions = app
        .video_lyrics
        .then(|| {
            video::caption(app, (window.width - 2.0 * MARGIN).clamp(240.0, LYRICS_MAX_WIDTH), 30.0, true)
        })
        .flatten();
    let overlay = column![
        container(top_bar(app, heading(app, track, 320.0))).width(Length::Fill).style(theme::scrim(0.8)),
        Space::with_height(Length::Fill),
        container(column![].push_maybe(captions).align_x(Alignment::Center))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        container(controls(app)).padding([12, 0]).width(Length::Fill).style(theme::scrim(0.8)),
    ];
    stack![
        container(video::stage(app)).width(Length::Fill).height(Length::Fill).style(theme::ground),
        overlay
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
