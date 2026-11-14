//! Rex WebSocket transport and bounded, pipelined control requests.
// Protocol interoperability informed by rexrs and rex-rune (MIT); see THIRD_PARTY.md.
use crate::program::Program;
use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, Stream, StreamExt, stream};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::{collections::HashMap, fmt, net::Ipv6Addr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Notify, mpsc, oneshot},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use url::Url;

pub const DATA_PROTOCOL: &str = "rex.data.terminal.v1";
pub const ENDED: &str = "Rex control connection ended";
const CONTROL_PROTOCOL: &str = "rex.control.v1";
const MAX_IN_FLIGHT: usize = 64;
/// How long one address may take to answer, and to agree to a WebSocket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one write may take, or one answer in a handshake.
const STEP_TIMEOUT: Duration = Duration::from_secs(8);
/// How often the control connection is looked over. A server not heard from between
/// two looks is pinged, and one still not heard from by the next is taken for gone.
const PING_EVERY: Duration = Duration::from_secs(10);
/// How many looks a request may wait for its answer: 40 s at the least. The server
/// itself gives a key's Lua handler 5 s and an action from its configuration 30 s.
const REQUEST_TICKS: u64 = 5;
/// How long a goodbye may take to leave. Quitting never waits longer on a dead link.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<Box<dyn Io>>>;

/// Why a connection is over, where the server or the link has said. It rides in the
/// error, for whoever must decide what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The server closed the control connection, with a WebSocket status and its reason.
    Closed { code: u16, reason: String },
    /// The server and this client have no API revision in common. Trying again will
    /// not help; the words say which of them is behind.
    Incompatible(String),
    /// The server closed a terminal's stream, likewise.
    StreamClosed { code: u16, reason: String },
    /// The terminal itself has closed. There is nothing left to reconnect to.
    BlockClosed,
    /// The server refused a stream's handshake, with the reason byte of its REJECT.
    Rejected(u8),
    /// The link went away with no close frame.
    Dropped,
    /// Nothing comes from the server, not even an answer to a ping.
    Silent,
}

impl Ended {
    /// What a close frame says, on the control connection or on a terminal's `stream`.
    fn closed(frame: Option<CloseFrame>, stream: bool) -> Self {
        // A close with no status is 1005, by the WebSocket standard.
        let (code, reason) = frame.map_or((1005, String::new()), |frame| {
            (frame.code.into(), frame.reason.to_string())
        });
        match (stream, code) {
            (true, 4000) => Self::BlockClosed,
            (true, _) => Self::StreamClosed { code, reason },
            (false, 1008) if reason.starts_with("no common API revision") => {
                Self::Incompatible(reason)
            }
            (false, _) => Self::Closed { code, reason },
        }
    }
}

/// A close this client has no words for, with its status and reason as they came.
fn unnamed(f: &mut fmt::Formatter<'_>, what: &str, code: u16, reason: &str) -> fmt::Result {
    match reason {
        "" => write!(f, "{what} (status {code})"),
        reason => write!(f, "{what} (status {code}: {reason})"),
    }
}

impl fmt::Display for Ended {
    /// The server's codes and reasons, as its API reference lists them, in plain words.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed { code, reason } => match (*code, reason.as_str()) {
                (1000 | 1005, _) => f.write_str("The server closed the connection"),
                (1001, "keepalive timeout") => {
                    f.write_str("The server stopped hearing from this client")
                }
                (1002 | 1003 | 1007, _) => {
                    f.write_str("The server could not read what this client sent")
                }
                (1008, "event backlog exceeded") => {
                    f.write_str("This client fell too far behind the server's news")
                }
                (1009, "event exceeds message limit") => {
                    f.write_str("The server had news too large to send")
                }
                (1009, _) => f.write_str("A request was larger than the server accepts"),
                _ => unnamed(f, "The server closed the connection", *code, reason),
            },
            Self::Incompatible(why) => {
                write!(f, "This client and the server cannot talk: {why}")
            }
            Self::StreamClosed { code, reason } => match (*code, reason.as_str()) {
                (1000 | 1005, _) => f.write_str("The terminal's stream ended"),
                (1001, "keepalive timeout") => {
                    f.write_str("The server stopped hearing from this terminal's stream")
                }
                (1001, "data provider canceled") => {
                    f.write_str("The server stopped the terminal's stream")
                }
                (1008, "data provider unavailable") => {
                    f.write_str("The terminal has no stream of this kind")
                }
                (1011, "data provider failed") => f.write_str("The terminal failed on the server"),
                (1013, "data reconnect required") => {
                    f.write_str("The server wants the terminal's stream started afresh")
                }
                (_, "data handshake rejected") => {
                    f.write_str("The server refused the terminal's stream")
                }
                _ => unnamed(f, "The server closed the terminal's stream", *code, reason),
            },
            Self::BlockClosed => f.write_str("The terminal has closed"),
            Self::Rejected(reason) => match reason {
                1 => f.write_str("The server no longer knows the terminal stream's ticket"),
                2 => f.write_str("This client already has a stream of this terminal"),
                3 => f.write_str("The terminal's stream is gone; the terminal may have closed"),
                4 => f.write_str("The server has no room for another stream just now"),
                5 => f.write_str("The server did not get the terminal stream's hello in time"),
                6 => f.write_str("The server failed to open the terminal's stream"),
                7 => f.write_str("The ticket was for another kind of stream"),
                _ => write!(
                    f,
                    "The server refused the terminal's stream (reason {reason})"
                ),
            },
            Self::Dropped => f.write_str("The connection to the server was lost"),
            Self::Silent => f.write_str("The server stopped answering"),
        }
    }
}

impl std::error::Error for Ended {}

/// A WebSocket error as the end it stands for, where it stands for one.
fn lost(error: tungstenite::Error) -> anyhow::Error {
    use tungstenite::{Error, error::ProtocolError::ResetWithoutClosingHandshake};
    match error {
        Error::ConnectionClosed
        | Error::AlreadyClosed
        | Error::Protocol(ResetWithoutClosingHandshake) => Ended::Dropped.into(),
        // The system's own word for it is worth keeping.
        Error::Io(error) => anyhow::Error::new(error).context(Ended::Dropped),
        error => error.into(),
    }
}

/// One address a WebSocket can be opened to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A Unix socket on this machine.
    Unix(PathBuf),
    /// A `ws://` or `wss://` address, with nothing after its port.
    Net(Url),
}

impl fmt::Display for Target {
    /// Written as an endpoint that stands for this target and no other.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) if path.is_absolute() => write!(f, "{}", path.display()),
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
            Self::Net(url) => write!(f, "{url}"),
        }
    }
}

/// The addresses an endpoint stands for, in the order to try them. A socket or a
/// URL is one address. A bare host name is tried as Rex tries it: HTTPS on port 443,
/// HTTPS on 7297, then HTTP on 7297. A bare host and port is tried with TLS, then without.
pub fn targets(endpoint: &str) -> Result<Vec<Target>> {
    // Rex writes a socket as a URL, its path percent-encoded.
    if let Some(path) = endpoint.strip_prefix("unix://") {
        let decoded = percent_decode_str(path).decode_utf8();
        let path = decoded.as_deref().unwrap_or(path);
        return Ok(vec![Target::Unix(path.into())]);
    }
    let path = endpoint.strip_prefix("unix:");
    if let Some(path) = path.or_else(|| endpoint.starts_with('/').then_some(endpoint)) {
        return Ok(vec![Target::Unix(path.into())]);
    }
    if endpoint.contains("://") {
        let mut url = Url::parse(endpoint)?;
        let scheme = match url.scheme() {
            "http" | "ws" => "ws",
            "https" | "wss" => "wss",
            _ => {
                bail!("Use a Unix socket path, a host name, or an ssh://, ws:// or wss:// endpoint")
            }
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow!("Invalid endpoint scheme"))?;
        url.set_path("");
        url.set_query(None);
        url.set_fragment(None);
        return Ok(vec![Target::Net(url)]);
    }
    // An IPv6 address may come without its brackets, and a URL cannot hold it so.
    let bare = match endpoint.parse::<Ipv6Addr>() {
        Ok(address) => format!("[{address}]"),
        Err(_) => endpoint.to_owned(),
    };
    // Under a scheme with no port of its own, so that any port found was written.
    let named = Url::parse(&format!("rex://{bare}")).context("Invalid endpoint")?;
    let host = named.host_str().filter(|host| !host.is_empty());
    let host = host.context("Missing endpoint host")?;
    let at = |scheme: &str, port: u16| {
        let url = Url::parse(&format!("{scheme}://{host}:{port}")).context("Invalid endpoint")?;
        Ok(Target::Net(url))
    };
    match named.port() {
        Some(port) => [at("wss", port), at("ws", port)].into_iter().collect(),
        None => [at("wss", 443), at("wss", 7297), at("ws", 7297)]
            .into_iter()
            .collect(),
    }
}

/// The host and port to connect to for a network address. An IPv6 address is written
/// in brackets, and looked up without them.
fn host_port(url: &Url) -> Result<(&str, u16)> {
    let host = url.host_str().context("Missing endpoint host")?;
    let port = url.port_or_known_default();
    let port = port.context("Missing endpoint port")?;
    Ok((host.trim_start_matches('[').trim_end_matches(']'), port))
}

async fn open(target: &Target, path: &str, protocol: &str) -> Result<Socket> {
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (io, mut url): (Box<dyn Io>, Url) = match target {
            Target::Unix(socket) => {
                let unix = tokio::net::UnixStream::connect(socket).await;
                let unix = unix.context("Connect to Rex Unix socket")?;
                (Box::new(unix), Url::parse("ws://localhost")?)
            }
            Target::Net(url) => {
                let tcp = tokio::net::TcpStream::connect(host_port(url)?).await?;
                tcp.set_nodelay(true)?;
                (Box::new(tcp), url.clone())
            }
        };
        url.set_path(path);
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

/// Open a WebSocket at the first of an endpoint's addresses that answers, and say
/// which that was.
async fn reach(endpoint: &str, path: &str, protocol: &str) -> Result<(Socket, Target)> {
    let targets = targets(endpoint)?;
    let several = targets.len() > 1;
    let mut failure = anyhow!("This endpoint has no address");
    for target in targets {
        match open(&target, path, protocol).await {
            Ok(socket) => return Ok((socket, target)),
            Err(error) => failure = error,
        }
    }
    // The last address's failure alone would not say that others were tried.
    Err(match several {
        true => failure.context(format!("No Rex server answered at {endpoint}")),
        false => failure,
    })
}

/// Open a WebSocket to an endpoint. A terminal's stream is opened many times, so give
/// this the one address its control connection found, `Control::endpoint`.
pub async fn socket(endpoint: &str, path: &str, protocol: &str) -> Result<Socket> {
    Ok(reach(endpoint, path, protocol).await?.0)
}

/// The next message of any kind. A close and a lost link are errors that say which,
/// in the words for the control connection or for a terminal's `stream`.
async fn message(socket: &mut Socket, stream: bool) -> Result<Message> {
    match socket.next().await {
        Some(Ok(Message::Close(frame))) => Err(Ended::closed(frame, stream).into()),
        Some(Ok(message)) => {
            // Reading a ping only queues its answer. This sends it.
            if message.is_ping() {
                socket.flush().await.map_err(lost)?;
            }
            Ok(message)
        }
        Some(Err(error)) => Err(lost(error)),
        None => Err(Ended::Dropped.into()),
    }
}

async fn text(socket: &mut Socket) -> Result<Value> {
    loop {
        if let Message::Text(text) = message(socket, false).await? {
            return Ok(serde_json::from_str(&text)?);
        }
    }
}

pub async fn next_binary(socket: &mut Socket) -> Result<Vec<u8>> {
    loop {
        if let Message::Binary(bytes) = message(socket, true).await? {
            return Ok(bytes.to_vec());
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
        .context("Rex WebSocket write timed out; request was not retried")?
        .map_err(lost)
}

pub async fn handshake(socket: &mut Socket, ticket: &str) -> Result<u64> {
    let ticket = URL_SAFE_NO_PAD.decode(ticket)?;
    ensure!(ticket.len() == 16, "Invalid data ticket length");
    let mut hello = vec![1];
    hello.extend(ticket);
    send_with_deadline(socket, Message::Binary(hello.into()), STEP_TIMEOUT).await?;
    let welcome = tokio::time::timeout(STEP_TIMEOUT, next_binary(socket)).await;
    let welcome = welcome.context("The server did not answer the terminal stream's hello")??;
    // A REJECT, with its reason.
    if let [0x82, reason, ..] = welcome[..] {
        return Err(Ended::Rejected(reason).into());
    }
    ensure!(
        welcome.len() == 32 && welcome[0] == 0x81,
        "Rex answered the terminal stream's hello with something else"
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
    /// The one address that answered, of those the endpoint stood for, as an endpoint.
    /// Terminal streams go straight to it.
    pub endpoint: String,
    /// Unbounded, so that reading replies never waits on whoever reads these. The
    /// server ends a connection that falls far behind.
    pub events: mpsc::UnboundedReceiver<Event>,
    /// Ends with the connection, and with an `Ended` in its error if it is known why.
    pub task: tokio::task::JoinHandle<Result<()>>,
    closing: Arc<Notify>,
    /// Keeps `events` open after the connection ends, so that its end is learned
    /// from `task`, which knows the reason, and never from here, which does not.
    _events: mpsc::UnboundedSender<Event>,
}

impl Drop for Control {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Control {
    pub async fn connect(endpoint: &str, token: Option<&str>) -> Result<Self> {
        let (mut socket, target) = reach(endpoint, "/api/control", CONTROL_PROTOCOL).await?;
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
            STEP_TIMEOUT,
        )
        .await?;
        // A server that cannot serve this client closes instead, and says why.
        let welcome = tokio::time::timeout(STEP_TIMEOUT, text(&mut socket)).await;
        let welcome = welcome.context("The server did not answer this client's hello")??;
        ensure!(
            welcome["type"] == "welcome" && welcome["protocol"] == CONTROL_PROTOCOL,
            "Invalid Rex control welcome"
        );
        let revision = welcome["api_revision"].as_u64().unwrap_or(0);
        if !(10..=13).contains(&revision) {
            let why = format!(
                "the server chose API revision {revision}, and this client asked for 10 to 13"
            );
            return Err(Ended::Incompatible(why).into());
        }
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
        let closing = Arc::new(Notify::new());
        let mut clock =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let ticks = Box::pin(stream::unfold(clock, |mut clock| async {
            clock.tick().await;
            Some(((), clock))
        }));
        let task = tokio::spawn(control_loop(
            socket,
            receiver,
            sender.clone(),
            max_message,
            closing.clone(),
            ticks,
        ));
        Ok(Self {
            rpc: Rpc { requests },
            client_id,
            reconnect_token,
            endpoint: target.to_string(),
            events,
            task,
            closing,
            _events: sender,
        })
    }

    /// Say goodbye, so that the server lets this client go at once and does not keep
    /// its place for a minute. Waits a moment for the word to leave, and never long.
    pub async fn close(&mut self) {
        self.closing.notify_one();
        // A task that has ended may have been asked for its result already.
        if !self.task.is_finished() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, &mut self.task).await;
        }
    }
}

/// What the server says has changed, of the things this client keeps up with.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// The sessions, their tabs and panes, or the host list.
    Structure,
    /// A terminal's title.
    Title { block: String, title: String },
    /// The program in a terminal's foreground, by the name `program` gives it, and
    /// its process.
    Program {
        block: String,
        program: Option<Program>,
        pid: Option<u64>,
    },
    /// What the programs in a terminal say they are doing.
    Status { block: String },
    /// A program in a terminal has copied text: all of it, or if it is long the
    /// `id` to ask for it by.
    Copied {
        block: String,
        text: Option<String>,
        id: u64,
    },
    /// A program in a terminal has rung the bell.
    Bell { block: String },
    /// One has a notice for the desktop. Either part of it may be empty.
    Notice {
        block: String,
        title: String,
        body: String,
    },
}

/// The icon of a process, if any of its names has one. The most telling come first:
/// the script it runs, what it was started as, and its executable, which may be a
/// file named for its version.
pub fn program(process: &Value) -> Option<Program> {
    let named = ["script", "argv0", "name"].map(|key| process[key].as_str());
    Program::named(named.into_iter().flatten())
}

/// The process a terminal started, if that is a shell. While it is also the one in
/// the foreground it is at its prompt, and closing the terminal ends nothing else.
pub fn shell(child: &Value) -> Option<u64> {
    const SHELLS: [&str; 13] = [
        "sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "tcsh", "csh", "nu", "pwsh", "elvish",
        "xonsh",
    ];
    let name = child["name"].as_str()?;
    child["pid"].as_u64().filter(|_| SHELLS.contains(&name))
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
            pid: block["payload"]["foreground"]["pid"].as_u64(),
        }),
        ("block_event", Some("program_status_changed" | "program_status_removed")) => {
            Some(Event::Status { block: id()? })
        }
        ("block_event", Some("clipboard_written")) => Some(Event::Copied {
            block: id()?,
            text: block["payload"]["text"].as_str().map(str::to_owned),
            id: block["payload"]["id"].as_u64().unwrap_or_default(),
        }),
        ("block_event", Some("bell")) => Some(Event::Bell { block: id()? }),
        ("block_event", Some("desktop_notification")) => {
            let text = |key| block["payload"][key].as_str().unwrap_or_default();
            Some(Event::Notice {
                block: id()?,
                title: text("title").to_owned(),
                body: text("body").to_owned(),
            })
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

/// A request on its way, and the look it was sent before.
struct Pending {
    sent: u64,
    method: String,
    reply: Reply,
}

/// Tell the server this client is leaving, so that it does not hold the client's place.
async fn goodbye(socket: &mut Socket) -> Result<()> {
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    };
    let leave = async {
        socket.close(Some(frame)).await?;
        // Written is not yet sent: a tunnel that ends with this client may still be
        // holding it. The server's own close, or the link's end, says that it came.
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
        Ok::<_, tungstenite::Error>(())
    };
    // The link may be dead, and the one leaving is not to be kept.
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, leave).await;
    Ok(())
}

/// Carry requests and their answers until the connection ends. Each of `ticks` is a
/// look over the link and over the requests still waiting.
async fn control_loop(
    mut socket: Socket,
    mut outgoing: mpsc::Receiver<Request>,
    events: mpsc::UnboundedSender<Event>,
    max_message: usize,
    closing: Arc<Notify>,
    mut ticks: impl Stream<Item = ()> + Unpin,
) -> Result<()> {
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let (mut serial, mut tick) = (0_u64, 0_u64);
    // Whether the server has been heard since the last look, and pinged since last heard.
    let (mut heard, mut pinged) = (false, false);
    loop {
        tokio::select! {
            // In this order, so that a look never finds silent a server whose words
            // were only waiting to be read.
            biased;
            _ = closing.notified() => return goodbye(&mut socket).await,
            request = outgoing.recv(), if pending.len() < MAX_IN_FLIGHT => {
                let Some(Request { method, payload, reply }) = request else {
                    return goodbye(&mut socket).await;
                };
                serial += 1;
                let id = serial.to_string();
                let text =
                    json!({"type": "request", "id": id, "method": method, "payload": payload})
                        .to_string();
                // Sent, it would have the server close the connection.
                if text.len() > max_message {
                    let _ = reply.send(Err(format!(
                        "Request exceeds server limit ({max_message} bytes)"
                    )));
                    continue;
                }
                send_with_deadline(&mut socket, Message::Text(text.into()), STEP_TIMEOUT)
                    .await?;
                pending.insert(id, Pending { sent: tick, method, reply });
            }
            incoming = message(&mut socket, false) => {
                heard = true;
                let Message::Text(text) = incoming? else { continue };
                let message: Value = serde_json::from_str(&text)?;
                if message["type"] == "response" {
                    let reply = message["id"].as_str().and_then(|id| pending.remove(id));
                    if let Some(Pending { reply, .. }) = reply {
                        let _ = reply.send(response_result(&message));
                    }
                } else if message["type"] == "event"
                    && let Some(event) = event(&message)
                {
                    let _ = events.send(event);
                }
            }
            _ = ticks.next() => {
                tick += 1;
                // A request the server does not answer fails alone, and the link is
                // judged by its pings. An answer that comes after all is to a number
                // no longer known, and is passed over.
                for (_, late) in pending.extract_if(|_, request| tick - request.sent >= REQUEST_TICKS) {
                    let _ = late.reply.send(Err(format!(
                        "The server took too long to answer {}; it was not asked again",
                        late.method
                    )));
                }
                if std::mem::take(&mut heard) {
                    pinged = false;
                } else if pinged {
                    return Err(Ended::Silent.into());
                } else {
                    let ping = Message::Ping(Default::default());
                    send_with_deadline(&mut socket, ping, STEP_TIMEOUT).await?;
                    pinged = true;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/wire.rs"]
mod tests;
