//! Un backend finto per i test dell'host: risponde come il vero agli URL firmati di ascolto e di
//! download e serve un solo file audio, con le richieste `Range` che il lettore progressivo fa; a
//! `/v1/cover` risponde con un PNG 8 x 8 (le copertine passano dal backend) e, se il test gliela
//! dà, a `POST /v1/meta/tracks/{id}/resolve` con la risposta di importazione e, per i videoclip, a
//! `GET /v1/tracks/{id}/video-url` con le altezze che il test sceglie (una per richiesta, l'ultima si
//! ripete) e a `POST /v1/tracks/{id}/video/prepare` con un «fatto». Gira
//! sul runtime del test, su una porta locale; ogni richiesta resta in un elenco, così un test dice
//! se il core è andato in rete o ha suonato dal disco.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Il backend finto in ascolto.
pub(crate) struct FakeBackend {
    /// `http://127.0.0.1:<porta>`, da scrivere in `server` nella configurazione del core.
    pub(crate) url: String,
    hits: Arc<Mutex<Vec<String>>>,
    /// Le altezze dei videoclip da servire: una per richiesta di `video-url`, l'ultima resta.
    #[cfg_attr(not(feature = "video"), expect(dead_code, reason = "lo leggono solo i test dei videoclip"))]
    heights: Arc<Mutex<VecDeque<u32>>>,
    accepting: tokio::task::JoinHandle<()>,
}

impl FakeBackend {
    /// Spegne il backend: la porta si chiude e ogni richiesta nuova fallisce, come senza rete.
    pub(crate) fn stop(&self) {
        self.accepting.abort();
    }

    /// Le richieste ricevute finora, nell'ordine, come «GET /percorso» senza la query.
    pub(crate) fn hits(&self) -> Vec<String> {
        self.hits.lock().map(|hits| hits.clone()).unwrap_or_default()
    }

    /// Le altezze che `video-url` darà alle richieste che seguono (senza altezze, un videoclip
    /// completo): la prima richiesta la prima, e così via; l'ultima si ripete.
    #[cfg_attr(not(feature = "video"), expect(dead_code, reason = "lo usano solo i test dei videoclip"))]
    pub(crate) fn serve_video_heights(&self, heights: &[u32]) {
        if let Ok(mut queue) = self.heights.lock() {
            *queue = heights.iter().copied().collect();
        }
    }

    /// Quante richieste hanno questo percorso come inizio.
    pub(crate) fn count(&self, prefix: &str) -> usize {
        self.hits().iter().filter(|hit| hit.starts_with(&format!("GET {prefix}"))).count()
    }
}

/// Avvia il backend: serve `audio` (di formato `format`) per ogni brano.
pub(crate) async fn start(audio: Vec<u8>, format: &'static str) -> FakeBackend {
    start_with(audio, format, String::new()).await
}

/// Come [`start`], e a `/v1/meta/tracks/{id}/resolve` risponde con `resolve` (il JSON di un
/// `DiscoverImport`): è ciò che il server dà quando importa un brano del catalogo ufficiale.
pub(crate) async fn start_with(audio: Vec<u8>, format: &'static str, resolve: String) -> FakeBackend {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
    let url = format!("http://{}", listener.local_addr().expect("a local address"));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let audio = Arc::new(audio);
    let resolve = Arc::new(resolve);
    let seen = Arc::clone(&hits);
    let heights = Arc::new(Mutex::new(VecDeque::new()));
    let video = Arc::clone(&heights);
    let accepting = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(
                socket,
                Arc::clone(&audio),
                format,
                Arc::clone(&resolve),
                Arc::clone(&seen),
                Arc::clone(&video),
            ));
        }
    });
    FakeBackend { url, hits, heights, accepting }
}

async fn serve(
    mut socket: TcpStream,
    audio: Arc<Vec<u8>>,
    format: &'static str,
    resolve: Arc<String>,
    hits: Arc<Mutex<Vec<String>>>,
    heights: Arc<Mutex<VecDeque<u32>>>,
) {
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
    let target = line.split_whitespace().nth(1).unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default();
    let method = line.split_whitespace().next().unwrap_or("GET");
    if let Ok(mut hits) = hits.lock() {
        hits.push(format!("{method} {path}"));
    }
    let range_start = text
        .lines()
        .find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned))
        .and_then(|range| range.split('-').next().and_then(|start| start.parse::<usize>().ok()));
    let json = |body: String| {
        format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
            .into_bytes()
    };
    let reply = if path.ends_with("/download-url") {
        json(format!(
            r#"{{"kind":"download","url":"/v1/media?t=file","expires_at":"2099-01-01T00:00:00Z","format":"{format}","quality":"lossless"}}"#
        ))
    } else if method == "GET" && path.ends_with("/video-url") {
        let height = heights
            .lock()
            .ok()
            .and_then(|mut queue| if queue.len() > 1 { queue.pop_front() } else { queue.front().copied() });
        let height = height.map(|height| format!(r#","height":{height}"#)).unwrap_or_default();
        json(format!(
            r#"{{"kind":"video","url":"/v1/media?t=clip","expires_at":"2099-01-01T00:00:00Z"{height}}}"#
        ))
    } else if method == "POST" && path.ends_with("/video/prepare") {
        json("{}".to_owned())
    } else if path.ends_with("/stream-url") {
        json(r#"{"kind":"stream","url":"/v1/media?t=file","expires_at":"2099-01-01T00:00:00Z"}"#.to_owned())
    } else if path == "/v1/media" {
        let total = audio.len();
        let (head, body) = match range_start {
            Some(start) if start < total => (
                format!(
                    "HTTP/1.1 206 Partial Content\r\naccept-ranges: bytes\r\ncontent-range: bytes {start}-{}/{total}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    total - 1,
                    total - start
                ),
                &audio[start..],
            ),
            _ => (
                format!(
                    "HTTP/1.1 200 OK\r\naccept-ranges: bytes\r\ncontent-length: {total}\r\nconnection: close\r\n\r\n"
                ),
                &audio[..],
            ),
        };
        let mut reply = head.into_bytes();
        reply.extend_from_slice(body);
        reply
    } else if method == "POST" && path.starts_with("/v1/meta/tracks/") && path.ends_with("/resolve") {
        if resolve.is_empty() {
            b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()
        } else {
            json(resolve.to_string())
        }
    } else if path == "/v1/cover" {
        let mut png = Vec::new();
        let written = image::DynamicImage::new_rgb8(8, 8)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
        match written {
            Ok(()) => {
                let mut reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    png.len()
                )
                .into_bytes();
                reply.extend_from_slice(&png);
                reply
            }
            Err(_) => b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                .to_vec(),
        }
    } else {
        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()
    };
    let _ = socket.write_all(&reply).await;
}
