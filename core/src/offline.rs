//! L'indice dei brani scaricati sul dispositivo (passo 4.6b). Ogni brano salvato ha una voce con ciò
//! che serve a suonarlo senza rete e a riconoscere un file cambiato: il brano (titolo, artista,
//! copertina: l'elenco dei Scaricati non chiede niente al server), il percorso, il formato, il peso
//! e l'SHA-256 del contenuto, calcolato mentre il file veniva scritto.
//!
//! L'indice sta in `config.json` (chiave `downloaded`), non in un file a parte: la scrittura è già
//! atomica, unica e con la chiusura che aspetta il disco (`state::settings`), e un secondo file
//! avrebbe due scrittori e due punti in cui perdersi. Costa una voce di circa mezzo chilobyte per
//! brano, come i brani piaciuti.
//!
//! Un file che manca o che pesa diverso all'avvio non si cancella e non si toglie dall'indice: lo si
//! segna ([`Lost`]) e il brano si ascolta dalla rete finché l'utente non lo scarica di nuovo o lo
//! toglie.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::{DownloadQuality, Track};
use crate::player::stream::StreamBuffer;

/// Un brano scaricato: dove sta e che file è.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTrack {
    pub track: Track,
    pub path: PathBuf,
    /// L'estensione del file (`flac`, `m4a`, …).
    pub format: String,
    /// La qualità che il server ha dato davvero, se l'ha detta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<DownloadQuality>,
    /// Il peso del file in byte, com'era a fine scrittura.
    pub size: u64,
    /// SHA-256 del contenuto, in esadecimale, calcolato mentre si scriveva.
    pub sha256: String,
}

impl SavedTrack {
    /// Il file da cui suonare, con il peso che deve avere.
    pub fn local(&self) -> LocalFile {
        LocalFile { path: self.path.clone(), size: self.size }
    }
}

/// Un file da cui suonare: il percorso e il peso che l'indice ricorda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: PathBuf,
    pub size: u64,
}

/// Perché un file dell'indice non è più quello di prima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostReason {
    /// Il file non c'è più (spostato o cancellato fuori dall'app).
    Missing,
    /// C'è, ma pesa diverso (o, alla verifica profonda, ha un altro contenuto).
    Changed,
}

/// Un brano dell'indice il cui file non è a posto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lost {
    pub id: Uuid,
    pub reason: LostReason,
}

/// Il file c'è e pesa quanto deve: un controllo da un metadato, senza leggerlo.
pub fn check_size(file: &LocalFile) -> Result<(), LostReason> {
    match std::fs::metadata(&file.path) {
        Ok(meta) if meta.is_file() && meta.len() == file.size => Ok(()),
        Ok(_) => Err(LostReason::Changed),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(LostReason::Missing),
        // Un errore che non sia «non c'è» (permessi, disco): non si dà per perso un file che forse c'è.
        Err(_) => Ok(()),
    }
}

/// Apre il file di un brano scaricato per suonarlo dal disco. `Err(Some(motivo))`: il file non è più
/// quello dell'indice (manca o pesa diverso), e l'indice lo segna; `Err(None)`: non si è potuto aprire
/// per un altro motivo (un permesso, il disco), che non dice nulla sul file. In tutti e due i casi si
/// ripiega sulla rete. Legge il disco: va chiamata fuori dal riduttore.
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

/// L'SHA-256 del contenuto di `path`, in esadecimale, letto a blocchi (mai tutto in memoria).
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

/// Controlla le voci dell'indice e dice quali file non sono a posto. Senza `deep` guarda solo che il
/// file ci sia e pesi quanto deve (è quello che si fa all'avvio: una `stat` per brano); con `deep`
/// rilegge ogni file e ne confronta l'SHA-256. Legge il disco: va chiamata fuori dal riduttore.
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
        // Una cartella al posto del file non è il file.
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
