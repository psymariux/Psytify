//! Impaginazione da telefono: un'app pensata per il pollice, non il desktop ristretto. Le quattro
//! schede (Home, Cerca, Libreria, Novità) hanno un titolo grande in cima alla pagina e nessuna
//! barra sopra; le pagine aperte da un'altra (album, artista, impostazioni) hanno solo «Indietro».
//! La pagina va da bordo a bordo sul nero, con la tinta di ciò che si guarda che sfuma dall'alto.
//! In basso il mini-player (un tocco apre il player) e la barra delle schede.
//!
//! Su Android la barra di stato e quella dei gesti stanno sopra l'app: si lascia il loro spazio.
//! In Cerca il campo sta in cima alla pagina e la tastiera del sistema si apre entrando
//! (`App::update`).

pub mod home;
pub mod library;
mod player;

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding, Size};

use super::components::{SUB_LINE, TITLE_LINE, art_of, heart, line};
use super::cover::cover;
use super::icons::{Icon, icon};
use super::{motion, theme};
use crate::app::{App, Message, Screen, phone_tab_of};
use crate::player::seek::progress as progress_of;

/// Lato della copertina nel mini-player.
const MINI_COVER: f32 = 44.0;
/// Margine del mini-player dai bordi dello schermo.
const MINI_MARGIN: f32 = 8.0;
/// Spazio fra il fondo della pagina e il mini-player: l'ultima riga non finisce sotto le barre.
const PAGE_BOTTOM: f32 = 32.0;

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
        return player::view(app, window, insets(app));
    }
    let edges = insets(app);
    let full = (window.width - edges.left - edges.right).max(280.0);
    let inner = full - 2.0 * theme::PHONE_GUTTER;
    let root = phone_tab_of(app.screen) == Some(app.screen);

    // La Home impagina da sé i margini: le file di copertine scorrono fino al bordo dello schermo.
    // Le altre pagine stanno fra i due margini.
    let (body, gutter) = match app.screen {
        Screen::Home => (home::view(app, full), 0.0),
        _ => (super::page(app, inner), theme::PHONE_GUTTER),
    };
    // Una pagina nuova sale di qualche pixel mentre entra, come sul desktop.
    let rise = theme::PAGE_RISE * (1.0 - app.page_motion.value());
    let top = if root { 12.0 } else { 4.0 };
    let content = scrollable(container(body).width(Length::Fill).padding(Padding {
        top: top + rise,
        bottom: PAGE_BOTTOM,
        left: gutter,
        right: gutter,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::scroll);

    let mut screen = column![].height(Length::Fill);
    if !root {
        screen = screen.push(back_bar(app));
    }
    // In Cerca il titolo e il campo restano fermi sopra i risultati, come nelle app da telefono.
    if app.screen == Screen::Search {
        screen = screen.push(search_header(app));
    }
    screen = screen.push(content);
    if let Some(mini) = mini_player(app, full) {
        screen = screen.push(mini);
    }
    let page = container(screen.push(nav(app))).width(Length::Fill).height(Length::Fill).padding(Padding {
        top: edges.top,
        left: edges.left,
        right: edges.right,
        bottom: 0.0,
    });
    // La barra dei gesti resta sul nero della barra delle schede, non sulla tinta.
    container(column![
        container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::phone_wash(super::page_tint(app))),
        container(Space::new(Length::Fill, edges.bottom)).style(theme::phone_nav),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ground)
    .into()
}

/// Titolo grande di una scheda, con le azioni a destra: la prima riga delle quattro schermate
/// principali.
pub fn title_row<'a>(title: &'a str, actions: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut bar =
        row![text(title).size(theme::FS_PHONE_TITLE).font(theme::FONT_SEMIBOLD).width(Length::Fill)]
            .spacing(4)
            .align_y(Alignment::Center)
            .height(theme::TOUCH_TARGET);
    for action in actions {
        bar = bar.push(action);
    }
    bar.into()
}

/// Un'icona da toccare, del lato minimo di un tocco.
pub fn touch_icon<'a>(glyph: Icon, color: iced::Color, on_press: Message) -> Element<'a, Message> {
    button(container(icon(glyph, 24.0, color)).center(Length::Fill))
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(on_press)
        .into()
}

/// Gli amici, dalla Home e dalla Libreria; un punto lime se c'è una richiesta da guardare.
pub fn friends_icon(app: &App) -> Element<'_, Message> {
    let glyph = touch_icon(Icon::Account, theme::TEXT, Message::Navigate(Screen::Friends));
    if app.friend_requests() == 0 {
        return glyph;
    }
    iced::widget::stack![
        glyph,
        container(container(Space::new(7.0, 7.0)).style(theme::bar(theme::ACCENT, 3.5))).padding(Padding {
            top: 9.0,
            left: 29.0,
            ..Padding::ZERO
        }),
    ]
    .into()
}

/// Le impostazioni, dall'angolo in alto a destra della Home e della Libreria.
pub fn settings_icon<'a>() -> Element<'a, Message> {
    touch_icon(Icon::Settings, theme::TEXT, Message::Navigate(Screen::Settings))
}

/// La barra delle pagine aperte da un'altra: solo «Indietro». Una pagina riaperta all'avvio non
/// ha dove tornare: porta alla Home.
fn back_bar(app: &App) -> Element<'_, Message> {
    let back = if app.can_go_back() { Message::GoBack } else { Message::Navigate(Screen::Home) };
    container(touch_icon(Icon::ChevronLeft, theme::TEXT, back))
        .height(theme::PHONE_TOP_HEIGHT)
        .padding(Padding { left: 4.0, ..Padding::ZERO })
        .align_y(Alignment::Center)
        .into()
}

/// Cerca: il titolo e il campo, fermi sopra i risultati.
fn search_header(app: &App) -> Element<'_, Message> {
    column![title_row("Cerca", Vec::new()), super::top_bar::search_field(app, Length::Fill, false),]
        .spacing(10)
        .padding(Padding { top: 12.0, bottom: 12.0, left: theme::PHONE_GUTTER, right: theme::PHONE_GUTTER })
        .into()
}

/// Le quattro schede. Quella accesa ha l'icona piena e il testo bianco; Novità ha un punto lime
/// quando ci sono uscite da ascoltare.
fn nav(app: &App) -> Element<'_, Message> {
    let tab = app.phone_tab();
    let unread = app.unread_news() > 0;
    let items = [
        (Icon::Home, Icon::HomeFill, "Home", Screen::Home),
        (Icon::Search, Icon::SearchFill, "Cerca", Screen::Search),
        (Icon::Library, Icon::LibraryFill, "Libreria", Screen::Collection),
        (Icon::Bell, Icon::BellFill, "Novità", Screen::News),
    ];
    let buttons = items.into_iter().map(|(idle, filled, label, screen)| {
        let active = tab == screen;
        let color = if active { theme::TEXT } else { theme::FAINT };
        let mut glyph = row![icon(if active { filled } else { idle }, 24.0, color)];
        if screen == Screen::News && unread {
            glyph = glyph.push(
                container(Space::new(7.0, 7.0))
                    .style(theme::bar(theme::ACCENT, 3.5))
                    .padding(Padding { bottom: 16.0, ..Padding::ZERO }),
            );
        }
        button(
            container(
                column![glyph, text(label).size(11.0).font(theme::FONT_MEDIUM).color(color)]
                    .spacing(4)
                    .align_x(Alignment::Center),
            )
            .center(Length::Fill),
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

/// Il brano in corso sopra le schede, sulla tinta della sua copertina: un tocco apre il player.
/// Sul fondo, una linea sottile con l'avanzamento.
fn mini_player(app: &App, window: f32) -> Option<Element<'_, Message>> {
    let track = app.queue.current()?;
    let outer = window - 2.0 * MINI_MARGIN;
    let text_width = outer - 2.0 * 8.0 - MINI_COVER - 3.0 * 10.0 - 2.0 * theme::TOUCH_TARGET;
    let glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    let play = button(container(icon(glyph, 24.0, theme::TEXT)).center(Length::Fill))
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
    .spacing(10)
    .align_y(Alignment::Center);
    let open = button(body)
        .padding(Padding { top: 0.0, bottom: 0.0, left: 8.0, right: 8.0 })
        .height(Length::Fill)
        .style(theme::bare)
        .on_press(Message::ToggleFocus);

    let fraction = progress_of(app.shown_position(), track.duration()).clamp(0.0, 1.0);
    let track_width = outer - 2.0 * 8.0;
    let progress = container(
        container(Space::new(Length::Fixed(track_width * fraction), 2.0)).style(theme::bar(theme::TEXT, 1.0)),
    )
    .width(track_width)
    .style(theme::bar(motion::fade(theme::TEXT, 0.2), 1.0));

    let tint = app.covers.tint_of(art_of(track));
    Some(
        container(
            container(column![open, container(progress).padding(Padding { left: 8.0, ..Padding::ZERO })])
                .padding(Padding { bottom: 4.0, ..Padding::ZERO })
                .height(theme::PHONE_MINI_HEIGHT)
                .width(Length::Fill)
                .style(theme::mini_player(tint)),
        )
        .padding(Padding { top: 0.0, bottom: 4.0, left: MINI_MARGIN, right: MINI_MARGIN })
        .into(),
    )
}
