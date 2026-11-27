//! Componenti visivi, nell'identità monocromatica pixel di `docs/design/`. Regioni a filo, divise da
//! una linea da 1 AP: la navigazione e la libreria a sinistra, la pagina al centro su tela nera (piatta,
//! senza tinte), a destra il pannello «In riproduzione». Sopra c'è la barra con i controlli della
//! finestra, sotto quella del player. Menu, finestre di dialogo, avvisi e lo splash d'ingresso stanno
//! su livelli propri.

pub mod components;
pub mod context_menu;
pub mod cover;
pub mod cursor;
pub mod dialog;
pub mod focus;
pub mod gesture;
pub mod icons;
pub mod lyrics;
pub mod motion;
pub mod now_playing;
pub mod pages;
pub mod phone;
pub mod pointer;
pub mod shelf;
pub mod shortcuts;
pub mod side_panel;
pub mod sidebar;
pub mod splash;
pub mod theme;
pub mod toast;
pub mod top_bar;
pub mod touch_seek;
pub mod update_banner;
pub mod volume_side;

use iced::widget::{Space, column, container, mouse_area, responsive, row, scrollable, stack};
use iced::{Element, Length, Padding, Size, mouse};

use crate::app::{App, Message, ResizeEdge, Screen};

/// Spazio riservato alla barra di scorrimento verticale dell'area centrale.
const SCROLLBAR: f32 = 10.0;
/// Larghezza minima su cui impaginare il contenuto, anche se la finestra è più stretta.
const MIN_CONTENT: f32 = 320.0;
/// Spessore delle maniglie con cui si ridimensiona la finestra, e lato dell'angolo.
const GRIP: f32 = 6.0;
const CORNER: f32 = 16.0;

/// La vista conosce la larghezza reale della finestra (`responsive`): serve a tagliare i testi
/// lunghi, perché Iced 0.13 non sa tenere un testo su una riga sola.
pub fn view(app: &App) -> Element<'_, Message> {
    responsive(move |size| layers(app, size)).into()
}

/// Il contenuto e, sopra, ciò che galleggia: maniglie, avviso, menu, finestra di dialogo.
fn layers(app: &App, window: Size) -> Element<'_, Message> {
    let mut layers = if crate::app::is_phone() {
        stack![phone::view(app, window)]
    } else {
        let mut layers = stack![frame(app, window)];
        if let Some(focus) = focus::view(app) {
            layers = layers.push(focus);
        }
        if !app.fullscreen {
            layers = layers.push(grips());
        }
        layers
    };
    if let Some(toast) = toast::view(app) {
        layers = layers.push(toast);
    }
    if crate::app::is_phone()
        && let Some(drawer) = phone::drawer::view(app, window)
    {
        layers = layers.push(drawer);
    }
    if let Some(menu) = context_menu::view(app) {
        layers = layers.push(menu);
    }
    if let Some(dialog) = dialog::view(app) {
        layers = layers.push(dialog);
    }
    if let Some(shortcuts) = shortcuts::view(app) {
        layers = layers.push(shortcuts);
    }
    // Lo splash d'ingresso copre tutto, finché sfuma.
    if let Some(splash) = splash::view(app) {
        layers = layers.push(splash);
    }
    layers.into()
}

fn frame(app: &App, window: Size) -> Element<'_, Message> {
    // Il pannello entra da destra: la sua larghezza segue il movimento, il contenuto resta fermo.
    let panel = app.panel_motion.value();
    let drawer = panel * (theme::SIDE_PANEL_WIDTH + theme::OUTER_GAP);
    let center = window.width - theme::SIDEBAR_WIDTH - theme::OUTER_GAP - drawer;
    let width = (center - 2.0 * theme::GUTTER - SCROLLBAR).max(MIN_CONTENT);

    let page = page(app, width);
    // Una pagina nuova sale di qualche pixel mentre entra.
    let rise = theme::PAGE_RISE * (1.0 - app.page_motion.value());
    let content = scrollable(container(page).width(Length::Fill).padding(Padding {
        top: 28.0 + rise,
        bottom: 28.0,
        left: theme::GUTTER,
        right: theme::GUTTER + SCROLLBAR,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::scroll);
    let center_panel =
        container(content).width(Length::Fill).height(Length::Fill).clip(true).style(theme::canvas);

    let mut middle = row![sidebar::view(app), center_panel].spacing(theme::OUTER_GAP);
    if drawer >= 1.0 {
        let side = container(side_panel::view(app))
            .width(Length::Fixed((drawer - theme::OUTER_GAP).max(0.0)))
            .height(Length::Fill)
            .clip(true)
            .style(theme::panel_box);
        middle = middle.push(side);
    }

    // Le regioni stanno a filo: lo spazio tra l'una e l'altra lascia vedere la finestra, che ha il colore
    // delle linee.
    let mut screen = column![top_bar::view(app)].spacing(theme::OUTER_GAP);
    if let Some(banner) = update_banner::view(app, false) {
        screen = screen.push(banner);
    }
    screen = screen.push(middle.height(Length::Fill));
    container(screen.push(now_playing::view(app, window.width)))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ground)
        .into()
}

/// La pagina della schermata corrente, impaginata su `width`.
pub(crate) fn page(app: &App, width: f32) -> Element<'_, Message> {
    match app.screen {
        Screen::Home => pages::home::view(app, width),
        Screen::Search => pages::search::view(app, width),
        Screen::Library => pages::library::view(app, width),
        Screen::Album(id) => pages::album::view(app, id, width),
        Screen::Playlist(id) => pages::playlist::view(app, id, width),
        Screen::Artist(id) => pages::artist::view(app, id, width),
        Screen::Settings => pages::settings::view(app, width),
        Screen::Jam => pages::jam::view(app, width),
        Screen::Liked => pages::liked::view(app, width),
        Screen::Downloads => pages::downloads::view(app, width),
        Screen::News => pages::news::view(app, width),
        Screen::SourcePlaylist => pages::source_playlist::view(app, width),
        Screen::Collection => phone::library::view(app, width),
        Screen::SignIn => pages::sign_in::view(app, width),
        Screen::Friends => pages::friends::view(app, width),
        Screen::Profile => pages::profile::view(app, width),
        Screen::FriendPlaylist => pages::profile::playlist_view(app, width),
    }
}

/// Le maniglie ai bordi destro e basso e all'angolo: la finestra non ha cornice di sistema.
fn grips<'a>() -> Element<'a, Message> {
    let grip = |width: Length, height: Length, edge: ResizeEdge, interaction: mouse::Interaction| {
        mouse_area(Space::new(width, height)).on_press(Message::ResizeStart(edge)).interaction(interaction)
    };
    let right = row![
        Space::with_width(Length::Fill),
        grip(Length::Fixed(GRIP), Length::Fill, ResizeEdge::Right, mouse::Interaction::ResizingHorizontally),
    ]
    .height(Length::Fill);
    let bottom = row![
        grip(Length::Fill, Length::Fixed(GRIP), ResizeEdge::Bottom, mouse::Interaction::ResizingVertically),
        grip(
            Length::Fixed(CORNER),
            Length::Fixed(GRIP),
            ResizeEdge::Corner,
            mouse::Interaction::ResizingDiagonallyDown
        ),
    ]
    .height(GRIP);
    column![right, bottom].width(Length::Fill).height(Length::Fill).into()
}
