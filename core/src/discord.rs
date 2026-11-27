//! Discord Rich Presence: «Sta ascoltando Psytify» con brano, artista, copertina e tempo, sul
//! profilo Discord dell'utente. Parla con l'app di Discord aperta sullo stesso computer attraverso
//! il suo canale locale (IPC), senza crate esterne: socket Unix `discord-ipc-N` su Linux e macOS,
//! named pipe `\\.\pipe\discord-ipc-N` su Windows. Su Android non c'è.
//!
//! Il protocollo: ogni messaggio è un opcode e una lunghezza (u32 little-endian) seguiti da JSON.
//! Prima l'handshake con l'Application ID dell'app Discord «Psytify», poi `SET_ACTIVITY` a ogni
//! cambio. Tutto avviene in un thread dedicato: l'interfaccia manda solo l'attività voluta su un
//! canale e non aspetta mai Discord. Se Discord non è aperto, si riprova ogni tanto in silenzio.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Ogni quanto si riprova a raggiungere Discord se l'attività non è arrivata.
const RETRY: Duration = Duration::from_secs(15);
/// Un messaggio di Discord più grande di così non è una risposta valida.
const MAX_FRAME: usize = 64 * 1024;
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

/// Cosa mostrare su Discord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Copertina pubblica (Archive o Deezer): Discord la scarica da sé.
    pub cover_url: Option<String>,
    /// Inizio e fine del brano in millisecondi dall'epoca; `None` in pausa.
    pub span: Option<(u64, u64)>,
}

impl Activity {
    /// Brano in corso dalla posizione e dalla durata: Discord mostra il tempo che scorre.
    pub fn playing(
        title: &str,
        artist: &str,
        album: Option<&str>,
        cover_url: Option<&str>,
        position: Duration,
        duration: Duration,
    ) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let start = now.saturating_sub(position);
        let millis = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        Self {
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: album.map(str::to_owned),
            cover_url: cover_url.filter(|url| url.starts_with("https://")).map(str::to_owned),
            span: (!duration.is_zero()).then(|| (millis(start), millis(start + duration))),
        }
    }

    /// Il JSON di `SET_ACTIVITY`. Discord vuole testi da 2 a 128 caratteri.
    fn to_json(&self) -> Value {
        let text = |value: &str| {
            let mut value: String = value.chars().take(128).collect();
            while value.chars().count() < 2 {
                value.push(' ');
            }
            value
        };
        let mut activity = json!({
            // 2 = «Sta ascoltando».
            "type": 2,
            "details": text(&self.title),
            "state": text(&self.artist),
        });
        let mut assets = json!({});
        if let Some(url) = &self.cover_url {
            assets["large_image"] = json!(url);
            if let Some(album) = &self.album {
                assets["large_text"] = json!(text(album));
            }
        }
        if assets.as_object().is_some_and(|a| !a.is_empty()) {
            activity["assets"] = assets;
        }
        if let Some((start, end)) = self.span {
            activity["timestamps"] = json!({"start": start, "end": end});
        }
        activity
    }
}

/// Il collegamento a Discord. Chiuderlo (drop) ferma il thread e toglie l'attività.
pub struct Presence {
    sender: Sender<Option<Activity>>,
}

impl std::fmt::Debug for Presence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Presence")
    }
}

impl Presence {
    /// Avvia il thread per l'app Discord `app_id`. `None` se l'id non è un numero o il thread non
    /// parte.
    pub fn start(app_id: &str) -> Option<Self> {
        let app_id = app_id.trim();
        if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let (sender, receiver) = mpsc::channel();
        let app_id = app_id.to_owned();
        std::thread::Builder::new().name("discord".into()).spawn(move || run(&app_id, &receiver)).ok()?;
        Some(Self { sender })
    }

    /// L'attività da mostrare, o nessuna. Non blocca.
    pub fn set(&self, activity: Option<Activity>) {
        let _ = self.sender.send(activity);
    }
}

fn run(app_id: &str, receiver: &Receiver<Option<Activity>>) {
    let mut connection: Option<Connection> = None;
    let mut wanted: Option<Activity> = None;
    // `true` finché l'ultima attività voluta non è arrivata a Discord.
    let mut dirty = false;
    loop {
        match receiver.recv_timeout(RETRY) {
            Ok(activity) => {
                // Arrivano a raffiche (salti, cambi di brano): conta l'ultima.
                wanted = receiver.try_iter().last().unwrap_or(activity);
                dirty = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(mut connection) = connection.take() {
                    let _ = connection.set_activity(None);
                    let _ = connection.write_frame(OP_CLOSE, &json!({}));
                }
                return;
            }
        }
        if !dirty {
            continue;
        }
        if connection.is_none() {
            connection = Connection::open(app_id);
        }
        let Some(open) = connection.as_mut() else { continue };
        match open.set_activity(wanted.as_ref()) {
            Ok(()) => dirty = false,
            // Discord chiuso o riavviato: al prossimo giro si riapre.
            Err(_) => connection = None,
        }
    }
}

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Stream = std::fs::File;

struct Connection {
    stream: Stream,
    nonce: u64,
}

impl Connection {
    /// I posti dove Discord apre il canale: la cartella runtime, e quelle di Flatpak e Snap.
    #[cfg(unix)]
    fn candidates() -> Vec<std::path::PathBuf> {
        let base = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .iter()
            .find_map(std::env::var_os)
            .map_or_else(|| std::path::PathBuf::from("/tmp"), std::path::PathBuf::from);
        let mut dirs = vec![base.clone()];
        dirs.push(base.join("app/com.discordapp.Discord"));
        dirs.push(base.join("snap.discord"));
        dirs.iter().flat_map(|dir| (0..10).map(move |n| dir.join(format!("discord-ipc-{n}")))).collect()
    }

    #[cfg(unix)]
    fn connect() -> Option<Stream> {
        let stream = Self::candidates().into_iter().find_map(|path| Stream::connect(path).ok())?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
        Some(stream)
    }

    #[cfg(windows)]
    fn connect() -> Option<Stream> {
        (0..10).find_map(|n| {
            std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}")).ok()
        })
    }

    fn open(app_id: &str) -> Option<Self> {
        let mut connection = Self { stream: Self::connect()?, nonce: 0 };
        connection.write_frame(OP_HANDSHAKE, &json!({"v": 1, "client_id": app_id})).ok()?;
        // La prima risposta è `READY`; un errore (id sconosciuto) chiude il canale.
        let (op, _) = connection.read_frame().ok()?;
        (op == OP_FRAME).then_some(connection)
    }

    fn set_activity(&mut self, activity: Option<&Activity>) -> std::io::Result<()> {
        self.nonce += 1;
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": std::process::id(), "activity": activity.map(Activity::to_json)},
            "nonce": self.nonce.to_string(),
        });
        self.write_frame(OP_FRAME, &payload)?;
        // Discord risponde a ogni comando: la risposta si legge, così il canale non si riempie.
        let (op, _) = self.read_frame()?;
        if op == OP_CLOSE { Err(std::io::ErrorKind::ConnectionAborted.into()) } else { Ok(()) }
    }

    fn write_frame(&mut self, op: u32, payload: &Value) -> std::io::Result<()> {
        let body = payload.to_string();
        let len =
            u32::try_from(body.len()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(body.as_bytes());
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    fn read_frame(&mut self) -> std::io::Result<(u32, Vec<u8>)> {
        let mut header = [0u8; 8];
        self.stream.read_exact(&mut header)?;
        let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        if len > MAX_FRAME {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body)?;
        Ok((op, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playing_track_becomes_a_listening_activity() {
        let activity = Activity::playing(
            "Low Tide",
            "Marea",
            Some("Banchi di sabbia"),
            Some("https://archive.org/services/img/marea"),
            Duration::from_secs(30),
            Duration::from_secs(200),
        );
        let json = activity.to_json();
        assert_eq!(json["type"], 2);
        assert_eq!((json["details"].as_str(), json["state"].as_str()), (Some("Low Tide"), Some("Marea")));
        assert_eq!(json["assets"]["large_text"], "Banchi di sabbia");
        let (start, end) = activity.span.unwrap_or_default();
        assert_eq!(end - start, 200_000);
    }

    #[test]
    fn short_texts_are_padded_and_long_ones_cut() {
        let activity = Activity::playing(
            "X",
            &"a".repeat(300),
            None,
            Some("http://insecure"),
            Duration::ZERO,
            Duration::ZERO,
        );
        let json = activity.to_json();
        assert_eq!(json["details"], "X ");
        assert_eq!(json["state"].as_str().map(|s| s.chars().count()), Some(128));
        assert!(json.get("assets").is_none() && json.get("timestamps").is_none());
    }

    #[test]
    fn only_numeric_application_ids_start_a_presence() {
        assert!(Presence::start("").is_none());
        assert!(Presence::start("not-a-number").is_none());
    }
}
