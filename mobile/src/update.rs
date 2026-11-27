//! The app's own update, in «Impostazioni»: the line and its control from the core's `UpdateView`,
//! as the desktop's settings row (`desktop/src/app/update_banner.rs`, `update_line` for the settings
//! place). The core checks the release at start and every six hours (builds with a release tag
//! only), downloads the package into `data_dir/updates`, and the app hands the file to Android's
//! `PackageInstaller`, which asks the person to confirm.

use std::path::PathBuf;

use psytify_core::snapshot::{UpdateState, UpdateStatus, UpdateView};

/// What the update row offers.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum UpdateControl {
    /// «Controlla ora»; `checking` while the core asks the server.
    Check { checking: bool },
    /// «Scarica» the offered package.
    Download,
    /// The ten-step bar while the package downloads; `None` until the server says its length.
    Steps { percent: Option<u8> },
    /// «Installa»: the package is in the app's files, Android asks to confirm.
    Install,
}

/// The update row of the settings.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UpdateLine {
    pub text: String,
    pub control: UpdateControl,
    /// The offer or the ready package can be put aside (`DownloadsCmd::DismissUpdate`).
    pub closable: bool,
}

impl UpdateLine {
    /// Always a line: with nothing offered, the result of the last check and «Controlla ora».
    pub(crate) fn of(view: &UpdateView) -> Self {
        let (text, control) = match &view.state {
            Some(UpdateState::Offered(offer)) => {
                (format!("Versione {} disponibile.", offer.tag), UpdateControl::Download)
            }
            Some(state @ UpdateState::Downloading { tag, .. }) => {
                let percent = state.percent();
                let text = match percent {
                    Some(percent) => format!("Download di {tag} in corso: {percent}%"),
                    None => format!("Download di {tag} in corso"),
                };
                (text, UpdateControl::Steps { percent })
            }
            Some(UpdateState::Downloaded(_)) => (
                "Aggiornamento scaricato. Android chiede di confermare l'installazione.".to_owned(),
                UpdateControl::Install,
            ),
            None => {
                let line = match view.status {
                    UpdateStatus::Checking => "Controllo in corso…",
                    UpdateStatus::Current => "Hai l'ultima versione.",
                    UpdateStatus::Failed => "Controllo non riuscito. Riprova più tardi.",
                    UpdateStatus::Idle => "Controlla se c'è una versione nuova.",
                };
                (line.to_owned(), UpdateControl::Check { checking: view.status == UpdateStatus::Checking })
            }
        };
        let closable = matches!(view.state, Some(UpdateState::Offered(_) | UpdateState::Downloaded(_)));
        Self { text, control, closable }
    }
}

/// The downloaded package, if the core has one ready.
pub(crate) fn package(view: &UpdateView) -> Option<PathBuf> {
    match &view.state {
        Some(UpdateState::Downloaded(path)) => Some(path.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::update::{Asset, Offer};

    use super::*;

    fn view(state: Option<UpdateState>, status: UpdateStatus) -> UpdateView {
        let mut view = UpdateView::default();
        view.state = state;
        view.status = status;
        view
    }

    #[test]
    fn the_row_follows_the_update_from_the_check_to_the_package() {
        let idle = UpdateLine::of(&view(None, UpdateStatus::Idle));
        assert_eq!(idle.control, UpdateControl::Check { checking: false });
        assert!(!idle.closable);
        let checking = UpdateLine::of(&view(None, UpdateStatus::Checking));
        assert_eq!(
            (checking.text.as_str(), checking.control),
            ("Controllo in corso…", UpdateControl::Check { checking: true })
        );
        assert_eq!(UpdateLine::of(&view(None, UpdateStatus::Current)).text, "Hai l'ultima versione.");

        let offer =
            Offer { tag: "v2".into(), asset: Asset { name: "psytify-v2-android.apk".into(), size: 9 } };
        let offered = UpdateLine::of(&view(Some(UpdateState::Offered(offer)), UpdateStatus::Idle));
        assert_eq!(
            (offered.text.as_str(), offered.control.clone(), offered.closable),
            ("Versione v2 disponibile.", UpdateControl::Download, true)
        );

        let downloading = UpdateState::Downloading { tag: "v2".into(), received: 420, total: Some(1000) };
        let line = UpdateLine::of(&view(Some(downloading), UpdateStatus::Idle));
        assert_eq!(
            (line.text.as_str(), line.control),
            ("Download di v2 in corso: 42%", UpdateControl::Steps { percent: Some(42) })
        );
        let unknown = UpdateState::Downloading { tag: "v2".into(), received: 0, total: None };
        assert_eq!(
            UpdateLine::of(&view(Some(unknown), UpdateStatus::Idle)).control,
            UpdateControl::Steps { percent: None }
        );

        let ready = view(
            Some(UpdateState::Downloaded(PathBuf::from("/data/updates/psytify-v2-android.apk"))),
            UpdateStatus::Idle,
        );
        let line = UpdateLine::of(&ready);
        assert_eq!((line.control, line.closable), (UpdateControl::Install, true));
        assert!(!line.text.contains("/data"), "the path is not shown: {}", line.text);
        assert_eq!(package(&ready), Some(PathBuf::from("/data/updates/psytify-v2-android.apk")));
        assert_eq!(package(&view(None, UpdateStatus::Idle)), None);
    }
}
