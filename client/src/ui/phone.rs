//! Impaginazione da telefono: la stessa app del desktop, ripiegata su una colonna. In alto
//! «Indietro» e il marchio; al centro la pagina a tutta larghezza con la tinta della sua copertina;
//! in basso il mini-player (un tocco apre il player) e la barra di navigazione con le quattro
//! schermate principali; le impostazioni si aprono dall'icona in alto a destra. Il player a schermo intero ha la copertina che respira, i controlli e,
//! sotto, il testo sincronizzato o la coda.
//!
//! Su Android la barra di stato e quella dei gesti stanno sopra l'app: si lascia il loro spazio.
//! In Cerca il campo sta in cima alla pagina e la tastiera del sistema si apre entrando
//! (`App::update`).

use std::time::Duration;

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding, Size, alignment};

use super::components::{H1_LINE, MUTED_LINE, SUB_LINE, TITLE_LINE, art_of, heart, icon_button, line};
use super::cover::cover;
use super::focus::breathing_cover;
use super::icons::{Icon, icon, mark};
use super::{lyrics, motion, now_playing, side_panel, theme};
use crate::app::{App, Message, Screen, SideTab};
use crate::player::seek::progress as progress_of;

/// Lato della copertina nel mini-player.
const MINI_COVER: f32 = 44.0;
/// Margine del player a schermo intero.
const PLAYER_GUTTER: f32 = 24.0;

/// Lo spazio delle barre del sistema: quello che Android dice (barra di stato, gesti o tre
/// pulsanti, notch); finché non lo dice, una stima. Sul desktop niente.
fn insets(app: &App) -> Padding {
    match app.insets {
        Some(found) => Padding { top: found.top, right: found.right, bottom: found.bottom, left: found.left },
        None if cfg!(target_os = "android") => {
            Padding { top: theme::PHONE_STATUS_INSET, bottom: theme::PHONE_GESTURE_INSET, ..Padding::ZERO }
        }
        None => Padding::ZERO,
    }
}

pub fn view(app: &App, window: Size) -> Element<'_, Message> {
    if app.focus && app.queue.current().is_some() {
        return player(app, window);
    }
    let edges = insets(app);
    let width = (window.width - edges.left - edges.right - 2.0 * theme::PHONE_GUTTER).max(240.0);
    // Una pagina nuova sale di qualche pixel mentre entra, come sul desktop.
    let rise = theme::PAGE_RISE * (1.0 - app.page_motion.value());
    let content = scrollable(container(super::page(app, width)).width(Length::Fill).padding(Padding {
        top: 16.0 + rise,
        bottom: 24.0,
        left: theme::PHONE_GUTTER,
        right: theme::PHONE_GUTTER,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::scroll);
    let page = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(theme::tinted_panel(super::page_tint(app)));

    let mut screen = column![top_bar(app)].height(Length::Fill);
    // In Cerca il campo resta fermo sopra i risultati, come nelle app da telefono.
    if app.screen == Screen::Search {
        screen =
            screen.push(container(super::top_bar::search_field(app, Length::Fill, false)).padding(Padding {
                top: 0.0,
                bottom: 12.0,
                left: theme::PHONE_GUTTER,
                right: theme::PHONE_GUTTER,
            }));
    }
    screen = screen.push(page);
    if let Some(mini) = mini_player(app, window.width - edges.left - edges.right) {
        screen = screen.push(mini);
    }
    container(screen.push(nav(app)))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(edges)
        .style(theme::ground)
        .into()
}

/// «Indietro» (se c'è dove tornare), il marchio e, a destra, le impostazioni.
fn top_bar(app: &App) -> Element<'_, Message> {
    let mut bar = row![].spacing(10).padding(Padding { left: 4.0, right: 4.0, ..Padding::ZERO });
    if app.can_go_back() {
        bar = bar.push(icon_button(Icon::ChevronLeft, "Indietro", false, Some(Message::GoBack)));
    } else {
        bar = bar.push(Space::with_width(theme::PHONE_GUTTER - 14.0));
    }
    let on_settings = app.screen == Screen::Settings;
    let settings = button(
        container(icon(
            if on_settings { Icon::SettingsFill } else { Icon::Settings },
            22.0,
            if on_settings { theme::TEXT } else { theme::MUTED },
        ))
        .center(Length::Fill),
    )
    .width(theme::TOUCH_TARGET)
    .height(theme::TOUCH_TARGET)
    .padding(0)
    .style(theme::bare)
    .on_press(Message::Navigate(Screen::Settings));
    bar.push(mark(22.0))
        .push(text("Psytify").size(theme::FS_SECTION).font(theme::FONT_SEMIBOLD))
        .push(Space::with_width(Length::Fill))
        .push(settings)
        .height(theme::PHONE_TOP_HEIGHT)
        .align_y(Alignment::Center)
        .into()
}

/// Le quattro schermate principali (le impostazioni stanno in alto a destra). La voce attiva ha l'icona piena, il testo bianco e una tacca
/// lime sopra.
fn nav(app: &App) -> Element<'_, Message> {
    let items = [
        (Icon::Home, Icon::HomeFill, "Home", Screen::Home, matches!(app.screen, Screen::Home)),
        (Icon::Search, Icon::SearchFill, "Cerca", Screen::Search, matches!(app.screen, Screen::Search)),
        (
            Icon::Library,
            Icon::LibraryFill,
            "Catalogo",
            Screen::Library,
            matches!(app.screen, Screen::Library | Screen::Album(_) | Screen::Artist(_)),
        ),
        (Icon::Bell, Icon::BellFill, "Novità", Screen::News, matches!(app.screen, Screen::News)),
    ];
    let buttons = items.into_iter().map(|(idle, filled, label, screen, active)| {
        let color = if active { theme::TEXT } else { theme::FAINT };
        let notch: Element<'_, Message> = if active {
            container(Space::new(16, 3)).style(theme::bar(theme::ACCENT, 1.5)).into()
        } else {
            Space::new(16, 3).into()
        };
        button(
            container(
                column![
                    notch,
                    icon(if active { filled } else { idle }, 22.0, color),
                    text(label).size(11.0).font(theme::FONT_MEDIUM).color(color),
                ]
                .spacing(4)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill),
        )
        .width(Length::Fill)
        .height(theme::PHONE_NAV_HEIGHT)
        .padding(0)
        .style(theme::bare)
        .on_press(Message::Navigate(screen))
        .into()
    });
    container(row(buttons)).width(Length::Fill).style(theme::phone_nav).into()
}

/// Il brano in corso sopra la barra di navigazione: un tocco apre il player. Sotto, una linea
/// sottile con l'avanzamento.
fn mini_player(app: &App, window: f32) -> Option<Element<'_, Message>> {
    let track = app.queue.current()?;
    let outer = window - 2.0 * 8.0;
    let text_width = outer - 2.0 * 10.0 - MINI_COVER - 12.0 - 2.0 * theme::TOUCH_TARGET - 8.0;
    let glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    let play = button(container(icon(glyph, 22.0, theme::TEXT)).center(Length::Fill))
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(Message::TogglePlay);
    let body = row![
        cover(&app.covers, art_of(track), MINI_COVER),
        column![line(&track.title, text_width, TITLE_LINE), line(&track.artist.name, text_width, SUB_LINE)]
            .spacing(2)
            .width(Length::Fill),
        heart(app, track, true),
        play,
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let open = button(body).padding(0).style(theme::bare).on_press(Message::ToggleFocus);

    let fraction = progress_of(app.shown_position(), track.duration()).clamp(0.0, 1.0);
    let track_width = outer - 2.0 * 10.0;
    let progress = container(
        container(Space::new(Length::Fixed(track_width * fraction), 2.0)).style(theme::bar(theme::TEXT, 1.0)),
    )
    .width(track_width)
    .style(theme::bar(motion::fade(theme::TEXT, 0.2), 1.0));

    let tint = app.covers.tint_of(art_of(track));
    Some(
        container(
            container(column![open, progress].spacing(8))
                .padding(Padding { top: 8.0, bottom: 6.0, left: 10.0, right: 10.0 })
                .height(theme::PHONE_MINI_HEIGHT)
                .width(Length::Fill)
                .style(theme::mini_player(tint)),
        )
        .padding([0.0, 8.0])
        .into(),
    )
}

/// Il player a schermo intero: copertina, titolo, avanzamento, controlli e, sotto, testo o coda.
fn player<'a>(app: &'a App, window: Size) -> Element<'a, Message> {
    let Some(track) = app.queue.current() else {
        return Space::new(Length::Fill, Length::Fill).into();
    };
    let edges = insets(app);
    let width = (window.width - edges.left - edges.right - 2.0 * PLAYER_GUTTER).max(200.0);
    let header = row![
        icon_button(Icon::ChevronDown, "Chiudi il player", false, Some(Message::ToggleFocus)),
        container(text("In riproduzione").size(theme::FS_SMALL).font(theme::FONT_MEDIUM).color(theme::MUTED))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        Space::with_width(super::components::ICON_BUTTON),
    ]
    .align_y(Alignment::Center);

    let side = (window.height * 0.34).min(width).max(120.0);
    let title_width = width - super::components::ICON_BUTTON - 8.0;
    let title = row![
        column![line(&track.title, title_width, H1_LINE), line(&track.artist.name, title_width, MUTED_LINE)]
            .spacing(4)
            .width(Length::Fill),
        heart(app, track, true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let queue_open = app.side_tab == SideTab::Queue;
    let choice = |label: &'static str, tab: SideTab, on: bool| -> Element<'a, Message> {
        button(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM))
            .padding([6, 16])
            .style(theme::pill(on))
            .on_press(Message::SideTab(tab))
            .into()
    };
    let tabs =
        row![choice("Testo", SideTab::Lyrics, !queue_open), choice("Coda", SideTab::Queue, queue_open)]
            .spacing(8);
    let below: Element<'a, Message> =
        if queue_open { side_panel::queue_body(app) } else { lyrics::view(app) };

    let page = column![
        header,
        container(breathing_cover(app, track, side))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        title,
        now_playing::progress(app),
        container(now_playing::controls(app)).width(Length::Fill).align_x(alignment::Horizontal::Center),
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
            top: edges.top + 8.0,
            bottom: edges.bottom + 12.0,
            left: edges.left + PLAYER_GUTTER,
            right: edges.right + PLAYER_GUTTER,
        })
        .style(theme::aurora_full(tint, drift))
        .into()
}
