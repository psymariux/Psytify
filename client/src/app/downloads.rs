//! I download e gli aggiornamenti dell'app. Dal passo 4.6 li possiede il core
//! (`core/src/state/downloads.rs`): qui i messaggi diventano `DownloadsCmd`, l'app legge la vista
//! (`App::downloads`: la coda con l'avanzamento di ogni brano, il lotto, l'aggiornamento offerto) e
//! traduce gli eventi del core negli avvisi di sempre. I due effetti della sola piattaforma (l'avviso
//! di sistema per una release nuova e l'installazione del pacchetto, su Android) restano qui.

use std::sync::Arc;

use iced::Task;
use psytify_core::Command;
use psytify_core::command::DownloadsCmd;
use psytify_core::snapshot::{DownloadsEvent, Refusal};

use super::{App, Message, Notice};
use crate::api;
use crate::error::AppError;

impl App {
    /// Un comando ai download del core. Il core lo applica al suo turno e pubblica la vista nuova: ciò
    /// che l'utente deve sapere lo dicono gli eventi, in [`App::downloads_published`].
    pub(super) fn send_downloads(&mut self, command: DownloadsCmd) -> Task<Message> {
        self.send_core(Command::Downloads(command))
    }

    /// Copia nell'app la vista dei download dell'ultima istantanea, senza mostrare niente: lo stato
    /// iniziale. Gli eventi già nella vista non si rimostrano.
    pub(super) fn copy_downloads(&mut self) {
        self.downloads = Arc::clone(&self.core_snapshot.downloads);
        self.downloads_event = self.downloads.events.back().map_or(0, |raised| raised.seq);
        self.sync_downloaded();
    }

    /// L'elenco dei brani sul dispositivo come lista di brani, rifatto solo quando l'indice del core
    /// è un altro (la vista non lo ricalcola).
    fn sync_downloaded(&mut self) {
        if !self.downloaded_from.same(&self.downloads.saved) {
            self.downloaded_from = self.downloads.saved.clone();
            self.downloaded = self.downloads.saved.iter().map(|saved| saved.track.clone()).collect();
        }
    }

    /// I download appena pubblicati dal core: si tiene la vista nuova e si dice ciò che gli eventi
    /// dicono, in ordine. Fra una pubblicazione e l'altra l'avanzamento cambia solo di un punto
    /// percentuale: qui non c'è altro lavoro.
    pub(super) fn downloads_published(&mut self) {
        self.downloads = Arc::clone(&self.core_snapshot.downloads);
        self.sync_downloaded();
        let shown = self.downloads_event;
        let view = Arc::clone(&self.downloads);
        for raised in view.events.iter().filter(|raised| raised.seq > shown) {
            self.downloads_event = raised.seq;
            self.notice = Some(match &raised.event {
                DownloadsEvent::Saved(done) => Notice::Info(saved_message(done)),
                DownloadsEvent::Failed { error, .. } => {
                    Notice::Error(format!("Download non riuscito. {error}"))
                }
                DownloadsEvent::BatchFinished { done, failed, last_error } => {
                    batch_summary(*done, *failed, last_error.as_ref())
                }
                DownloadsEvent::Refused(refusal) => Notice::Info(refusal_message(*refusal).into()),
                DownloadsEvent::UpdateOffered { tag, notify } => {
                    #[cfg(target_os = "android")]
                    if *notify {
                        crate::phone_notify_update(tag);
                    }
                    #[cfg(not(target_os = "android"))]
                    let _ = notify;
                    Notice::Info(format!("Nuova versione {tag} disponibile"))
                }
                DownloadsEvent::UpToDate => Notice::Info("Hai già l'ultima versione".into()),
                DownloadsEvent::UpdateCheckFailed(err) => {
                    Notice::Error(format!("Controllo non riuscito: {err}"))
                }
                DownloadsEvent::Unreachable => Notice::Error("Il server non è raggiungibile".into()),
                DownloadsEvent::UpdateSaved(path) => {
                    #[cfg(target_os = "android")]
                    crate::phone_install(path);
                    Notice::Info(if cfg!(target_os = "android") {
                        "Scaricato. Android ti chiede di confermare l'installazione.".to_owned()
                    } else {
                        format!("Scaricato in {}. Installalo a mano.", path.display())
                    })
                }
                DownloadsEvent::UpdateFailed(err) => Notice::Error(format!("Download non riuscito: {err}")),
                DownloadsEvent::Removed { title, kept_file } => Notice::Info(if *kept_file {
                    format!(
                        "«{title}» non è più nei download. Il file è cambiato fuori dall'app: l'ho lasciato dov'è"
                    )
                } else {
                    format!("«{title}» tolto dal dispositivo")
                }),
                DownloadsEvent::RemoveFailed { title, error } => Notice::Error(format!(
                    "«{title}» non è più nei download, ma il file non si è cancellato. {error}"
                )),
                DownloadsEvent::Verified { checked, lost: 0 } => {
                    Notice::Info(format!("Controllati {checked} brani: i file sono tutti a posto"))
                }
                DownloadsEvent::Verified { checked, lost } => Notice::Error(format!(
                    "Controllati {checked} brani: {lost} non hanno più il loro file. Suonano dalla rete finché non li scarichi di nuovo"
                )),
                DownloadsEvent::FolderNotOpened => {
                    Notice::Error("Non riesco ad aprire la cartella dei download".into())
                }
                DownloadsEvent::UpdateFolderNotOpened => {
                    Notice::Error("Non riesco ad aprire la cartella dell'aggiornamento".into())
                }
            });
        }
    }

    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_downloads(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateCheck(kind) => self.send_downloads(DownloadsCmd::CheckUpdate(kind)),
            Message::UpdateDismiss => self.send_downloads(DownloadsCmd::DismissUpdate),
            Message::UpdateRequested => self.send_downloads(DownloadsCmd::DownloadUpdate),
            Message::UpdateReveal => self.send_downloads(DownloadsCmd::RevealUpdate),
            Message::Download(track) => self.send_downloads(DownloadsCmd::Download(Box::new(track))),
            Message::DownloadAs(track, quality) => {
                self.send_downloads(DownloadsCmd::DownloadAs(Box::new(track), quality))
            }
            Message::DownloadAll(tracks) => self.send_downloads(DownloadsCmd::DownloadAll(tracks)),
            // La qualità la tiene il core e la scrive in `config.json`.
            Message::SetDownloadQuality(quality) => self.send_downloads(DownloadsCmd::SetQuality(quality)),
            Message::DownloadCancel(id) => self.send_downloads(DownloadsCmd::Cancel(id)),
            Message::DownloadRetry(id) => self.send_downloads(DownloadsCmd::Retry(id)),
            Message::DownloadRemove(id) => self.send_downloads(DownloadsCmd::Remove(id)),
            Message::DownloadsVerify => self.send_downloads(DownloadsCmd::Verify),
            Message::DownloadsOpenFolder => self.send_downloads(DownloadsCmd::OpenFolder),
            Message::DownloadsShowMore => {
                self.downloads_rows += crate::ui::pages::downloads::PAGE_ROWS;
                Task::none()
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}

/// L'avviso finale di «Scarica tutto»: quanti brani sono arrivati e, se qualcuno no, l'ultimo errore.
fn batch_summary(done: usize, failed: usize, last_error: Option<&AppError>) -> Notice {
    match (failed, last_error) {
        (0, _) => Notice::Info(format!("Scaricati {done} brani")),
        (_, Some(err)) => Notice::Error(format!("Scaricati {done} brani su {}. {err}", done + failed)),
        _ => Notice::Error(format!("Scaricati {done} brani su {}", done + failed)),
    }
}

/// Perché un download non è partito.
fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotDownloadable => "Questo brano non si può scaricare: la licenza non lo permette",
        Refusal::NoneDownloadable => "Nessuno di questi brani si può scaricare: la licenza non lo permette",
        Refusal::AlreadyQueued => "Niente da scaricare: i brani sono già in corso",
        Refusal::QueueFull => "Troppi brani in coda: riprova quando ne sono scesi",
        Refusal::AlreadySaved => "Questo brano è già sul dispositivo",
    }
}

/// L'avviso dopo un download: dove è finito e, se la qualità non è quella chiesta, quale è arrivata.
fn saved_message(done: &api::Downloaded) -> String {
    let place = done.path.display();
    match done.delivered {
        Some(got) if done.is_lower_than_asked() => {
            format!(
                "Salvato in {place} · qualità {}, la migliore che il brano ha",
                got.label().to_lowercase()
            )
        }
        Some(got) => format!("Salvato in {place} · qualità {}", got.label().to_lowercase()),
        None => format!("Salvato in {place}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_batch_summary_counts_what_arrived() {
        assert_eq!(batch_summary(3, 0, None), Notice::Info("Scaricati 3 brani".into()));
        assert_eq!(
            batch_summary(2, 1, Some(&AppError::Network)),
            Notice::Error(format!("Scaricati 2 brani su 3. {}", AppError::Network))
        );
        assert_eq!(batch_summary(2, 1, None), Notice::Error("Scaricati 2 brani su 3".into()));
    }
}
