//! Rex WebSocket transport and bounded, pipelined control requests.
// Protocol interoperability informed by rexrs and rex-rune (MIT); see THIRD_PARTY.md.
use crate::program::Program;
use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};

pub const DATA_PROTOCOL: &str = "rex.data.terminal.v1";
pub const ENDED: &str = "Rex control connection ended";
const CONTROL_PROTOCOL: &str = "rex.control.v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_IN_FLIGHT: usize = 64;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<Box<dyn Io>>>;

pub async fn socket(endpoint: &str, path: &str, protocol: &str) -> Result<Socket> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let unix = endpoint
            .strip_prefix("unix://")
            .or_else(|| endpoint.strip_prefix("unix:"))
            .or_else(|| endpoint.starts_with('/').then_some(endpoint));
        let mut url = url::Url::parse(if unix.is_some() {
            "ws://localhost"
        } else {
            endpoint
        })?;
        let scheme = match url.scheme() {
            "http" | "ws" => "ws",
            "https" | "wss" => "wss",
            _ => bail!("Use a Unix socket path or an ssh://, ws:// or wss:// endpoint"),
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow!("Invalid endpoint scheme"))?;
        url.set_path(path);
        url.set_query(None);
        let io: Box<dyn Io> = if let Some(path) = unix {
            Box::new(
                tokio::net::UnixStream::connect(path)
                    .await
                    .context("Connect to Rex Unix socket")?,
            )
        } else {
            let tcp = tokio::net::TcpStream::connect((
                url.host_str().context("Missing endpoint host")?,
                url.port_or_known_default()
                    .context("Missing endpoint port")?,
            ))
            .await?;
            tcp.set_nodelay(true)?;
            Box::new(tcp)
        };
        let mut request = url.as_str().into_client_request()?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", protocol.parse()?);
        let config = WebSocketConfig::default()
            .max_message_size(Some(64 * 1024 * 1024))
            .max_frame_size(Some(16 * 1024 * 1024));
        let (socket, response) =
            tokio_tungstenite::client_async_tls_with_config(request, io, Some(config), None)
                .await?;
        ensure!(
            response
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|h| h.to_str().ok())
                == Some(protocol),
            "Rex did not negotiate {protocol}"
        );
        Ok(socket)
    })
    .await
    .context("Rex connection timed out")?
}

async fn text(socket: &mut Socket) -> Result<Value> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Ok(serde_json::from_str(&text)?),
            Some(Ok(Message::Close(_))) | None => bail!("Rex control connection closed"),
            Some(Err(error)) => return Err(error.into()),
            Some(Ok(_)) => socket.flush().await?,
        }
    }
}

pub async fn next_binary(socket: &mut Socket) -> Result<Vec<u8>> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => return Ok(bytes.to_vec()),
            Some(Ok(Message::Close(_))) | None => bail!("Rex terminal stream closed"),
            Some(Err(error)) => return Err(error.into()),
            Some(Ok(_)) => socket.flush().await?,
        }
    }
}

async fn send_with_deadline(
    socket: &mut Socket,
    message: Message,
    deadline: Duration,
) -> Result<()> {
    tokio::time::timeout(deadline, socket.send(message))
        .await
        .context("Rex WebSocket write timed out; request was not retried")??;
    Ok(())
}

pub async fn handshake(socket: &mut Socket, ticket: &str) -> Result<u64> {
    let ticket = URL_SAFE_NO_PAD.decode(ticket)?;
    ensure!(ticket.len() == 16, "Invalid data ticket length");
    let mut hello = vec![1];
    hello.extend(ticket);
    send_with_deadline(socket, Message::Binary(hello.into()), REQUEST_TIMEOUT).await?;
    let welcome = tokio::time::timeout(REQUEST_TIMEOUT, next_binary(socket)).await??;
    ensure!(
        welcome.len() == 32 && welcome[0] == 0x81,
        "Rex rejected terminal handshake"
    );
    let nonce = u64::from_le_bytes(welcome[24..32].try_into()?);
    ensure!(nonce != 0, "Invalid terminal nonce");
    Ok(nonce)
}

type Reply = oneshot::Sender<Result<Value, String>>;
pub type Response = oneshot::Receiver<Result<Value, String>>;

struct Request {
    method: String,
    payload: Value,
    reply: Reply,
}

#[derive(Clone)]
pub struct Rpc {
    requests: mpsc::Sender<Request>,
}

impl Rpc {
    /// Queue requests in wire order. Both this queue and the outstanding response map are bounded.
    pub async fn enqueue(&self, method: &str, payload: Value) -> Result<Response> {
        let (reply, rx) = oneshot::channel();
        self.requests
            .send(Request {
                method: method.into(),
                payload,
                reply,
            })
            .await
            .map_err(|_| anyhow!(ENDED))?;
        Ok(rx)
    }

    pub async fn call(&self, method: &str, payload: Value) -> Result<Value> {
        let rx = self.enqueue(method, payload).await?;
        rx.await.context(ENDED)?.map_err(anyhow::Error::msg)
    }
}

pub struct Control {
    pub rpc: Rpc,
    pub client_id: String,
    pub reconnect_token: Option<String>,
    /// Unbounded, so that reading replies never waits on whoever reads these. The
    /// server ends a connection that falls far behind.
    pub events: mpsc::UnboundedReceiver<Event>,
    pub task: tokio::task::JoinHandle<Result<()>>,
}

impl Drop for Control {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Control {
    pub async fn connect(endpoint: &str, token: Option<&str>) -> Result<Self> {
        let mut socket = socket(endpoint, "/api/control", CONTROL_PROTOCOL).await?;
        let mut hello = json!({
            "type": "hello",
            "api_revision": 13,
            "api_min_revision": 10,
            "client": {
                "kind": "app",
                "platform": "linux",
                "name": "Rex Lite",
                "version": env!("CARGO_PKG_VERSION"),
                "label": "Rex Lite",
            },
        });
        if let Some(token) = token {
            hello["reconnect_token"] = json!(token);
        }
        send_with_deadline(
            &mut socket,
            Message::Text(hello.to_string().into()),
            REQUEST_TIMEOUT,
        )
        .await?;
        let welcome = tokio::time::timeout(REQUEST_TIMEOUT, text(&mut socket)).await??;
        ensure!(
            welcome["type"] == "welcome" && welcome["protocol"] == CONTROL_PROTOCOL,
            "Invalid Rex control welcome"
        );
        ensure!(
            (10..=13).contains(&welcome["api_revision"].as_u64().unwrap_or(0)),
            "Unsupported Rex API revision"
        );
        let client_id = welcome["client"]["id"]
            .as_str()
            .context("Missing Rex client identity")?
            .to_owned();
        let reconnect_token = welcome["client"]["reconnect_token"]
            .as_str()
            .map(str::to_owned);
        let max_message = welcome["limits"]["max_message_bytes"]
            .as_u64()
            .unwrap_or(65536) as usize;
        let (requests, receiver) = mpsc::channel(MAX_IN_FLIGHT);
        let (sender, events) = mpsc::unbounded_channel();
        let task = tokio::spawn(control_loop(socket, receiver, sender, max_message));
        Ok(Self {
            rpc: Rpc { requests },
            client_id,
            reconnect_token,
            events,
            task,
        })
    }
}

/// What the server says has changed, of the things this client keeps up with.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// The sessions, their tabs and panes, or the host list.
    Structure,
    /// A terminal's title.
    Title { block: String, title: String },
    /// The program in a terminal's foreground, by the name `program` gives it.
    Program {
        block: String,
        program: Option<Program>,
    },
    /// What the programs in a terminal say they are doing.
    Status { block: String },
}

/// The icon of a process, if any of its names has one. The most telling come first:
/// the script it runs, what it was started as, and its executable, which may be a
/// file named for its version.
pub fn program(process: &Value) -> Option<Program> {
    let named = ["script", "argv0", "name"].map(|key| process[key].as_str());
    Program::named(named.into_iter().flatten())
}

fn event(message: &Value) -> Option<Event> {
    let event = &message["payload"];
    let event = if event["type"] == "session_event" {
        &event["payload"]["event"]
    } else {
        event
    };
    let block = &event["payload"];
    let id = || block["block_id"].as_str().map(str::to_owned);
    match (event["type"].as_str()?, block["name"].as_str()) {
        (
            "session_created"
            | "session_destroyed"
            | "session_moved"
            | "session_label_changed"
            | "session_view_changed"
            | "hosts_changed"
            | "block_closed"
            | "block_owner_changed",
            _,
        ) => Some(Event::Structure),
        ("block_event", Some("title_changed")) => Some(Event::Title {
            block: id()?,
            title: block["payload"]["title"].as_str()?.into(),
        }),
        ("block_event", Some("process_changed")) => Some(Event::Program {
            block: id()?,
            program: program(&block["payload"]["foreground"]),
        }),
        ("block_event", Some("program_status_changed" | "program_status_removed")) => {
            Some(Event::Status { block: id()? })
        }
        _ => None,
    }
}

fn response_result(message: &Value) -> Result<Value, String> {
    let error = &message["error"];
    if error.is_object() {
        return Err(format!(
            "{}: {}",
            error["code"].as_str().unwrap_or("request_failed"),
            error["message"].as_str().unwrap_or("Request failed")
        ));
    }
    Ok(message.get("payload").cloned().unwrap_or_else(|| json!({})))
}

async fn control_loop(
    mut socket: Socket,
    mut outgoing: mpsc::Receiver<Request>,
    events: mpsc::UnboundedSender<Event>,
    max_message: usize,
) -> Result<()> {
    let mut pending: HashMap<String, (Instant, Reply)> = HashMap::new();
    let mut serial = 0_u64;
    let mut clock = tokio::time::interval(Duration::from_millis(250));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            request = outgoing.recv(), if pending.len() < MAX_IN_FLIGHT => {
                let Some(Request { method, payload, reply }) = request else {
                    let _ = tokio::time::timeout(REQUEST_TIMEOUT, socket.close(None)).await;
                    return Ok(());
                };
                serial += 1;
                let id = serial.to_string();
                let text =
                    json!({"type": "request", "id": id, "method": method, "payload": payload})
                        .to_string();
                if text.len() > max_message {
                    let _ = reply.send(Err(format!(
                        "Request exceeds server limit ({max_message} bytes)"
                    )));
                    continue;
                }
                send_with_deadline(&mut socket, Message::Text(text.into()), REQUEST_TIMEOUT)
                    .await?;
                pending.insert(id, (Instant::now(), reply));
            }
            message = text(&mut socket) => {
                let message = message?;
                if message["type"] == "response" {
                    let reply = message["id"].as_str().and_then(|id| pending.remove(id));
                    if let Some((_, reply)) = reply {
                        let _ = reply.send(response_result(&message));
                    }
                } else if message["type"] == "event"
                    && let Some(event) = event(&message)
                {
                    let _ = events.send(event);
                }
            }
            _ = clock.tick(), if !pending.is_empty() => {
                // A timed-out operation is never replayed: reconnect from a fresh server snapshot.
                if pending.values().any(|(sent, _)| sent.elapsed() > REQUEST_TIMEOUT) {
                    bail!("Rex request timed out; input was not retried");
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/wire.rs"]
mod tests;
