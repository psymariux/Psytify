//! Scrittura sicura di un file scaricato (passo 4.6, BACKLOG 3). Il file arriva in un temporaneo
//! dal nome unico (`.psytify-<uuid>.part`), aperto con `create_new`: non tronca mai un file
//! dell'utente e due download non condividono mai il temporaneo. A download riuscito prende il suo
//! nome vero senza coprirne un altro; se il lavoro finisce male o viene annullato (il futuro è
//! lasciato cadere) il temporaneo si toglie da solo.
//!
//! Qui stanno anche i due oggetti con cui un lavoro parla con il riduttore senza canali:
//! [`Progress`] (byte arrivati, letti a intervalli) e [`Cancel`] (annullamento cooperativo).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::Notify;
use uuid::Uuid;

/// Quanti nomi liberi si provano prima di arrendersi: un altro processo che riempie la cartella
/// mentre si sceglie il nome è già raro, otto volte di fila no.
const PUBLISH_ATTEMPTS: usize = 8;

/// A che punto è un download: il lavoro scrive, il riduttore legge a intervalli (mai a ogni blocco).
#[derive(Debug, Default)]
pub struct Progress {
    received: AtomicU64,
    /// Lunghezza annunciata dal server; 0 se non l'ha detta.
    total: AtomicU64,
    /// La connessione è aperta e i byte arrivano; prima il server sta preparando la versione.
    started: AtomicBool,
}

impl Progress {
    /// Byte scritti finora.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// Lunghezza del file, se il server l'ha detta.
    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Ordering::Relaxed)).filter(|total| *total > 0)
    }

    /// I byte hanno cominciato ad arrivare.
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    /// Una nuova connessione di dati si è aperta: il conto riparte da zero (un URL rinnovato
    /// riscarica da capo).
    pub(crate) fn begin(&self, total: Option<u64>) {
        self.received.store(0, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
        self.started.store(true, Ordering::Release);
    }

    pub(crate) fn advance(&self, bytes: u64) {
        self.received.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Un `Progress` già avviato, per i test del riduttore.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set(&self, received: u64, total: Option<u64>) {
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
        self.received.store(received, Ordering::Relaxed);
        self.started.store(true, Ordering::Release);
    }
}

#[derive(Debug, Default)]
struct CancelState {
    flag: AtomicBool,
    wake: Notify,
}

/// Annullamento cooperativo: chi tiene un clone può chiamare [`Cancel::cancel`], e il lavoro, che
/// aspetta [`Cancel::cancelled`] accanto al suo lavoro vero, si ferma.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<CancelState>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::Release);
        self.0.wake.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::Acquire)
    }

    /// Si risolve quando qualcuno annulla (subito, se è già stato annullato).
    pub async fn cancelled(&self) {
        let woken = self.0.wake.notified();
        tokio::pin!(woken);
        // Registrarsi prima di guardare il flag: un annullamento nel mezzo non si perde.
        woken.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        woken.await;
    }
}

/// Un file in scrittura nella cartella di destinazione. Se non viene pubblicato (errore, annullo, il
/// futuro lasciato cadere) sparisce quando lo si lascia cadere.
#[derive(Debug)]
pub(crate) struct TempFile {
    path: PathBuf,
    kept: bool,
}

impl TempFile {
    /// Crea un temporaneo nuovo in `dir`. `create_new` fallisce se il nome esiste già: non si apre
    /// mai un file che non è nostro.
    pub(crate) async fn create(dir: &Path) -> io::Result<(Self, tokio::fs::File)> {
        let path = dir.join(format!(".psytify-{}.part", Uuid::new_v4()));
        let file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await?;
        Ok((Self { path, kept: false }, file))
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Il file finito prende il nome `filename` in `dir`. Senza `overwrite` non ne copre mai uno
    /// esistente: sceglie " (2)", " (3)", … e prende il nome con un collegamento fisso (che fallisce
    /// se il nome è già occupato) prima di togliere il temporaneo; un file system senza collegamenti
    /// fissi (una scheda SD) ricade sulla rinomina. Con `overwrite` il nome è nostro (la cartella
    /// degli aggiornamenti) e la rinomina lo sostituisce.
    pub(crate) async fn publish(
        mut self,
        dir: &Path,
        filename: &str,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        if overwrite {
            let dest = dir.join(filename);
            tokio::fs::rename(&self.path, &dest).await?;
            self.kept = true;
            return Ok(dest);
        }
        for _ in 0..PUBLISH_ATTEMPTS {
            let dest = super::unique_path(dir, filename).await;
            match tokio::fs::hard_link(&self.path, &dest).await {
                // Il temporaneo si toglie quando `self` cade.
                Ok(()) => return Ok(dest),
                // Qualcuno ha preso il nome nel frattempo: si sceglie il successivo.
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    tokio::fs::rename(&self.path, &dest).await?;
                    self.kept = true;
                    return Ok(dest);
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "nessun nome libero"))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-transfer-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn two_temp_files_never_share_a_name_and_an_unrelated_part_file_is_untouched() {
        let dir = scratch();
        std::fs::write(dir.join("Brano.part"), b"dell'utente").unwrap();
        let (first, _) = TempFile::create(&dir).await.unwrap();
        let (second, _) = TempFile::create(&dir).await.unwrap();
        assert_ne!(first.path(), second.path());
        assert_eq!(std::fs::read(dir.join("Brano.part")).unwrap(), b"dell'utente");
        drop((first, second));
        assert_eq!(names(&dir), ["Brano.part"], "i temporanei si tolgono da soli");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn publishing_never_covers_an_existing_file() {
        let dir = scratch();
        std::fs::write(dir.join("Brano.flac"), b"mio").unwrap();
        let (temp, mut file) = TempFile::create(&dir).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut file, b"nuovo").await.unwrap();
        drop(file);
        let dest = temp.publish(&dir, "Brano.flac", false).await.unwrap();
        assert_eq!(dest, dir.join("Brano (2).flac"));
        assert_eq!(std::fs::read(dir.join("Brano.flac")).unwrap(), b"mio");
        assert_eq!(std::fs::read(&dest).unwrap(), b"nuovo");
        assert_eq!(names(&dir), ["Brano (2).flac", "Brano.flac"], "nessun temporaneo rimasto");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn overwrite_replaces_the_name_it_owns() {
        let dir = scratch();
        std::fs::write(dir.join("update.tar.gz"), b"vecchio").unwrap();
        let (temp, mut file) = TempFile::create(&dir).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut file, b"nuovo").await.unwrap();
        drop(file);
        let dest = temp.publish(&dir, "update.tar.gz", true).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"nuovo");
        assert_eq!(names(&dir), ["update.tar.gz"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_cancel_is_seen_before_and_after_it_is_awaited() {
        let cancel = Cancel::default();
        let waiting = tokio::spawn({
            let cancel = cancel.clone();
            async move { cancel.cancelled().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        cancel.cancel();
        waiting.await.unwrap();
        // Già annullato: si risolve subito.
        tokio::time::timeout(std::time::Duration::from_secs(1), cancel.cancelled()).await.unwrap();
    }

    #[test]
    fn progress_restarts_on_a_new_connection() {
        let progress = Progress::default();
        assert!(!progress.started() && progress.total().is_none());
        progress.begin(Some(100));
        progress.advance(40);
        assert_eq!((progress.received(), progress.total(), progress.started()), (40, Some(100), true));
        progress.begin(None);
        assert_eq!((progress.received(), progress.total()), (0, None));
    }
}
