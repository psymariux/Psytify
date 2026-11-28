//! The index of tracks downloaded to the device (step 4.6b). Each saved track has an entry with what
//! is needed to play it without network and to recognise a changed file: the track (title, artist,
//! cover: the Downloads list asks the server for nothing), the path, the format, the weight
//! and the SHA-256 of the content, computed while the file was being written.
//!
//! The index lives in `config.json` (key `downloaded`), not in a separate file: the write is already
//! atomic, single and with the close waiting for the disk (`state::settings`), and a second file
//! would have two writers and two points where things can get lost. It costs an entry of about half a kilobyte per
//! track, like the liked tracks.
//!
//! A file that is missing or has a different weight at startup is neither deleted nor removed from the index: it is
//! marked ([`Lost`]) and the track plays from the network until the user downloads it again or removes it.
//! toglie.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::{DownloadQuality, Track};
use crate::player::stream::StreamBuffer;

/// A downloaded track: where it is and what file it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTrack {
    pub track: Track,
    pub path: PathBuf,
    /// The file extension (`flac`, `m4a`, …).
    pub format: String,
    /// The quality the server actually gave, if it said so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<DownloadQuality>,
    /// The file's weight in bytes, as it was at the end of writing.
    pub size: u64,
    /// SHA-256 of the content, in hexadecimal, computed while it was written.
    pub sha256: String,
}

impl SavedTrack {
    /// The file to play from, with the weight it must have.
    pub fn local(&self) -> LocalFile {
        LocalFile { path: self.path.clone(), size: self.size }
    }
}

/// A file to play from: the path and the weight the index remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: PathBuf,
    pub size: u64,
}

/// Why an index file is no longer what it was before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostReason {
    /// The file is gone (moved or deleted outside the app).
    Missing,
    /// It is there, but its weight differs (or, on the deep check, it has other content).
    Changed,
}

/// A track in the index whose file is not in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lost {
    pub id: Uuid,
    pub reason: LostReason,
}

/// The file is there and weighs what it must: a check from a metadata, without reading it.
pub fn check_size(file: &LocalFile) -> Result<(), LostReason> {
    match std::fs::metadata(&file.path) {
        Ok(meta) if meta.is_file() && meta.len() == file.size => Ok(()),
        Ok(_) => Err(LostReason::Changed),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(LostReason::Missing),
        // An error other than "not there" (permissions, disk): a file that may be there is not given up for lost.
        Err(_) => Ok(()),
    }
}

/// Opens a downloaded track's file to play it from disk. `Err(Some(reason))`: the file is no longer
/// the index's (it is missing or weighs differently), and the index marks it; `Err(None)`: it could not be opened
/// for another reason (a permission, the disk), which says nothing about the file. In both cases it
/// falls back to the network. It reads the disk: it must be called outside the reducer.
pub fn open_local(file: &LocalFile) -> Result<StreamBuffer, Option<LostReason>> {
    match StreamBuffer::open_file(&file.path, file.size) {
        Ok(stream) => Ok(stream),
        Err(err) => Err(match err.kind() {
            std::io::ErrorKind::NotFound => Some(LostReason::Missing),
            std::io::ErrorKind::InvalidData => Some(LostReason::Changed),
            _ => None,
        }),
    }
}

/// The SHA-256 of the content of `path`, in hexadecimal, read in blocks (never all in memory).
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hasher.update(&block[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Lowercase hexadecimal text of a digest.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}

/// Checks the index entries and says which files are not in order. Without `deep` it only checks that the
/// file is there and weighs what it must (this is what is done at startup: one `stat` per track); with `deep`
/// it rereads each file and compares its SHA-256. It reads the disk: it must be called outside the reducer.
pub fn verify(entries: &[(Uuid, LocalFile, String)], deep: bool) -> Vec<Lost> {
    entries
        .iter()
        .filter_map(|(id, file, sha256)| {
            let reason = match check_size(file) {
                Err(reason) => Some(reason),
                Ok(()) if deep => match hash_file(&file.path) {
                    Ok(found) if found == *sha256 => None,
                    Ok(_) => Some(LostReason::Changed),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(LostReason::Missing),
                    Err(_) => None,
                },
                Ok(()) => None,
            };
            reason.map(|reason| Lost { id: *id, reason })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-offline-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_intact_missing_or_changed() {
        let dir = scratch();
        let path = dir.join("a.flac");
        std::fs::write(&path, b"abcdef").unwrap();
        let file = LocalFile { path: path.clone(), size: 6 };
        assert_eq!(check_size(&file), Ok(()));
        assert_eq!(check_size(&LocalFile { size: 7, ..file.clone() }), Err(LostReason::Changed));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(check_size(&file), Err(LostReason::Missing));
        // A folder in place of the file is not the file.
        std::fs::create_dir(&path).unwrap();
        assert_eq!(check_size(&file), Err(LostReason::Changed));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_local_file_opens_only_if_it_is_the_one_that_was_saved() {
        let dir = scratch();
        let path = dir.join("a.flac");
        std::fs::write(&path, b"abcdef").unwrap();
        let file = LocalFile { path: path.clone(), size: 6 };
        assert!(open_local(&file).is_ok());
        assert_eq!(
            open_local(&LocalFile { size: 9, ..file.clone() }).unwrap_err(),
            Some(LostReason::Changed)
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(open_local(&file).unwrap_err(), Some(LostReason::Missing));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_hash_is_the_sha256_of_the_content() {
        let dir = scratch();
        let path = dir.join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_quick_check_looks_at_the_size_and_a_deep_one_at_the_content() {
        let dir = scratch();
        let intact = dir.join("intact.flac");
        let swapped = dir.join("swapped.flac");
        let gone = dir.join("gone.flac");
        std::fs::write(&intact, b"abcdef").unwrap();
        std::fs::write(&swapped, b"abcdeX").unwrap();
        let hash = hash_file(&intact).unwrap();
        let entries = vec![
            (Uuid::from_u128(1), LocalFile { path: intact, size: 6 }, hash.clone()),
            (Uuid::from_u128(2), LocalFile { path: swapped, size: 6 }, hash.clone()),
            (Uuid::from_u128(3), LocalFile { path: gone, size: 6 }, hash),
        ];
        let missing = Lost { id: Uuid::from_u128(3), reason: LostReason::Missing };
        assert_eq!(verify(&entries, false), [missing], "stesso peso: il controllo veloce non se ne accorge");
        assert_eq!(
            verify(&entries, true),
            [Lost { id: Uuid::from_u128(2), reason: LostReason::Changed }, missing]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
