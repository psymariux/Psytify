//! Una stanza della jam finta per i test dell'host, su una porta locale: `POST /v1/jam` dà il codice,
//! `GET /v1/jam/{code}/ws` apre il WebSocket (e risponde con il benvenuto, con lo stato che il test
//! vuole), `GET /v1/catalog/tracks/{id}` serve i brani che il test le dà. Quello che il test le fa
//! dire (`say`) arriva al core come un messaggio della stanza; ciò che il core le manda si legge con
//! `received`. Gira sul runtime del test; ogni connessione chiusa si conta.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use uuid::Uuid;

use crate::api::Track;

/// La stanza finta in ascolto.
pub(crate) struct FakeRoom {
    /// `http://127.0.0.1:<porta>`, da scrivere in `server` nella configurazione del core.
    pub(crate) url: String,
    received: Arc<Mutex<Vec<String>>>,
    names: Arc<Mutex<Vec<String>>>,
    closed: Arc<AtomicUsize>,
    said: mpsc::UnboundedSender<String>,
    accepting: tokio::task::JoinHandle<()>,
}

impl FakeRoom {
    /// Fa dire alla stanza un messaggio (il suo JSON) al core collegato.
    pub(crate) fn say(&self, json: &serde_json::Value) {
        let _ = self.said.send(json.to_string());
    }

    /// I messaggi che il core ha mandato alla stanza, nell'ordine.
    pub(crate) fn received(&self) -> Vec<serde_json::Value> {
        let received = self.received.lock().map(|texts| texts.clone()).unwrap_or_default();
        received.iter().filter_map(|text| serde_json::from_str(text).ok()).collect()
    }

    /// I nomi con cui il core è entrato.
    pub(crate) fn names(&self) -> Vec<String> {
        self.names.lock().map(|names| names.clone()).unwrap_or_default()
    }

    /// Quante connessioni WebSocket sono finite.
    pub(crate) fn closed(&self) -> usize {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for FakeRoom {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

/// Avvia la stanza: ha il codice `code`, manda `state` (un oggetto con `queue`, `index`,
/// `position_ms`, `playing` e `from`) a chi entra, e serve `tracks` dal catalogo.
pub(crate) async fn start(code: &str, state: Option<serde_json::Value>, tracks: Vec<Track>) -> FakeRoom {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
    let url = format!("http://{}", listener.local_addr().expect("a local address"));
    let (said, hear) = mpsc::unbounded_channel();
    let room = Room {
        code: code.to_owned(),
        state,
        tracks: Arc::new(tracks),
        hear: Arc::new(tokio::sync::Mutex::new(hear)),
        received: Arc::new(Mutex::new(Vec::new())),
        names: Arc::new(Mutex::new(Vec::new())),
        closed: Arc::new(AtomicUsize::new(0)),
    };
    let (received, names, closed) =
        (Arc::clone(&room.received), Arc::clone(&room.names), Arc::clone(&room.closed));
    let accepting = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, room.clone()));
        }
    });
    FakeRoom { url, received, names, closed, said, accepting }
}

/// Ciò che ogni connessione condivide con la stanza.
#[derive(Clone)]
struct Room {
    code: String,
    state: Option<serde_json::Value>,
    tracks: Arc<Vec<Track>>,
    hear: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    received: Arc<Mutex<Vec<String>>>,
    names: Arc<Mutex<Vec<String>>>,
    closed: Arc<AtomicUsize>,
}

async fn serve(mut socket: TcpStream, room: Room) {
    let Room { code, state, tracks, hear, received, names, closed } = room;
    // Il WebSocket vuole la richiesta intera: si guarda il metodo senza consumarla.
    let mut first = [0u8; 4];
    loop {
        match socket.peek(&mut first).await {
            Ok(n) if n >= 4 => break,
            Ok(_) => tokio::task::yield_now().await,
            Err(_) => return,
        }
    }
    if &first != b"GET " || peek_path(&mut socket).await.is_some_and(|path| !path.ends_with("/ws")) {
        plain(socket, &code, &tracks).await;
        return;
    }
    let seen = Arc::clone(&names);
    // La firma della callback è quella di tungstenite: l'errore grosso non lo decidiamo noi.
    #[allow(clippy::result_large_err, reason = "firma imposta da tungstenite")]
    let callback = move |request: &Request, response: Response| {
        let name = request
            .uri()
            .query()
            .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("name=")))
            .unwrap_or_default()
            .to_owned();
        if let Ok(mut names) = seen.lock() {
            names.push(name);
        }
        Ok(response)
    };
    let Ok(mut ws) = accept_hdr_async(socket, callback).await else { return };
    let you = Uuid::from_u128(500);
    let name = names.lock().ok().and_then(|names| names.last().cloned()).unwrap_or_default();
    let welcome = serde_json::json!({
        "type": "welcome",
        "you": you,
        "code": code,
        "members": [{ "id": you, "name": name }],
        "state": state.map(|mut state| {
            if let Some(object) = state.as_object_mut() {
                object.insert("at_ms".into(), 1.into());
            }
            state
        }),
    });
    if ws.send(Message::Text(welcome.to_string().into())).await.is_err() {
        return;
    }
    let mut hear = hear.lock().await;
    loop {
        tokio::select! {
            said = hear.recv() => match said {
                Some(text) => {
                    if ws.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            incoming = ws.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(mut received) = received.lock() {
                        received.push(text.to_string());
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    closed.fetch_add(1, Ordering::SeqCst);
}

/// Il percorso della richiesta, guardato senza consumarla.
async fn peek_path(socket: &mut TcpStream) -> Option<String> {
    let mut buffer = [0u8; 256];
    let n = socket.peek(&mut buffer).await.ok()?;
    let text = String::from_utf8_lossy(&buffer[..n]).into_owned();
    let target = text.lines().next()?.split_whitespace().nth(1)?.to_owned();
    Some(target.split('?').next()?.to_owned())
}

/// Le richieste HTTP semplici: la creazione della stanza e i brani del catalogo.
async fn plain(mut socket: TcpStream, code: &str, tracks: &[Track]) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&request).into_owned();
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    let path = target.split('?').next().unwrap_or_default();
    let json = |body: String| {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let reply = if method == "POST" && path == "/v1/jam" {
        json(serde_json::json!({ "code": code }).to_string())
    } else if let Some(id) = path.strip_prefix("/v1/catalog/tracks/")
        && let Some(track) = tracks.iter().find(|track| track.id.to_string() == id)
    {
        json(serde_json::to_string(track).unwrap_or_default())
    } else {
        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
    };
    let _ = socket.write_all(reply.as_bytes()).await;
}
