//! Il cassetto del telefono: dall'avatar in cima alla Home si apre da destra, sopra la pagina.
//! Raccoglie il profilo, le novità, gli amici, la jam e le impostazioni; chi non è collegato vede
//! l'invito ad accedere al posto del nome.

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, Size};

use super::{avatar, profile_target};
use crate::app::{Account, App, Message, Screen};
use crate::ui::components::{divider_line, fit};
use crate::ui::icons::{Icon, icon};
use crate::ui::{motion, theme};

/// Larghezza massima del cassetto; sui telefoni stretti lascia comunque una fascia di pagina a sinistra.
const MAX_WIDTH: f32 = 340.0;
/// Quanto della finestra può coprire.
const SHARE: f32 = 0.84;
const AVATAR: f32 = 64.0;
const PAD: f32 = 20.0;

/// Il livello del cassetto, a tutta finestra; `None` se è chiuso.
pub fn view(app: &App, window: Size) -> Option<Element<'_, Message>> {
    let since = app.drawer?;
    let shown = motion::appear(since, theme::MOTION_PANEL);
    let width = (window.width * SHARE).min(MAX_WIDTH);
    let (top, bottom) = app.insets.map_or((0.0, 0.0), |edges| (edges.top, edges.bottom));

    let mut items = column![
        item(Icon::Bell, "Novità", app.unread_news() > 0, None, Screen::News),
        item(Icon::Account, "Amici", app.friend_requests() > 0, requests(app), Screen::Friends),
        entry(Icon::People, "Jam", None, Message::OpenJam),
        item(Icon::Settings, "Impostazioni", false, None, Screen::Settings),
    ]
    .width(Length::Fill);
    if matches!(app.account, Account::SignedIn(_)) {
        items = items.push(divider_line()).push(entry(Icon::Close, "Esci", None, Message::SignOut));
    }

    let panel = container(
        column![
            container(profile(app, width - 2.0 * PAD)).padding([6.0, PAD]),
            divider_line(),
            scrollable(items).height(Length::Fill).style(theme::scroll),
        ]
        .spacing(12),
    )
    .width(width)
    .height(Length::Fill)
    .padding(Padding { top: top + 16.0, bottom: bottom + 12.0, left: 0.0, right: 0.0 })
    .style(theme::drawer(shown));

    Some(
        stack![
            mouse_area(container(Space::new(Length::Fill, Length::Fill)).style(theme::scrim(shown)))
                .on_press(Message::CloseDrawer)
                .on_right_press(Message::CloseDrawer),
            // Il cassetto scorre da fuori schermo: a sinistra, uno spazio che si accorcia.
            row![Space::with_width(Length::Fixed(window.width - width * shown)), opaque(panel)],
        ]
        .into(),
    )
}

/// Avatar grande e nome: un tocco apre il profilo, o l'accesso se non si è collegati.
fn profile(app: &App, text_width: f32) -> Element<'_, Message> {
    let (name, detail) = match app.signed_in_profile() {
        Some(profile) => (profile.display_name.clone(), "Vedi il tuo profilo".to_string()),
        None => match app.account {
            Account::SignedIn(_) => ("Il tuo account".to_string(), "Scegli un nome utente".to_string()),
            _ => ("Accedi".to_string(), "Per amici, jam e profilo".to_string()),
        },
    };
    let label = text_width - AVATAR - 16.0;
    button(
        row![
            avatar(app, AVATAR),
            column![
                text(fit(&name, label, theme::FS_TITLE)).size(theme::FS_TITLE).font(theme::FONT_SEMIBOLD),
                text(fit(&detail, label, theme::FS_BODY)).size(theme::FS_BODY).color(theme::MUTED),
            ]
            .spacing(4),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .width(Length::Fill)
    .style(theme::bare)
    .on_press(Message::DrawerGo(Box::new(profile_target(app))))
    .into()
}

/// Le richieste d'amicizia da guardare, scritte a destra di «Amici».
fn requests(app: &App) -> Option<String> {
    let count = app.friend_requests();
    (count > 0).then(|| count.to_string())
}

/// Una voce che porta a una schermata; un punto lime dice che c'è qualcosa di nuovo.
fn item(
    glyph: Icon,
    label: &str,
    dot: bool,
    count: Option<String>,
    screen: Screen,
) -> Element<'static, Message> {
    let trailing = count.or_else(|| dot.then(String::new));
    entry(glyph, label, trailing, Message::Navigate(screen))
}

/// Una riga del cassetto: icona, nome e, a destra, un numero (o un punto se è vuoto).
fn entry(glyph: Icon, label: &str, trailing: Option<String>, message: Message) -> Element<'static, Message> {
    let mut content = row![
        icon(glyph, 24.0, theme::MUTED),
        text(label.to_owned()).size(theme::FS_SHEET).font(theme::FONT_MEDIUM),
        Space::with_width(Length::Fill),
    ]
    .spacing(18)
    .align_y(Alignment::Center);
    if let Some(count) = trailing {
        content = content.push(if count.is_empty() {
            container(Space::new(8.0, 8.0)).style(theme::bar(theme::ACCENT, 4.0)).into()
        } else {
            Element::from(text(count).size(theme::FS_BODY).font(theme::FONT_SEMIBOLD).color(theme::ACCENT))
        });
    }
    button(container(content).height(Length::Fill).align_y(iced::alignment::Vertical::Center))
        .width(Length::Fill)
        .height(theme::SHEET_ITEM)
        .padding([0.0, PAD])
        .style(theme::row(false))
        .on_press(Message::DrawerGo(Box::new(message)))
        .into()
}
