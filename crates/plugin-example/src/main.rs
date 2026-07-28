//! A reference Kopuz plugin: it serves a folder of audio files.
//!
//! This exists to prove the extension point is genuinely generic — it reaches
//! no network, knows no service, and is what the host's integration test drives
//! end to end. Anything in the plugin host that this cannot exercise is
//! over-fitted and should be cut.
//!
//! Run it with `KOPUZ_PLUGIN_MEDIA_DIR` pointing at a folder of audio files;
//! it indexes them at startup, answers the protocol on stdio, and serves the
//! bytes from a loopback port it picks itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The protocol this plugin speaks; must equal the host's.
const PROTOCOL_VERSION: u32 = 1;

/// Extensions the indexer picks up. Deliberately short — this is a reference,
/// not a media library.
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "wav"];

#[derive(Clone)]
struct Item {
    id: String,
    title: String,
    path: PathBuf,
    size: u64,
}

struct Library {
    items: Vec<Item>,
    by_id: HashMap<String, usize>,
}

impl Library {
    fn index(root: &Path) -> Self {
        let mut items = Vec::new();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) => {
                log("warn", &format!("cannot read {}: {e}", root.display()));
                return Self {
                    items,
                    by_id: HashMap::new(),
                };
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if !is_audio {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(Item {
                // A stable id with no ':' or '/' — those would collide with the
                // host's ref parsing.
                id: format!("f{}", items.len()),
                title: name.to_string(),
                path,
                size,
            });
        }
        items.sort_by(|a, b| a.title.cmp(&b.title));
        let by_id = items
            .iter()
            .enumerate()
            .map(|(i, item)| (item.id.clone(), i))
            .collect();
        Self { items, by_id }
    }

    fn get(&self, id: &str) -> Option<&Item> {
        self.by_id.get(id).and_then(|i| self.items.get(*i))
    }

    fn track_json(&self, item: &Item) -> Value {
        json!({
            "item_id": item.id,
            "title": item.title,
            "artist": "Example plugin",
            "artists": ["Example plugin"],
            "album": "Local folder",
            "album_id": "folder",
            "duration_secs": 0,
            "khz": 44100,
            "bitrate": 0,
        })
    }
}

struct Plugin {
    library: Library,
    base_url: String,
    token: String,
}

impl Plugin {
    /// One protocol method. Returning `Err` becomes a JSON-RPC error with the
    /// given `kind`.
    fn call(&self, method: &str, params: &Value) -> Result<Value, (&'static str, String)> {
        match method {
            "initialize" => Ok(json!({
                "protocol": PROTOCOL_VERSION,
                "name": "Example",
                "version": env!("CARGO_PKG_VERSION"),
                "capabilities": {
                    "sync": true,
                    "favorites_sync": "Instant",
                },
                "auth_required": false,
                "data_base_url": self.base_url,
                "data_token": self.token,
            })),
            "ping" => Ok(Value::Null),
            "validate" => Ok(json!("Valid")),
            // Nothing to sign in to: the wizard is one step long.
            "auth_begin" | "auth_submit" => Ok(json!("Done")),
            "fetch_library" => Ok(json!({
                "albums": [{
                    "album_id": "folder",
                    "title": "Local folder",
                    "artist": "Example plugin",
                }],
                "tracks": self
                    .library
                    .items
                    .iter()
                    .map(|i| self.library.track_json(i))
                    .collect::<Vec<_>>(),
                "artist_images": [],
            })),
            "fetch_favorites" => Ok(json!([])),
            "search" => {
                let query = params
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                let tracks: Vec<Value> = self
                    .library
                    .items
                    .iter()
                    .filter(|i| i.title.to_lowercase().contains(&query))
                    .map(|i| self.library.track_json(i))
                    .collect();
                Ok(json!({ "tracks": tracks, "albums": [] }))
            }
            "resolve_stream" => {
                let item_id = params
                    .get("item_id")
                    .and_then(Value::as_str)
                    .ok_or(("invalid_input", "item_id is required".to_string()))?;
                let item = self
                    .library
                    .get(item_id)
                    .ok_or(("invalid_input", format!("no item {item_id}")))?;
                Ok(json!({
                    "url": format!("{}/a/{}/{}", self.base_url, self.token, item.id),
                    "content_length": item.size,
                }))
            }
            // Everything else is honestly unsupported rather than a silent
            // empty success — the host degrades the optional ones itself.
            other => Err((
                "unsupported",
                format!("the example plugin does not implement {other}"),
            )),
        }
    }
}

// ============================== byte server ==============================

/// A minimal HTTP/1.1 server for the audio bytes. The host's player only ever
/// consumes a URL, so this is how a plugin gets bytes into it: a plain GET
/// answered with 2xx, real bytes and an accurate `Content-Length`.
async fn serve_bytes(listener: TcpListener, plugin: Arc<Plugin>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let plugin = plugin.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_request(stream, plugin).await {
                log("debug", &format!("byte request failed: {e}"));
            }
        });
    }
}

async fn handle_request(mut stream: TcpStream, plugin: Arc<Plugin>) -> std::io::Result<()> {
    let mut request = String::new();
    BufReader::new(&mut stream).read_line(&mut request).await?;
    let target = request.split_whitespace().nth(1).unwrap_or_default();

    // `/a/<token>/<id>`. A wrong token is a 404, never a distinguishable error.
    let item = target
        .strip_prefix("/a/")
        .and_then(|rest| rest.split_once('/'))
        .filter(|(token, _)| *token == plugin.token)
        .and_then(|(_, id)| plugin.library.get(id));

    let Some(item) = item else {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };

    let mut file = tokio::fs::File::open(&item.path).await?;
    let length = file.metadata().await?.len();
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {length}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).await?;
        if read == 0 {
            return Ok(());
        }
        stream.write_all(&buf[..read]).await?;
    }
}

// ================================ stdio ==================================

/// Log to stderr. stdout carries the protocol and nothing else — a stray write
/// there corrupts the stream.
fn log(level: &str, message: &str) {
    #[expect(clippy::print_stderr, reason = "stderr is this plugin's only log sink")]
    {
        eprintln!("[{level}] {message}");
    }
}

fn error_response(id: u64, kind: &str, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32000, "message": message, "data": { "kind": kind } },
    })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let media_dir = std::env::var_os("KOPUZ_PLUGIN_MEDIA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("KOPUZ_PLUGIN_DATA_DIR").map(|d| PathBuf::from(d).join("media"))
        })
        .unwrap_or_else(|| PathBuf::from("media"));

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(e) => {
            log("error", &format!("cannot bind the byte server: {e}"));
            return std::process::ExitCode::FAILURE;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            log("error", &format!("cannot read the byte server port: {e}"));
            return std::process::ExitCode::FAILURE;
        }
    };

    let library = Library::index(&media_dir);
    log(
        "info",
        &format!(
            "indexed {} files in {}",
            library.items.len(),
            media_dir.display()
        ),
    );

    let plugin = Arc::new(Plugin {
        library,
        base_url: format!("http://127.0.0.1:{port}"),
        // Good enough for a reference: the port is loopback-only and the token
        // just stops another local process guessing the path.
        token: format!("{:x}", std::process::id() as u64 * 2_654_435_761),
    });

    let bytes = tokio::spawn(serve_bytes(listener, plugin.clone()));

    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            log("warn", "ignoring an unparseable line");
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        // No id: a notification, and `shutdown` is the one that matters.
        let Some(id) = request.get("id").and_then(Value::as_u64) else {
            if method == "shutdown" {
                break;
            }
            continue;
        };

        let response = match plugin.call(&method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((kind, message)) => error_response(id, kind, message),
        };
        let Ok(mut encoded) = serde_json::to_vec(&response) else {
            log("error", "cannot encode a response");
            continue;
        };
        encoded.push(b'\n');
        if stdout.write_all(&encoded).await.is_err() || stdout.flush().await.is_err() {
            break;
        }
    }

    bytes.abort();
    std::process::ExitCode::SUCCESS
}
