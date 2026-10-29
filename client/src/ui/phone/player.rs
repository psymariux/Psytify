//! Il player a schermo intero del telefono: la copertina grande che respira, titolo e artista, la
//! barra di avanzamento con i tempi sotto, i controlli grandi per il pollice e, in fondo, il testo
//! sincronizzato o la coda.

use std::time::Duration;

use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Alignment, Color, Element, Length, Padding, Size, alignment};

use super::touch_icon;
use crate::app::{App, MenuTarget, Message, SideTab};
use crate::player::queue::Repeat;
use crate::player::seek::{format_time, progress as progress_of};
use crate::ui::components::{H1_LINE, MUTED_LINE, art_of, heart, line, small};
use crate::ui::focus::breathing_cover;
use crate::ui::icons::{Icon, icon};
use crate::ui::{lyrics, motion, side_panel, theme};

/// Margine del player dai bordi.
const GUTTER: f32 = 24.0;
/// Il tasto Play: un tondo bianco, il più grande dello schermo dopo la copertina.
const PLAY: f32 = 68.0;
/// Precedente e successivo: più grandi degli interruttori ai lati.
const SKIP: f32 = 56.0;

pub fn view(app: &App, window: Size, edges: Padding) -> Element<'_, Message> {
    let Some(track) = app.queue.current() else {
        return Space::new(Length::Fill, Length::Fill).into();
    };
    let width = (window.width - edges.left - edges.right - 2.0 * GUTTER).max(200.0);
    let header = row![
        touch_icon(Icon::ChevronDown, theme::TEXT, Message::ToggleFocus),
        container(text("In riproduzione").size(theme::FS_SMALL).font(theme::FONT_MEDIUM).color(theme::MUTED))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        touch_icon(Icon::More, theme::TEXT, Message::OpenMenu(MenuTarget::Player)),
    ]
    .align_y(Alignment::Center);

    let side = (window.height * theme::PHONE_PLAYER_COVER).min(width).max(160.0);
    let mut detail = track.artist.name.clone();
    if let Some(album) = &track.album {
        detail = format!("{detail} · {}", album.title);
    }
    let title_width = width - theme::TOUCH_TARGET - 8.0;
    let title = row![
        column![line(&track.title, title_width, H1_LINE), line(&detail, title_width, MUTED_LINE)]
            .spacing(6)
            .width(Length::Fill),
        heart(app, track, true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let queue_open = app.side_tab == SideTab::Queue;
    let choice = |label: &'static str, tab: SideTab, on: bool| -> Element<'_, Message> {
        button(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM))
            .padding([8, 18])
            .style(theme::pill(on))
            .on_press(Message::SideTab(tab))
            .into()
    };
    let tabs =
        row![choice("Testo", SideTab::Lyrics, !queue_open), choice("Coda", SideTab::Queue, queue_open)]
            .spacing(8);
    let below: Element<'_, Message> =
        if queue_open { side_panel::queue_body(app) } else { lyrics::view(app) };

    let page = column![
        header,
        Space::with_height(8),
        container(breathing_cover(app, track, side))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        Space::with_height(12),
        title,
        progress(app),
        controls(app),
        Space::with_height(4),
        tabs,
        container(below).height(Length::Fill),
    ]
    .spacing(14)
    .height(Length::Fill);

    let tint = app.covers.tint_of(art_of(track));
    let drift = if app.playback.is_playing() { motion::drift(Duration::from_secs(14)) } else { 0.0 };
    container(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: edges.top + 4.0,
            bottom: edges.bottom + 12.0,
            left: edges.left + GUTTER - 10.0,
            right: edges.right + GUTTER - 10.0,
        })
        .style(theme::aurora_full(tint, drift))
        .into()
}

/// La barra trascinabile a tutta larghezza, con il tempo trascorso e la durata sotto i due capi.
fn progress(app: &App) -> Element<'_, Message> {
    let duration = app.queue.current().map(|t| t.duration()).unwrap_or_default();
    let position = app.shown_position().min(duration);
    let value = app.playback.scrubbing.unwrap_or_else(|| progress_of(position, duration));
    let bar = slider(0.0..=1.0, value, Message::Scrub)
        .step(0.001_f32)
        .on_release(Message::ScrubReleased)
        .style(theme::progress_slider);
    column![
        bar,
        row![small(format_time(position)), Space::with_width(Length::Fill), small(format_time(duration))],
    ]
    .spacing(2)
    .into()
}

/// Casuale, precedente, Play, successivo e ripeti, distribuiti sulla riga.
fn controls(app: &App) -> Element<'_, Message> {
    let play_glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    // Al cambio fra Play e Pausa l'icona rimbalza un poco; il tasto resta della sua misura.
    let pop = app.play_pop.map_or(0.0, |since| motion::pop(since, theme::MOTION_PANEL));
    let play =
        button(container(icon(play_glyph, 28.0 * (1.0 + 0.2 * pop), theme::ON_PRIMARY)).center(Length::Fill))
            .width(PLAY)
            .height(PLAY)
            .padding(0)
            .style(theme::primary)
            .on_press(Message::TogglePlay);
    let skip = |glyph: Icon, message: Message| {
        button(container(icon(glyph, 30.0, theme::TEXT)).center(Length::Fill))
            .width(SKIP)
            .height(SKIP)
            .padding(0)
            .style(theme::bare)
            .on_press(message)
    };
    // Gli interruttori accesi sono lime con un punto sotto, come nelle app da telefono.
    let switch = |glyph: Icon, on: bool, message: Message| {
        let color = if on { theme::ACCENT } else { theme::MUTED };
        let dot = container(Space::new(4.0, 4.0))
            .style(theme::bar(if on { color } else { Color::TRANSPARENT }, 2.0));
        button(
            container(column![icon(glyph, 22.0, color), dot].spacing(3).align_x(Alignment::Center))
                .center(Length::Fill),
        )
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(message)
    };
    let repeat = match app.queue.repeat() {
        Repeat::One => Icon::RepeatOne,
        Repeat::Off | Repeat::All => Icon::Repeat,
    };
    row![
        switch(Icon::Shuffle, app.queue.shuffle(), Message::ToggleShuffle),
        Space::with_width(Length::Fill),
        skip(Icon::Previous, Message::Previous),
        Space::with_width(Length::Fill),
        play,
        Space::with_width(Length::Fill),
        skip(Icon::Next, Message::Next),
        Space::with_width(Length::Fill),
        switch(repeat, app.queue.repeat() != Repeat::Off, Message::CycleRepeat),
    ]
    .align_y(Alignment::Center)
    .into()
}
