//! Avviso in basso, sopra il player: download salvati, playlist, errori. Entra salendo e
//! schiarendosi; un avviso informativo se ne va da solo dopo `TOAST_INFO_TIME`, un errore resta
//! finché non si chiude. Sta sopra al contenuto senza coprirne i clic (`stack` di Iced).

use iced::widget::{Space, container, row};
use iced::{Alignment, Element, Length, Padding};

use super::components::{BODY_LINE, ICON_BUTTON, Line, icon_button, line};
use super::icons::{Icon, icon};
use super::motion::fade;
use super::theme;
use crate::app::{App, Message, Notice};

/// Spazio di icona, distanze e padding attorno al testo dell'avviso.
const CHROME: f32 = 14.0 + 12.0 * 2.0 + 16.0 * 2.0 + ICON_BUTTON;

/// Il livello dell'avviso, a tutta area; `None` se non ce n'è.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let toast = app.toast.as_ref()?;
    let shown = toast.motion.value();
    let (glyph, color, message) = match &toast.notice {
        Notice::Info(message) => (Icon::Download, theme::ACCENT, message),
        Notice::Error(message) => (Icon::Close, theme::NEGATIVE, message),
    };
    let label = Line { color: Some(fade(theme::TEXT, shown)), ..BODY_LINE };
    let body = container(
        row![
            icon(glyph, 14.0, fade(color, shown)),
            line(message, theme::TOAST_WIDTH - CHROME, label),
            icon_button(Icon::Close, "Chiudi avviso", false, Some(Message::DismissNotice)),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(Padding { top: 6.0, bottom: 6.0, left: 16.0, right: 8.0 })
    // Larghezza fissa: la riga di testo è `Fill` e allargherebbe l'avviso a tutta l'area.
    .width(theme::TOAST_WIDTH)
    .style(theme::toast(shown));
    // Sopra la barra del player (e sopra i controlli della vista immersiva, alti uguali): coprirla
    // nasconderebbe play e avanzamento proprio quando l'errore chiede di riprovare.
    let rise = theme::NOW_PLAYING_HEIGHT + theme::TOAST_GAP - theme::TOAST_RISE * (1.0 - shown);
    Some(
        container(iced::widget::column![Space::with_height(Length::Fill), body])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .padding(Padding { bottom: rise.max(0.0), ..Padding::ZERO })
            .into(),
    )
}
