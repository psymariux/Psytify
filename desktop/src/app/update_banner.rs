//! L'aggiornamento dell'app: la fascia sotto la barra del titolo (come `client/src/ui/update_banner.rs`
//! del client Iced) e i testi che condivide con la riga «Aggiornamento» delle Impostazioni.
//!
//! [`update_line`] è l'unica funzione che dice, dalla vista del core (`UpdateView`), che cosa scrivere e
//! quale controllo mettere: per la fascia (`Place::Banner`, solo se il core dice che c'è qualcosa da
//! mostrare: un'offerta non chiusa, il download o il file pronto) e per le Impostazioni (`Place::Settings`,
//! sempre, anche l'ultimo controllo con «Controlla ora»). I testi sono quelli del client Iced.
//!
//! La fascia è una vista a sé in `.cached()` alla sua altezza fissa: `Shell` la mette nella finestra solo
//! mentre c'è qualcosa da dire, e il loader del download (uno sprite animato) ridisegna solo lei, mai le
//! pagine. Nascosta non costa niente: nessun timer, nessun disegno, solo il confronto della riga quando la
//! vista dei download cambia. Monocroma: nessun lime (un aggiornamento non è uno stato della riproduzione).

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

/// L'altezza della fascia: quella della barra del titolo, sopra cui sta.
pub const BANNER_H: f32 = TITLE_BAR_H;

/// Dove si mostra l'aggiornamento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// La fascia sotto la barra del titolo.
    Banner,
    /// La riga «Aggiornamento» delle Impostazioni.
    Settings,
}

/// Che cosa sta a destra del testo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateControl {
    /// «Scarica» (primario): `DownloadsCmd::DownloadUpdate`.
    Download,
    /// Il download del pacchetto è in corso: la barra a dieci passi del kit con i punti percentuali del
    /// core, o il loader finché il server non ha detto la lunghezza (BACKLOG 38).
    Steps(Option<u8>),
    /// «Apri cartella» (secondario) sul pacchetto pronto: `DownloadsCmd::RevealUpdate`, che apre la sua
    /// cartella nel gestore di file per installarlo a mano (BACKLOG 39).
    Reveal,
    /// «Controlla ora» (`DownloadsCmd::CheckUpdate(Manual)`), che gira mentre il controllo è in corso.
    Check { checking: bool },
}

/// La riga dell'aggiornamento: il testo, il controllo e se si può chiudere (`DownloadsCmd::DismissUpdate`,
/// solo dove il core fa qualcosa: un'offerta, che torna con la prossima release, o il file pronto).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateLine {
    pub text: String,
    pub control: Option<UpdateControl>,
    pub closable: bool,
}

/// La riga dell'aggiornamento per `place`. Per la fascia `None` se non c'è niente da mostrare; per le
/// Impostazioni sempre una riga. Pura.
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

/// La fascia: icona, testo, «Scarica» o il loader, e la chiusura.
pub struct UpdateBanner {
    core: Entity<CoreModel>,
    line: Option<Shown>,
}

/// La riga già pronta per il disegno.
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

    /// La riga nuova (da `Shell`, quando la vista dei download cambia): si ridisegna solo se è un'altra.
    /// Dice se la fascia è da mostrare.
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
