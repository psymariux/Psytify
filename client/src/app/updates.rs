//! Aggiornamenti dell'app: controllo all'avvio, ogni tanto e a richiesta; l'offerta resta in
//! `App::update` finché non si scarica o non la si chiude. Il banner (`ui/update_banner.rs`) e la
//! riga in Impostazioni leggono di qui; sul telefono un controllo periodico che trova una release
//! nuova manda anche la notifica di sistema.

use std::time::Duration;

use iced::Task;

use super::{App, Message, Notice, UpdateState};
use crate::api;
use crate::error::AppError;

/// Ogni quanto un'app rimasta aperta richiede l'ultima release. Una richiesta ogni sei ore: il
/// backend tiene la lista in cache per quindici minuti e GitHub non vede nulla.
pub(super) const UPDATE_POLL: Duration = Duration::from_secs(6 * 60 * 60);

/// Chi ha chiesto il controllo: cambia cosa si dice quando non c'è nulla di nuovo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    /// All'avvio.
    Startup,
    /// Il timer, ad app aperta da ore.
    Poll,
    /// «Controlla ora» in Impostazioni: risponde sempre, anche «sei aggiornato» o un errore.
    Manual,
}

/// Com'è andato l'ultimo controllo, per la riga in Impostazioni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// La release più recente è questa.
    Current,
    Failed,
}

impl App {
    /// Chiede al backend l'ultima release. Una compilazione locale (senza tag) non controlla da
    /// sola, perché non saprebbe cosa confrontare; a richiesta sì, e offre l'ultima release.
    pub(super) fn check_update(&mut self, kind: UpdateCheck) -> Task<Message> {
        if api::update::BUILD_TAG.is_none() && kind != UpdateCheck::Manual {
            return Task::none();
        }
        let Ok(api) = self.api.clone() else {
            if kind == UpdateCheck::Manual {
                self.notice = Some(Notice::Error("Il server non è raggiungibile".into()));
            }
            return Task::none();
        };
        self.update_status = UpdateStatus::Checking;
        let build = Some(api::update::BUILD_TAG.unwrap_or(""));
        Task::perform(
            async move { api.latest_release().await.map(|release| api::update::offer(release, build)) },
            move |result| Message::UpdateChecked(kind, result),
        )
    }

    /// Il controllo è tornato: un'offerta nuova compare nel banner e in un avviso; senza offerta
    /// si resta com'eravamo, e a chi ha chiesto a mano si dice come stanno le cose.
    pub(super) fn update_checked(
        &mut self,
        kind: UpdateCheck,
        result: Result<Option<api::update::Offer>, AppError>,
    ) -> Task<Message> {
        // Un download in corso o già fatto non si tocca.
        let busy = matches!(self.update, Some(UpdateState::Downloading(_) | UpdateState::Downloaded(_)));
        match result {
            Ok(Some(offer)) => {
                self.update_status = UpdateStatus::Idle;
                if busy {
                    return Task::none();
                }
                let fresh =
                    !matches!(&self.update, Some(UpdateState::Offered(known)) if known.tag == offer.tag);
                let tag = offer.tag.clone();
                self.update = Some(UpdateState::Offered(offer));
                if fresh {
                    self.update_dismissed = None;
                    self.notice = Some(Notice::Info(format!("Nuova versione {tag} disponibile")));
                    // L'avviso di sistema serve a chi non guarda l'app: solo dal timer, quando
                    // l'app è rimasta aperta (o in sottofondo) per ore.
                    if kind == UpdateCheck::Poll && self.update_notified.as_deref() != Some(tag.as_str()) {
                        self.update_notified = Some(tag.clone());
                        #[cfg(target_os = "android")]
                        crate::phone_notify_update(&tag);
                    }
                }
            }
            Ok(None) => {
                self.update_status = UpdateStatus::Current;
                if !busy {
                    self.update = None;
                }
                if kind == UpdateCheck::Manual {
                    self.notice = Some(Notice::Info("Hai già l'ultima versione".into()));
                }
            }
            Err(err) => {
                self.update_status = UpdateStatus::Failed;
                if kind == UpdateCheck::Manual {
                    self.notice = Some(Notice::Error(format!("Controllo non riuscito: {err}")));
                }
            }
        }
        Task::none()
    }

    /// Il banner si può chiudere: torna da solo con la prossima release.
    pub(super) fn dismiss_update(&mut self) {
        if let Some(UpdateState::Offered(offer)) = &self.update {
            self.update_dismissed = Some(offer.tag.clone());
        } else if matches!(self.update, Some(UpdateState::Downloaded(_))) {
            self.update = None;
        }
    }

    /// Il banner da mostrare, se c'è: un'offerta non chiusa, il download o il file pronto.
    pub fn update_banner(&self) -> Option<&UpdateState> {
        match self.update.as_ref()? {
            UpdateState::Offered(offer) if self.update_dismissed.as_deref() == Some(offer.tag.as_str()) => {
                None
            }
            state => Some(state),
        }
    }

    /// Serve il timer del controllo periodico: solo con una release da cui confrontare.
    pub(super) fn polls_updates(&self) -> bool {
        api::update::BUILD_TAG.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::update::{Asset, Offer};

    fn offer(tag: &str) -> Offer {
        Offer { tag: tag.into(), asset: Asset { name: format!("psytify-{tag}.bin"), size: 1 } }
    }

    #[test]
    fn a_new_release_is_offered_announced_once_and_can_be_dismissed() {
        let mut app = App::new().0;
        let _ = app.update_checked(UpdateCheck::Startup, Ok(Some(offer("v2"))));
        assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("v2")));
        assert!(app.update_banner().is_some());

        app.notice = None;
        let _ = app.update_checked(UpdateCheck::Poll, Ok(Some(offer("v2"))));
        assert!(app.notice.is_none(), "la stessa release non si annuncia di nuovo");

        app.dismiss_update();
        assert!(app.update_banner().is_none());
        let _ = app.update_checked(UpdateCheck::Poll, Ok(Some(offer("v3"))));
        assert!(app.update_banner().is_some(), "una release più nuova riporta il banner");
    }

    #[test]
    fn a_manual_check_always_answers_and_a_poll_stays_quiet() {
        let mut app = App::new().0;
        // Su una macchina senza configurazione l'avvio ha già un suo avviso: qui conta solo il resto.
        app.notice = None;
        let _ = app.update_checked(UpdateCheck::Poll, Ok(None));
        assert!(app.notice.is_none());
        assert_eq!(app.update_status, UpdateStatus::Current);

        let _ = app.update_checked(UpdateCheck::Manual, Ok(None));
        assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("ultima versione")));

        app.notice = None;
        let _ = app.update_checked(UpdateCheck::Poll, Err(AppError::Network));
        assert!(app.notice.is_none());
        assert_eq!(app.update_status, UpdateStatus::Failed);
        let _ = app.update_checked(UpdateCheck::Manual, Err(AppError::Network));
        assert!(matches!(&app.notice, Some(Notice::Error(_))));
    }

    #[test]
    fn a_download_in_progress_is_not_replaced_by_a_check() {
        let mut app = App::new().0;
        app.update = Some(UpdateState::Downloading("v2".into()));
        let _ = app.update_checked(UpdateCheck::Poll, Ok(Some(offer("v3"))));
        assert!(matches!(app.update, Some(UpdateState::Downloading(_))));
        let _ = app.update_checked(UpdateCheck::Manual, Ok(None));
        assert!(matches!(app.update, Some(UpdateState::Downloading(_))));
    }
}
