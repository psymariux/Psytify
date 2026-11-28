//! Safe writing of a downloaded file (step 4.6, BACKLOG 3). The file arrives in a temporary with a
//! unique name (`.psytify-<uuid>.part`), opened with `create_new`: it never truncates a file
//! of the user's and two downloads never share the temporary. On success it takes its real
//! name without covering another one; if the job ends badly or is cancelled (the future is
//! dropped) the temporary removes itself.
//!
//! Here also sit the two objects with which a job talks to the reducer without channels:
//! [`Progress`] (byte arrivati, letti a intervalli) e [`Cancel`] (annullamento cooperativo).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::Notify;
use uuid::Uuid;

/// How many free names are tried before giving up: another process filling the folder
/// while the name is chosen is already rare, eight times in a row is not.
const PUBLISH_ATTEMPTS: usize = 8;

/// How far a download has got: the job writes, the reducer reads at intervals (never at each block).
#[derive(Debug, Default)]
pub struct Progress {
    received: AtomicU64,
    /// Length announced by the server; 0 if it did not say.
    total: AtomicU64,
    /// The connection is open and the bytes arrive; before, the server is preparing the version.
    started: AtomicBool,
}

impl Progress {
    /// Bytes written so far.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// Length of the file, if the server said it.
    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Ordering::Relaxed)).filter(|total| *total > 0)
    }

    /// The bytes have started to arrive.
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    /// A new data connection has opened: the count starts from zero (a renewed URL
    /// starts over from zero).
    pub(crate) fn begin(&self, total: Option<u64>) {
        self.received.store(0, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
        self.started.store(true, Ordering::Release);
    }

    pub(crate) fn advance(&self, bytes: u64) {
        self.received.fetch_add(bytes, Ordering::Relaxed);
    }

    /// A `Progress` already started, for the reducer's tests.
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

/// Cooperative cancellation: whoever holds a clone can call [`Cancel::cancel`], and the job, which
/// waits for [`Cancel::cancelled`] beside its real work, and stops.
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

    /// Resolves when someone cancels (at once, if it has already been cancelled).
    pub async fn cancelled(&self) {
        let woken = self.0.wake.notified();
        tokio::pin!(woken);
        // Register before looking at the flag: a cancellation in between is not lost.
        woken.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        woken.await;
    }
}

/// A file being written in the destination folder. If it is not published (error, cancel, the
/// future dropped) it disappears when dropped.
#[derive(Debug)]
pub(crate) struct TempFile {
    path: PathBuf,
    kept: bool,
}

impl TempFile {
    /// Creates a new temporary in `dir`. `create_new` fails if the name already exists: a file that
    /// is not ours is never opened.
    pub(crate) async fn create(dir: &Path) -> io::Result<(Self, tokio::fs::File)> {
        let path = dir.join(format!(".psytify-{}.part", Uuid::new_v4()));
        let file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await?;
        Ok((Self { path, kept: false }, file))
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The finished file takes the name `filename` in `dir`. Without `overwrite` it never covers an
    /// existing one: it picks " (2)", " (3)", … and takes the name with a hard link (which fails
    /// if the name is already taken) before removing the temporary; a file system without hard links
    /// (an SD card) falls back to renaming. With `overwrite` the name is ours (the updates folder)
    /// and renaming replaces it.
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
                // The temporary is removed when `self` drops.
                Ok(()) => return Ok(dest),
                // Someone took the name in the meantime: the next one is chosen.
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
        // Already cancelled: it resolves at once.
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
