//! A fake jam room for the host tests, on a local port: `POST /v1/jam` gives the code,
//! `GET /v1/jam/{code}/ws` opens the WebSocket (and answers with the welcome, with the state the test
//! wants), `GET /v1/catalog/tracks/{id}` serves the tracks the test gives it. What the test makes it
//! say (`say`) reaches the core as a room message; what the core sends it is read with
//! `received`. It runs on the test's runtime; every closed connection is counted.

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

/// The fake room listening.
pub(crate) struct FakeRoom {
    /// `http://127.0.0.1:<port>`, to write as `server` in the core configuration.
    pub(crate) url: String,
    received: Arc<Mutex<Vec<String>>>,
    names: Arc<Mutex<Vec<String>>>,
    closed: Arc<AtomicUsize>,
    said: mpsc::UnboundedSender<String>,
    accepting: tokio::task::JoinHandle<()>,
}

impl FakeRoom {
    /// Makes the room say a message (its JSON) to the connected core.
    pub(crate) fn say(&self, json: &serde_json::Value) {
        let _ = self.said.send(json.to_string());
    }

    /// The messages the core has sent to the room, in order.
    pub(crate) fn received(&self) -> Vec<serde_json::Value> {
        let received = self.received.lock().map(|texts| texts.clone()).unwrap_or_default();
        received.iter().filter_map(|text| serde_json::from_str(text).ok()).collect()
    }

    /// The names the core has joined with.
    pub(crate) fn names(&self) -> Vec<String> {
        self.names.lock().map(|names| names.clone()).unwrap_or_default()
    }

    /// How many WebSocket connections have ended.
    pub(crate) fn closed(&self) -> usize {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for FakeRoom {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

/// Starts the room: it has the code `code`, sends `state` (an object with `queue`, `index`,
/// `position_ms`, `playing` and `from`) to whoever joins, and serves `tracks` from the catalog.
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

/// What every connection shares with the room.
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
    // The WebSocket wants the whole request: the method is looked at without consuming it.
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
    // The signature of the callback is tungstenite's: we do not decide the big error.
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

/// The request's path, looked at without consuming it.
async fn peek_path(socket: &mut TcpStream) -> Option<String> {
    let mut buffer = [0u8; 256];
    let n = socket.peek(&mut buffer).await.ok()?;
    let text = String::from_utf8_lossy(&buffer[..n]).into_owned();
    let target = text.lines().next()?.split_whitespace().nth(1)?.to_owned();
    Some(target.split('?').next()?.to_owned())
}

/// The simple HTTP requests: the room's creation and the catalog's tracks.
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
