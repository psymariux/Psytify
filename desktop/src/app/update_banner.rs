//! The app update: the banner under the title bar and the texts it shares with the «Aggiornamento» line of the Settings.
//!
//! [`update_line`] is the only function that says, from the core's view (`UpdateView`), what to write and
//! which control to put: for the banner (`Place::Banner`, only if the core says there is something to
//! show: an offer not closed, the download or the file ready) and for the Settings (`Place::Settings`,
//! always, even the last check with «Controlla ora»). The texts are shared by both lines.
//!
//! The banner is a view of its own in `.cached()` at its fixed height: `Shell` puts it in the window only
//! while there is something to say, and the download loader (an animated sprite) redraws only it, never the
//! pages. Hidden it costs nothing: no timer, no drawing, only the comparison of the line when the
//! downloads view changes. Monochrome: no lime (an update is not a playback state).

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
};
use psytify_core::command::DownloadsCmd;
use psytify_core::snapshot::{UpdateState, UpdateStatus, UpdateView};
use psytify_design::sprite;
use psytify_design::tokens::color::{ICON_ACTIVE, SURFACE_RAISED, TEXT_PRIMARY};
use psytify_design::tokens::size::TITLE_BAR_H;
use psytify_design::tokens::space::{SPACE_2, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::TYPE_BODY;

use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::{DownloadSteps, LoadingIndicator};
use crate::core_model::CoreModel;
use crate::pixel::PixelSprite;
use crate::theme::{TypeStyle, col, dp};

/// The banner's height: that of the title bar, above which it sits.
pub const BANNER_H: f32 = TITLE_BAR_H;

/// Where the update is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The banner under the title bar.
    Banner,
    /// The «Aggiornamento» line of the Settings.
    Settings,
}

/// What sits on the right of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateControl {
    /// «Scarica» (primary): `DownloadsCmd::DownloadUpdate`.
    Download,
    /// The package download is in progress: the kit's ten-step bar with the core's percentage
    /// points, or the loader until the server has said the length (BACKLOG 38).
    Steps(Option<u8>),
    /// «Apri cartella» (secondary) on the package ready: `DownloadsCmd::RevealUpdate`, which opens its
    /// folder in the file manager to install it by hand (BACKLOG 39).
    Reveal,
    /// «Controlla ora» (`DownloadsCmd::CheckUpdate(Manual)`), which runs while the check is in progress.
    Check { checking: bool },
}

/// The update line: the text, the control and whether it can be closed (`DownloadsCmd::DismissUpdate`,
/// only where the core does something: an offer, which comes back with the next release, or the file ready).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateLine {
    pub text: String,
    pub control: Option<UpdateControl>,
    pub closable: bool,
}

/// The update line for `place`. For the banner `None` if there is nothing to show; for the
/// Settings always a line. Pure.
pub fn update_line(view: &UpdateView, place: Place) -> Option<UpdateLine> {
    let state = match place {
        Place::Banner => Some(view.banner()?),
        Place::Settings => view.state.as_ref(),
    };
    let banner = place == Place::Banner;
    let (text, control) = match state {
        Some(UpdateState::Offered(offer)) if banner => {
            (format!("Nuova versione {} disponibile", offer.tag), Some(UpdateControl::Download))
        }
        Some(UpdateState::Offered(offer)) => {
            (format!("Versione {} disponibile.", offer.tag), Some(UpdateControl::Download))
        }
        Some(state @ UpdateState::Downloading { tag, .. }) => {
            let percent = state.percent();
            let text = match (banner, percent) {
                (true, Some(percent)) => format!("Scarico {tag}… {percent}%"),
                (true, None) => format!("Scarico {tag}…"),
                (false, Some(percent)) => format!("Download di {tag} in corso: {percent}%"),
                (false, None) => format!("Download di {tag} in corso"),
            };
            (text, Some(UpdateControl::Steps(percent)))
        }
        Some(UpdateState::Downloaded(path)) => {
            (format!("Scaricato in {}", path.display()), Some(UpdateControl::Reveal))
        }
        None => {
            let line = match view.status {
                UpdateStatus::Checking => "Controllo in corso…",
                UpdateStatus::Current => "Hai l'ultima versione.",
                UpdateStatus::Failed => "Controllo non riuscito. Riprova più tardi.",
                UpdateStatus::Idle => "Controlla se c'è una versione nuova.",
            };
            (line.to_owned(), Some(UpdateControl::Check { checking: view.status == UpdateStatus::Checking }))
        }
    };
    let closable = banner && matches!(state, Some(UpdateState::Offered(_) | UpdateState::Downloaded(_)));
    Some(UpdateLine { text, control, closable })
}

/// The banner: icon, text, «Scarica» or the loader, and the close.
pub struct UpdateBanner {
    core: Entity<CoreModel>,
    line: Option<Shown>,
}

/// The line already ready for drawing.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    text: SharedString,
    control: Option<UpdateControl>,
    closable: bool,
}

impl UpdateBanner {
    pub fn new(core: Entity<CoreModel>) -> Self {
        Self { core, line: None }
    }

    /// The new line (from `Shell`, when the downloads view changes): it redraws only if it is another one.
    /// Says whether the banner is to be shown.
    pub fn set(&mut self, line: Option<UpdateLine>, cx: &mut Context<Self>) -> bool {
        let shown =
            line.map(|line| Shown { text: line.text.into(), control: line.control, closable: line.closable });
        if shown != self.line {
            self.line = shown;
            cx.notify();
        }
        self.line.is_some()
    }
}

impl Render for UpdateBanner {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strip = div()
            .size_full()
            .flex()
            .items_center()
            .gap(dp(SPACE_4))
            .pl(dp(SPACE_5))
            .pr(dp(SPACE_2))
            .bg(col(SURFACE_RAISED));
        let Some(line) = &self.line else { return strip };
        let control = line.control.and_then(|control| match control {
            UpdateControl::Download => Some(
                Button::new("update-download", "Scarica")
                    .primary()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).downloads(DownloadsCmd::DownloadUpdate)
                    }))
                    .into_any_element(),
            ),
            UpdateControl::Steps(Some(percent)) => {
                Some(DownloadSteps::new(f32::from(percent) / 100.0).into_any_element())
            }
            UpdateControl::Steps(None) => Some(LoadingIndicator::new().into_any_element()),
            UpdateControl::Reveal => Some(
                Button::new("update-reveal", "Apri cartella")
                    .secondary()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).downloads(DownloadsCmd::RevealUpdate)
                    }))
                    .into_any_element(),
            ),
            UpdateControl::Check { .. } => None,
        });
        let close = line.closable.then(|| {
            IconButton::new("update-close", sprite::ICON_CLOSE).tooltip("Chiudi", None).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| this.core.update(cx, |core, cx| core.dismiss_update(cx)),
            ))
        });
        strip
            .child(PixelSprite::new(sprite::ICON_DOWNLOAD).tint(ICON_ACTIVE))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_PRIMARY))
                    .child(line.text.clone()),
            )
            .children(control)
            .children(close)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use psytify_core::api::update::{Asset, Offer};

    use super::*;

    fn view(state: Option<UpdateState>, status: UpdateStatus) -> UpdateView {
        let mut view = UpdateView::default();
        view.state = state;
        view.status = status;
        view
    }

    fn offered(tag: &str) -> UpdateState {
        UpdateState::Offered(Offer {
            tag: tag.to_owned(),
            asset: Asset { name: "psytify-linux.tar.gz".to_owned(), size: 0 },
        })
    }

    fn downloading(tag: &str, received: u64, total: Option<u64>) -> UpdateState {
        UpdateState::Downloading { tag: tag.to_owned(), received, total }
    }

    #[test]
    fn the_banner_shows_only_what_the_core_offers() {
        for status in
            [UpdateStatus::Idle, UpdateStatus::Checking, UpdateStatus::Current, UpdateStatus::Failed]
        {
            assert_eq!(update_line(&view(None, status), Place::Banner), None, "niente da dire: {status:?}");
        }
        assert_eq!(
            update_line(&view(Some(offered("v2")), UpdateStatus::Current), Place::Banner),
            Some(UpdateLine {
                text: "Nuova versione v2 disponibile".into(),
                control: Some(UpdateControl::Download),
                closable: true,
            })
        );
        assert_eq!(
            update_line(&view(Some(downloading("v2", 0, None)), UpdateStatus::Idle), Place::Banner),
            Some(UpdateLine {
                text: "Scarico v2…".into(),
                control: Some(UpdateControl::Steps(None)),
                closable: false
            }),
            "durante il download la chiusura non farebbe niente: non c'è"
        );
        assert_eq!(
            update_line(&view(Some(downloading("v2", 420, Some(1000))), UpdateStatus::Idle), Place::Banner),
            Some(UpdateLine {
                text: "Scarico v2… 42%".into(),
                control: Some(UpdateControl::Steps(Some(42))),
                closable: false
            }),
            "con la lunghezza, i punti del core e la barra a dieci passi"
        );
        let ready = UpdateState::Downloaded(PathBuf::from("/dati/updates/psytify-v2.bin"));
        assert_eq!(
            update_line(&view(Some(ready), UpdateStatus::Idle), Place::Banner),
            Some(UpdateLine {
                text: "Scaricato in /dati/updates/psytify-v2.bin".into(),
                control: Some(UpdateControl::Reveal),
                closable: true,
            })
        );
    }

    #[test]
    fn the_settings_row_always_says_something() {
        assert_eq!(
            update_line(&view(None, UpdateStatus::Checking), Place::Settings),
            Some(UpdateLine {
                text: "Controllo in corso…".into(),
                control: Some(UpdateControl::Check { checking: true }),
                closable: false,
            })
        );
        let text = |state, status| update_line(&view(state, status), Place::Settings).map(|line| line.text);
        assert_eq!(text(None, UpdateStatus::Current).as_deref(), Some("Hai l'ultima versione."));
        assert_eq!(
            text(None, UpdateStatus::Failed).as_deref(),
            Some("Controllo non riuscito. Riprova più tardi.")
        );
        assert_eq!(text(None, UpdateStatus::Idle).as_deref(), Some("Controlla se c'è una versione nuova."));
        assert_eq!(
            text(Some(offered("v2")), UpdateStatus::Idle).as_deref(),
            Some("Versione v2 disponibile.")
        );
        let line = update_line(&view(Some(downloading("v2", 0, None)), UpdateStatus::Idle), Place::Settings);
        assert_eq!(
            line,
            Some(UpdateLine {
                text: "Download di v2 in corso".into(),
                control: Some(UpdateControl::Steps(None)),
                closable: false,
            })
        );
        let line =
            update_line(&view(Some(downloading("v2", 999, Some(1000))), UpdateStatus::Idle), Place::Settings);
        assert_eq!(line.map(|line| line.text).as_deref(), Some("Download di v2 in corso: 99%"));
        let ready = UpdateState::Downloaded(PathBuf::from("/dati/updates/psytify-v2.bin"));
        assert_eq!(
            update_line(&view(Some(ready), UpdateStatus::Idle), Place::Settings)
                .and_then(|line| line.control),
            Some(UpdateControl::Reveal),
            "le Impostazioni aprono la stessa cartella della fascia"
        );
    }
}
