//! Videoclip del brano in riproduzione: il video senza controlli propri, a tutto schermo
//! (`stage`) o in piccolo nel pannello «In riproduzione» (`mini`). Pausa, seek e cambio brano
//! passano dalla barra di riproduzione; l'audio resta quello del player. Sopra il video, se c'è, il
//! testo sincronizzato come sottotitolo (`caption`).

use iced::widget::{column, container, text};
use iced::{Alignment, Element, Length, alignment};

use crate::app::{App, Load, Message};
use crate::ui::components::muted;
use crate::ui::motion::fade;
use crate::ui::theme;

/// La riga in corso del testo sincronizzato e, con `next`, la successiva, centrate su uno sfondo
/// scuro che le tiene leggibili su qualsiasi immagine. `None` senza testo sincronizzato o prima
/// della prima riga.
pub fn caption(app: &App, width: f32, size: f32, next: bool) -> Option<Element<'_, Message>> {
    let Load::Ready(shown) = &app.lyrics else { return None };
    if !shown.is_synced() {
        return None;
    }
    let current = app.lyrics_line?;
    let now = shown.lines.get(current)?;
    let mut lines = column![
        text(now.text.as_str())
            .size(size)
            .font(theme::FONT_SEMIBOLD)
            .color(theme::TEXT)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center)
    ]
    .spacing(4)
    .align_x(Alignment::Center);
    if let Some(next) = shown.lines.get(current + 1).filter(|_| next) {
        lines = lines.push(
            text(next.text.as_str())
                .size(size * 0.72)
                .font(theme::FONT_MEDIUM)
                .color(fade(theme::TEXT, 0.55))
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
        );
    }
    Some(container(lines).width(width).padding([8, 14]).style(theme::scrim(1.0)).into())
}

/// Il video a tutta l'area, centrato; mentre il server lo scarica o GStreamer lo apre, un avviso.
#[cfg(feature = "video")]
pub fn stage(app: &App) -> Element<'_, Message> {
    match app.video.as_ref().and_then(|view| view.player.as_ref()) {
        Some(player) => {
            container(iced_video_player::VideoPlayer::new(player).on_error(|_| Message::VideoFailed))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Center)
                .align_y(alignment::Vertical::Center)
                .into()
        }
        None => {
            let message = if app.video_loading() {
                "Preparo il videoclip…"
            } else {
                "Il videoclip non è disponibile."
            };
            container(muted(message)).center(Length::Fill).into()
        }
    }
}

#[cfg(not(feature = "video"))]
pub fn stage(_app: &App) -> Element<'_, Message> {
    container(muted("Questa build non riproduce video.")).center(Length::Fill).into()
}

/// Il videoclip in piccolo, largo `width` e in formato 16:9, per il pannello «In riproduzione», con
/// il testo sincronizzato in basso. `None` se il brano non ha un videoclip o non è ancora pronto:
/// lì si mostra la copertina.
#[cfg(feature = "video")]
pub fn mini(app: &App, width: f32) -> Option<Element<'_, Message>> {
    use iced::widget::stack;

    let view = app.video.as_ref()?;
    let player = view.player.as_ref()?;
    // La 0.5 riempie lo spazio che riceve mantenendo le proporzioni: la scatola 16:9 lo decide.
    let video: iced_video_player::VideoPlayer<'_, Message> =
        iced_video_player::VideoPlayer::new(player).on_error(|_| Message::VideoFailed);
    let height = width * 9.0 / 16.0;
    let frame = container(video).width(width).height(height).clip(true);
    // Sul video piccolo solo la riga in corso: due righe ne coprirebbero metà.
    let captions = app.video_lyrics.then(|| caption(app, width, theme::FS_BODY, false)).flatten();
    Some(match captions {
        Some(captions) => stack![
            frame,
            container(captions).width(width).height(height).align_y(alignment::Vertical::Bottom)
        ]
        .into(),
        None => frame.into(),
    })
}

#[cfg(not(feature = "video"))]
pub fn mini(_app: &App, _width: f32) -> Option<Element<'_, Message>> {
    None
}
