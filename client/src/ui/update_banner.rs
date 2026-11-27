//! Il banner dell'aggiornamento: una fascia sotto la barra alta (desktop) o in cima alla pagina
//! (telefono) che dice quando c'è una versione nuova, scarica, e si può chiudere. Sta nella vista
//! perché una novità così non deve nascondersi in Impostazioni.

use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use super::components::{outlined_button, primary_button, step_bar};
use super::icons::{Icon, icon};
use super::theme;
use crate::app::{App, Message, UpdateState};

/// Il banner, se c'è qualcosa da dire. `phone` lo fa più alto, con il bersaglio per il dito.
pub fn view(app: &App, phone: bool) -> Option<Element<'_, Message>> {
    let (label, action): (String, Option<Element<'_, Message>>) = match app.downloads.update.banner()? {
        UpdateState::Offered(offer) => (
            format!("Nuova versione {} disponibile", offer.tag),
            Some(primary_button("Scarica", None, Message::UpdateRequested)),
        ),
        // La barra a dieci scatti dei download, con i punti che il core pubblica (BACKLOG 38).
        state @ UpdateState::Downloading { tag, .. } => match state.percent() {
            Some(percent) => (format!("Scarico {tag}… {percent}%"), Some(step_bar(Some(percent)))),
            None => (format!("Scarico {tag}…"), None),
        },
        UpdateState::Downloaded(_) if cfg!(target_os = "android") => {
            ("Scaricato: conferma l'installazione".to_owned(), None)
        }
        UpdateState::Downloaded(path) => (
            format!("Scaricato in {}", path.display()),
            Some(outlined_button("Apri cartella", None, Some(Message::UpdateReveal))),
        ),
    };
    let close = button(container(icon(Icon::Close, 0.0, theme::MUTED)).center(Length::Fill))
        .width(if phone { theme::TOUCH_TARGET } else { 32.0 })
        .height(if phone { theme::TOUCH_TARGET } else { 32.0 })
        .padding(0)
        .style(theme::icon_plain)
        .on_press(Message::UpdateDismiss);
    let mut bar = row![
        icon(Icon::Download, 0.0, theme::TEXT),
        container(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM)).width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(action) = action {
        bar = bar.push(action);
    }
    bar = bar.push(close);
    let gutter = if phone { theme::PHONE_GUTTER } else { theme::OUTER_GAP };
    Some(
        container(
            container(bar)
                .padding(Padding { top: 6.0, bottom: 6.0, left: 16.0, right: 6.0 })
                .width(Length::Fill)
                .style(theme::update_banner),
        )
        .padding(Padding { left: gutter, right: gutter, top: if phone { 8.0 } else { 0.0 }, bottom: 8.0 })
        .width(Length::Fill)
        .into(),
    )
}

/// La riga di Impostazioni → Informazioni: com'è andato l'ultimo controllo.
pub fn status_line(app: &App) -> &'static str {
    use crate::app::UpdateStatus;
    match app.downloads.update.status {
        UpdateStatus::Checking => "Controllo in corso…",
        UpdateStatus::Current => "Hai l'ultima versione.",
        UpdateStatus::Failed => "Controllo non riuscito. Riprova più tardi.",
        UpdateStatus::Idle => "Controlla se c'è una versione nuova.",
    }
}
