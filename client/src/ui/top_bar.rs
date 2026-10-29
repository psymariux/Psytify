//! Barra superiore (56px) sul nero: marchio, Home e ricerca al centro, account e i tre controlli
//! della finestra a destra. Il resto della barra trascina la finestra (doppio clic: ingrandisce).

use iced::widget::{Space, button, container, mouse_area, row, text, text_input};
use iced::{Alignment, Element, Length, Theme, alignment};

use super::components::{icon_button, muted, primary_button, small};
use super::icons::{Icon, icon, mark};
use super::theme;
use crate::app::{Account, App, Message, SEARCH_INPUT, Screen};

const SEARCH_WIDTH: f32 = 440.0;
const ROUND: f32 = 40.0;

/// Il campo di ricerca: sul desktop sta nella barra, sul telefono in cima alla pagina Cerca.
/// `hint`: a campo vuoto ricorda la scorciatoia Ctrl F (solo dove c'è una tastiera).
pub fn search_field(app: &App, width: impl Into<Length>, hint: bool) -> Element<'_, Message> {
    let input = text_input("Cosa vuoi ascoltare?", &app.search.query)
        .id(text_input::Id::new(SEARCH_INPUT))
        .on_input(Message::SearchInput)
        .size(theme::FS_BODY + 1.0)
        .padding([8, 0])
        .style(bare_input);
    let clear: Element<'_, Message> = if !app.search.query.is_empty() {
        button(icon(Icon::Close, 14.0, theme::MUTED))
            .padding(4)
            .style(theme::icon_plain)
            .on_press(Message::SearchInput(String::new()))
            .into()
    } else if hint {
        small("Ctrl F").into()
    } else {
        Space::with_width(0).into()
    };
    let searching = app.screen == Screen::Search;
    container(
        row![icon(if searching { Icon::SearchFill } else { Icon::Search }, 18.0, theme::MUTED), input, clear]
            .spacing(12)
            .align_y(Alignment::Center),
    )
    .padding([0, 16])
    .height(ROUND)
    .width(width)
    .align_y(alignment::Vertical::Center)
    .style(theme::search_field(searching))
    .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let brand = container(
        row![mark(24.0), text("Psytify").size(theme::FS_SECTION).font(theme::FONT_SEMIBOLD)]
            .spacing(10)
            .align_y(Alignment::Center),
    )
    .width(theme::SIDEBAR_WIDTH + theme::OUTER_GAP)
    .padding([0, 16]);

    let on_home = app.screen == Screen::Home;
    let home = button(
        container(icon(if on_home { Icon::HomeFill } else { Icon::Home }, 20.0, theme::TEXT))
            .center(Length::Fill),
    )
    .width(ROUND)
    .height(ROUND)
    .padding(0)
    .style(theme::round_button(on_home))
    .on_press(Message::Navigate(Screen::Home));

    let search = search_field(app, SEARCH_WIDTH, true);

    // Indietro e avanti, come in un browser (anche Alt+← e Alt+→, e i tasti laterali del mouse).
    let history = row![
        icon_button(
            Icon::ChevronLeft,
            "Indietro (Alt+←)",
            false,
            app.can_go_back().then_some(Message::GoBack)
        ),
        icon_button(
            Icon::ChevronRight,
            "Avanti (Alt+→)",
            false,
            app.can_go_forward().then_some(Message::GoForward)
        ),
    ]
    .spacing(4);
    let bar = row![
        brand,
        history,
        Space::with_width(Length::Fill),
        row![home, search].spacing(8).align_y(Alignment::Center),
        Space::with_width(Length::Fill),
        container(account(app)).padding([0, 8]),
        window_controls(app),
    ]
    .height(theme::TOP_BAR_HEIGHT)
    .align_y(Alignment::Center);
    // Ciò che non è un pulsante trascina la finestra.
    mouse_area(bar).on_press(Message::TitleBarPressed).into()
}

/// Riduci a icona, schermo intero, chiudi.
pub(super) fn window_controls(app: &App) -> Element<'_, Message> {
    let fullscreen = if app.fullscreen {
        control(Icon::FullscreenExit, Message::ToggleFullscreen, false)
    } else {
        control(Icon::Fullscreen, Message::ToggleFullscreen, false)
    };
    row![
        control(Icon::Minimize, Message::Minimize, false),
        fullscreen,
        control(Icon::Close, Message::Quit, true)
    ]
    .spacing(2)
    .padding([0, 10])
    .align_y(Alignment::Center)
    .into()
}

fn control(glyph: Icon, message: Message, close: bool) -> Element<'static, Message> {
    button(container(icon(glyph, 14.0, theme::TEXT)).center(Length::Fill))
        .width(40)
        .height(32)
        .padding(0)
        .style(theme::window_control(close))
        .on_press(message)
        .into()
}

/// L'account: accedere se si è ospiti, altrimenti aprire le impostazioni dove c'è il resto.
/// Senza login configurato non si mostra nulla.
fn account(app: &App) -> Element<'_, Message> {
    match &app.account {
        Account::Unavailable => Space::with_width(0.0).into(),
        Account::SignedOut => primary_button("Accedi", None, Message::SignIn),
        Account::Working => muted("Accesso in corso…").into(),
        Account::SignedIn(_) => {
            button(container(icon(Icon::Account, 18.0, theme::TEXT)).center(Length::Fill))
                .width(ROUND)
                .height(ROUND)
                .padding(0)
                .style(theme::round_button(app.screen == Screen::Settings))
                .on_press(Message::Navigate(Screen::Settings))
                .into()
        }
    }
}

/// Il campo vive dentro la pillola: niente fondo né bordo propri.
fn bare_input(theme_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: iced::Background::Color(iced::Color::TRANSPARENT),
        border: iced::Border::default(),
        ..theme::search_input(theme_, status)
    }
}
