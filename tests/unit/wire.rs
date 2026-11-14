use super::*;
use futures_util::FutureExt;

/// An event as the server sends one, of a session if it is of one.
fn sent(session: Option<&str>, kind: &str, payload: Value) -> Value {
    let event = json!({"type": kind, "payload": payload});
    let event = match session {
        Some(session) => {
            let payload = json!({"session_id": session, "event": event});
            json!({"type": "session_event", "payload": payload})
        }
        None => event,
    };
    json!({"type": "event", "seq": 7, "payload": event})
}

#[test]
fn keeps_up_with_sessions_and_their_views_by_what_the_news_carries() {
    let created = json!({"session": {"session_id": "session:2", "label": "Second"}});
    let created = event(&sent(None, "session_created", created));
    let (session, name) = ("session:2".to_owned(), "Second".to_owned());
    assert_eq!(created, Some(Event::Created { session, name }));
    let destroyed = sent(None, "session_destroyed", json!({"session_id": "s"}));
    let session = "s".to_owned();
    assert_eq!(event(&destroyed), Some(Event::Destroyed { session }));
    let moved = |payload: Value, before: Option<&str>| {
        let (session, before) = ("s".to_owned(), before.map(str::to_owned));
        let moved = event(&sent(None, "session_moved", payload));
        assert_eq!(moved, Some(Event::Moved { session, before }));
    };
    let before = json!({"session_id": "s", "before_session_id": "t"});
    moved(before, Some("t"));
    // Last of all, there is none it comes before.
    moved(json!({"session_id": "s"}), None);
    // A view's news is its revision, and the session is the one it came in.
    let viewed = sent(Some("s"), "session_view_changed", json!({"revision": 16}));
    let (session, revision) = ("s".to_owned(), 16);
    assert_eq!(event(&viewed), Some(Event::Viewed { session, revision }));
    let hosts = sent(None, "hosts_changed", json!({"hash": "4f53"}));
    let hash = "4f53".to_owned();
    assert_eq!(event(&hosts), Some(Event::Hosts { hash }));
    // What a view holds is told by its revision, and is not taken twice.
    let label = json!({"window_id": "w", "label": "Named"});
    let owner = json!({"block_id": "b", "owner": "client:1", "reason": "take"});
    for (kind, payload) in [
        ("window_label_changed", label),
        ("active_window_changed", json!({"window_id": "w"})),
        ("session_label_changed", json!({"label": "Renamed"})),
        ("block_owner_changed", owner),
        ("block_closed", json!({"block_id": "b"})),
        ("client_connected", json!({"client_id": "client:1"})),
    ] {
        assert_eq!(event(&sent(Some("s"), kind, payload)), None, "{kind}");
    }
    let joined = sent(None, "client_joined", json!({"client": {}}));
    assert_eq!(event(&joined), None);
}

#[test]
fn keeps_up_with_what_each_terminal_says_of_itself() {
    let block = |name: &str, payload: Value| {
        let payload = json!({
            "block_id": "b",
            "creator_name": "com.superlogical.terminal",
            "name": name,
            "payload": payload,
        });
        event(&sent(Some("s"), "block_event", payload))
    };
    let news = |news| {
        let block = "b".to_owned();
        Some(Event::Block { block, news })
    };
    let owner = json!({"pid": 2455975, "name": "zsh"});
    let changed = block(
        "title_changed",
        json!({"title": "ada@studio: ~", "owner": owner}),
    );
    assert_eq!(changed, news(News::Title("ada@studio: ~".into())));
    // A program has the icon of its script, of what it was started as, or of its executable.
    let program = |foreground: Value, icon: Option<&str>| {
        let pid = foreground["pid"].as_u64();
        let event = block(
            "process_changed",
            json!({"generation": 2, "foreground": foreground}),
        );
        let program = Program::named(icon);
        assert_eq!(event, news(News::Program { program, pid }));
    };
    program(
        json!({"name": "2.1.325", "argv0": "claude"}),
        Some("claude-code"),
    );
    let script = "/opt/lib/pi-coding-agent/dist/cli.js";
    let script = json!({"name": "node", "argv0": "node", "script": script});
    program(script, Some("pi"));
    program(json!({"name": "zsh", "argv0": "-zsh", "pid": 7}), None);
    // A terminal that started with a shell, and one that started with something else.
    assert_eq!(shell(&json!({"name": "zsh", "pid": 7})), Some(7));
    assert_eq!(shell(&json!({"name": "vim", "pid": 7})), None);
    program(Value::Null, None);
    // What a program reports comes whole, under its name, and what is cleared by
    // name alone. The program's own record has no name.
    let record = json!({"state": "blocked", "kind": "permission", "msg": "Run it?"});
    let reported = json!({"id": "", "record": record, "reason": "report"});
    let status = News::Status(vec![(String::new(), Some(record))]);
    assert_eq!(block("program_status_changed", reported), news(status));
    let cleared = json!({"ids": ["", "build/test"], "reason": "clear"});
    let status = News::Status(vec![(String::new(), None), ("build/test".into(), None)]);
    assert_eq!(block("program_status_removed", cleared), news(status));
    // Text a program copies comes with the news of it, unless it is long.
    let copied = |text: Option<&str>, id| {
        let text = text.map(str::to_owned);
        news(News::Copied { text, id })
    };
    let short = json!({"text": "hello", "id": 3, "bytes": 5});
    assert_eq!(block("clipboard_written", short), copied(Some("hello"), 3));
    let long = json!({"id": 4, "bytes": 200_000});
    assert_eq!(block("clipboard_written", long), copied(None, 4));
    // A program may ring its terminal's bell, or have a notice for the desktop.
    assert_eq!(block("bell", json!({})), news(News::Bell));
    let notice = block("desktop_notification", json!({"title": "", "body": "hi"}));
    let (title, body) = (String::new(), "hi".to_owned());
    assert_eq!(notice, news(News::Notice { title, body }));
    assert_eq!(block("size_changed", json!({})), None);
}

#[tokio::test]
async fn nonreading_peer_cannot_stall_a_control_write_forever() {
    let (stream, _nonreading_peer) = tokio::io::duplex(8);
    let io: Box<dyn Io> = Box::new(stream);
    let mut socket = WebSocketStream::from_raw_socket(
        tokio_tungstenite::MaybeTlsStream::Plain(io),
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let error = send_with_deadline(
        &mut socket,
        Message::Text("a".repeat(256).into()),
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("write timed out"));
}

#[test]
fn an_endpoint_stands_for_the_addresses_to_try_in_order() {
    let unix = |path: &str| vec![Target::Unix(path.into())];
    let net = |urls: &[&str]| -> Vec<Target> {
        let urls = urls.iter().map(|url| Url::parse(url).unwrap());
        urls.map(Target::Net).collect()
    };
    let of = |endpoint: &str| targets(endpoint).unwrap();
    // A socket, in each way of writing one. Rex writes the first, percent-encoded.
    let socket = unix("/run/rex/server.sock");
    assert_eq!(of("unix:///run/rex/server.sock"), socket);
    assert_eq!(of("unix:/run/rex/server.sock"), socket);
    assert_eq!(of("/run/rex/server.sock"), socket);
    assert_eq!(of("unix://here.sock"), unix("here.sock"));
    assert_eq!(of("unix:here.sock"), unix("here.sock"));
    let encoded = "unix:///Users/me/Application%20Support/rex/100%/caf%C3%A9.sock";
    let decoded = "/Users/me/Application Support/rex/100%/café.sock";
    assert_eq!(of(encoded), unix(decoded));
    assert_eq!(of("/tmp/a%20b.sock"), unix("/tmp/a%20b.sock"));
    // A URL is one address, whatever follows its port.
    assert_eq!(of("ws://box:7297"), net(&["ws://box:7297"]));
    assert_eq!(of("http://10.0.0.5:7297/"), net(&["ws://10.0.0.5:7297"]));
    let long = "wss://box.ts.net/api/control?x=1#y";
    assert_eq!(of(long), net(&["wss://box.ts.net"]));
    assert_eq!(of("https://box.ts.net"), net(&["wss://box.ts.net"]));
    let six = "https://[fd7a::1]:7297";
    assert_eq!(of(six), net(&["wss://[fd7a::1]:7297"]));
    // A bare name is tried as Rex tries it.
    let rex = |host: &str| {
        let urls = ["wss://{}", "wss://{}:7297", "ws://{}:7297"].map(|url| url.replace("{}", host));
        net(&urls.each_ref().map(String::as_str))
    };
    assert_eq!(of("studio.tail1234.ts.net"), rex("studio.tail1234.ts.net"));
    assert_eq!(of("Studio"), rex("studio"));
    assert_eq!(of("192.168.1.20"), rex("192.168.1.20"));
    assert_eq!(of("[fd7a:115c:a1e0::2]"), rex("[fd7a:115c:a1e0::2]"));
    assert_eq!(of("fd7a:115c:a1e0::2"), rex("[fd7a:115c:a1e0::2]"));
    assert_eq!(of("::1"), rex("[::1]"));
    // With a port, that port alone: with TLS, then without.
    let both = |at: &str| net(&[&format!("wss://{at}"), &format!("ws://{at}")]);
    assert_eq!(of("192.168.1.20:7297"), both("192.168.1.20:7297"));
    assert_eq!(of("[::1]:7297"), both("[::1]:7297"));
    assert_eq!(of("box:443"), both("box:443"));
    for bad in [
        "",
        "ssh://box",
        "tailnet://:7297",
        "ws://",
        "box:port",
        "a b",
    ] {
        assert!(targets(bad).is_err(), "{bad:?} is no endpoint");
    }
}

#[test]
fn an_address_is_looked_up_without_its_brackets() {
    let at = |url: &str| {
        let url = Url::parse(url).unwrap();
        let (host, port) = host_port(&url).unwrap();
        (host.to_owned(), port)
    };
    assert_eq!(at("wss://[fd7a::1]:7297"), ("fd7a::1".into(), 7297));
    assert_eq!(at("ws://[::1]"), ("::1".into(), 80));
    assert_eq!(at("wss://studio.ts.net"), ("studio.ts.net".into(), 443));
    assert_eq!(at("ws://192.168.1.20:7297"), ("192.168.1.20".into(), 7297));
}

type Server = WebSocketStream<tokio::io::DuplexStream>;

/// A client's socket and the server's end of it, joined in memory.
async fn pair() -> (Socket, Server) {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let (client, server) = tokio::io::duplex(1 << 16);
    let io: Box<dyn Io> = Box::new(client);
    let io = tokio_tungstenite::MaybeTlsStream::Plain(io);
    let client = WebSocketStream::from_raw_socket(io, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    (client, server)
}

/// A control connection past its welcome, with the server's end and the clock in hand.
struct Link {
    rpc: Rpc,
    events: mpsc::UnboundedReceiver<Event>,
    task: tokio::task::JoinHandle<Result<()>>,
    server: Server,
    closing: Arc<Notify>,
    clock: mpsc::UnboundedSender<()>,
}

impl Link {
    async fn open(max_message: usize) -> Self {
        let (socket, server) = pair().await;
        let (requests, receiver) = mpsc::channel(MAX_IN_FLIGHT);
        let (sender, events) = mpsc::unbounded_channel();
        let closing = Arc::new(Notify::new());
        // Time passes only when a test says so.
        let (clock, mut ticks) = mpsc::unbounded_channel();
        let ticks = stream::poll_fn(move |cx| ticks.poll_recv(cx));
        let closed = closing.clone();
        let run = control_loop(socket, receiver, sender, max_message, closed, ticks);
        Self {
            rpc: Rpc { requests },
            events,
            task: tokio::spawn(run),
            server,
            closing,
            clock,
        }
    }

    /// One look over the link, and time for it to be taken: with one thread, the
    /// loop runs until it has nothing left to do before this returns.
    async fn tick(&self) {
        self.clock.send(()).unwrap();
        tokio::task::yield_now().await;
    }

    /// Whether the server has been sent anything it has not read.
    fn sent(&mut self) -> bool {
        self.server.next().now_or_never().is_some()
    }

    /// The next request to reach the server: its number and its method.
    async fn asked(&mut self) -> (String, String) {
        let Some(Ok(Message::Text(text))) = self.server.next().await else {
            panic!("the server was sent something other than a request");
        };
        let request: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(request["type"], "request");
        let text = |key: &str| request[key].as_str().unwrap().to_owned();
        (text("id"), text("method"))
    }

    async fn answer(&mut self, message: Value) {
        let message = Message::Text(message.to_string().into());
        self.server.send(message).await.unwrap();
    }

    /// Why the connection ended.
    async fn ended(self) -> Ended {
        let error = self.task.await.unwrap().unwrap_err();
        error.downcast_ref::<Ended>().expect("a reason").clone()
    }
}

fn close(code: u16, reason: &str) -> Option<CloseFrame> {
    let (code, reason) = (CloseCode::from(code), reason.into());
    Some(CloseFrame { code, reason })
}

#[tokio::test]
async fn answers_reach_their_askers_and_news_its_readers() {
    let mut link = Link::open(65536).await;
    let first = link.rpc.enqueue("one", json!({})).await.unwrap();
    let second = link.rpc.enqueue("two", json!({})).await.unwrap();
    assert_eq!(link.asked().await, ("1".into(), "one".into()));
    assert_eq!(link.asked().await, ("2".into(), "two".into()));
    // Answers come in any order, among news and things this client does not know.
    let response = |id: &str| json!({"type": "response", "id": id, "payload": {"for": id}});
    link.answer(response("2")).await;
    link.answer(sent(None, "hosts_changed", json!({"hash": "h"})))
        .await;
    link.answer(sent(None, "client_joined", json!({}))).await;
    link.answer(response("99")).await;
    link.answer(json!({"type": "later", "id": "1"})).await;
    let error = json!({"code": "not_found", "message": "No such session"});
    let refusal = json!({"type": "response", "id": "1", "error": error});
    link.answer(refusal).await;
    assert_eq!(second.await.unwrap(), Ok(json!({"for": "2"})));
    let refused = Err("not_found: No such session".into());
    assert_eq!(first.await.unwrap(), refused);
    let hash = "h".to_owned();
    assert_eq!(link.events.recv().await, Some(Event::Hosts { hash }));
    assert!(link.events.try_recv().is_err());
    assert!(!link.task.is_finished());
}

#[tokio::test]
async fn a_request_too_large_for_the_server_fails_alone() {
    let mut link = Link::open(256).await;
    let large = link.rpc.call("paste", json!({"text": "a".repeat(256)}));
    let error = large.await.unwrap_err().to_string();
    assert!(
        error.contains("exceeds server limit (256 bytes)"),
        "{error}"
    );
    // Nothing of it was sent, and the next request is the first to arrive.
    let small = link.rpc.enqueue("key", json!({})).await.unwrap();
    let (id, method) = link.asked().await;
    assert_eq!((id.as_str(), method.as_str()), ("2", "key"));
    link.answer(json!({"type": "response", "id": id})).await;
    assert_eq!(small.await.unwrap(), Ok(json!({})));
}

#[tokio::test]
async fn a_request_never_answered_fails_alone() {
    let mut link = Link::open(65536).await;
    let mut slow = link.rpc.enqueue("slow", json!({})).await.unwrap();
    let (id, _) = link.asked().await;
    // The server is there all the while, and so is not pinged.
    for look in 1..=REQUEST_TICKS {
        assert!(slow.try_recv().is_err(), "given up on at look {look}");
        let pong = Message::Pong(Default::default());
        link.server.send(pong).await.unwrap();
        link.tick().await;
    }
    let error = slow.await.unwrap().unwrap_err();
    assert!(error.contains("took too long to answer slow"), "{error}");
    assert!(!link.sent());
    // Its answer comes after all, and is passed over. The connection carries on.
    link.answer(json!({"type": "response", "id": id})).await;
    let next = link.rpc.enqueue("next", json!({})).await.unwrap();
    let (id, method) = link.asked().await;
    assert_eq!(method, "next");
    let answer = json!({"type": "response", "id": id, "payload": 1});
    link.answer(answer).await;
    assert_eq!(next.await.unwrap(), Ok(json!(1)));
}

#[tokio::test]
async fn a_server_that_answers_no_ping_is_gone() {
    let mut link = Link::open(65536).await;
    let ping = |message| matches!(message, Some(Ok(Message::Ping(_))));
    // Nothing heard by the first look: the server is asked whether it is there.
    link.tick().await;
    assert!(ping(link.server.next().await));
    // It is. Reading the ping queued its answer, which this sends.
    link.server.flush().await.unwrap();
    link.tick().await;
    assert!(!link.sent());
    // Then nothing again. It is asked again, and this time does not answer.
    link.tick().await;
    assert!(ping(link.server.next().await));
    assert!(!link.task.is_finished());
    link.tick().await;
    assert_eq!(link.ended().await, Ended::Silent);
}

#[tokio::test]
async fn a_server_that_talks_is_not_pinged() {
    let mut link = Link::open(65536).await;
    for _ in 0..3 {
        let news = json!({"type": "event", "payload": {"type": "session_created"}});
        link.answer(news).await;
        link.tick().await;
        assert!(!link.sent());
    }
    // Until it stops.
    link.tick().await;
    assert!(link.sent());
}

#[tokio::test]
async fn the_server_says_why_it_closes_the_control_connection() {
    let ended = async |frame: Option<CloseFrame>| {
        let mut link = Link::open(65536).await;
        link.server.close(frame).await.unwrap();
        link.ended().await
    };
    let behind = ended(close(1008, "event backlog exceeded")).await;
    let (code, reason) = (1008, "event backlog exceeded".into());
    assert_eq!(behind, Ended::Closed { code, reason });
    let words = "This client fell too far behind the server's news";
    assert_eq!(behind.to_string(), words);
    let reason = "no common API revision: server speaks 14 to 15, client needs 13 to 13; \
                  update the client";
    let new = ended(close(1008, reason)).await;
    let why = "server speaks 14 to 15, client needs 13 to 13; update the client";
    assert_eq!(new, Ended::Incompatible(why.into()));
    let words = format!("This client and the server share no API revision: {why}");
    assert_eq!(new.to_string(), words);
    let plain = ended(None).await.to_string();
    assert_eq!(plain, "The server closed the connection");
    let unknown = ended(close(4321, "new")).await.to_string();
    assert_eq!(
        unknown,
        "The server closed the connection (status 4321: new)"
    );
    // A link that goes with no close frame is another thing.
    let Link {
        server, task, rpc, ..
    } = Link::open(65536).await;
    drop(server);
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.downcast_ref(), Some(&Ended::Dropped));
    // Whoever asks after the end is told it has ended.
    let error = rpc.call("one", json!({})).await.unwrap_err();
    assert_eq!(error.to_string(), ENDED);
}

#[tokio::test]
async fn the_server_says_why_a_terminal_stream_ends() {
    let ended = async |frame: Option<CloseFrame>| {
        let (mut socket, mut server) = pair().await;
        server.close(frame).await.unwrap();
        let error = next_binary(&mut socket).await.unwrap_err();
        error.downcast_ref::<Ended>().expect("a reason").clone()
    };
    // The one end after which there is nothing to reconnect to.
    let closed = ended(close(4000, "block closed")).await;
    assert_eq!(closed, Ended::BlockClosed);
    assert_eq!(closed.to_string(), "The terminal has closed");
    let afresh = ended(close(1013, "data reconnect required")).await;
    let (code, reason) = (1013, "data reconnect required".into());
    assert_eq!(afresh, Ended::StreamClosed { code, reason });
    assert!(afresh.to_string().contains("started afresh"));
    let plain = ended(None).await.to_string();
    assert_eq!(plain, "The terminal's stream ended");
    // A handshake is answered in one message: refused with a reason, or accepted
    // with the nonce that marks the stream's own messages.
    let answered = async |answer: Vec<u8>| {
        let (mut socket, mut server) = pair().await;
        server.send(Message::Binary(answer.into())).await.unwrap();
        let nonce = handshake(&mut socket, "AAAAAAAAAAAAAAAAAAAAAA").await;
        (nonce, server.next().await)
    };
    let (refused, hello) = answered(vec![0x82, 1]).await;
    let refused = refused.unwrap_err();
    assert_eq!(refused.downcast_ref(), Some(&Ended::Rejected(1)));
    assert!(refused.to_string().contains("ticket"), "{refused}");
    assert!(Ended::Rejected(200).to_string().contains("reason 200"));
    let hello = hello.unwrap().unwrap().into_data();
    assert_eq!(hello[..], [&[1][..], &[0; 16][..]].concat());
    let mut welcome = vec![0x81];
    welcome.extend([0; 23]);
    welcome.extend(7_u64.to_le_bytes());
    assert_eq!(answered(welcome).await.0.unwrap(), 7);
}

#[tokio::test]
async fn a_client_that_leaves_says_goodbye() {
    let mut link = Link::open(65536).await;
    link.closing.notify_one();
    let Some(Ok(Message::Close(Some(frame)))) = link.server.next().await else {
        panic!("the server was sent no close frame");
    };
    assert_eq!(frame.code, CloseCode::Normal);
    // It stays to hear that the server heard. Reading the close queued the
    // server's own, which this sends.
    assert!(!link.task.is_finished());
    let _ = link.server.flush().await;
    assert!(link.task.await.unwrap().is_ok());
    // A server that only hangs up has heard as well.
    let mut link = Link::open(65536).await;
    link.closing.notify_one();
    let said = link.server.next().await;
    assert!(matches!(said, Some(Ok(Message::Close(_)))));
    drop(link.server);
    assert!(link.task.await.unwrap().is_ok());
}

/// A server of one connection at a socket of its own. It hears the hello, gives
/// `answer`, and hears what comes next.
async fn serve(listener: tokio::net::UnixListener, answer: Message) -> (Value, Option<Message>) {
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let (stream, _) = listener.accept().await.unwrap();
    // The size of the error is the library's choice.
    #[allow(clippy::result_large_err)]
    let agree = |request: &Request, mut response: Response| {
        let protocol = request.headers()["Sec-WebSocket-Protocol"].clone();
        let headers = response.headers_mut();
        headers.insert("Sec-WebSocket-Protocol", protocol);
        Ok::<_, ErrorResponse>(response)
    };
    let socket = tokio_tungstenite::accept_hdr_async(stream, agree);
    let mut socket = socket.await.unwrap();
    let Some(Ok(Message::Text(hello))) = socket.next().await else {
        panic!("the server was sent no hello");
    };
    socket.send(answer).await.unwrap();
    let next = socket.next().await.and_then(Result::ok);
    (serde_json::from_str(&hello).unwrap(), next)
}

#[tokio::test]
async fn a_client_is_welcomed_at_a_socket_or_told_why_not() {
    let name = format!("rex-lite-test-{}.sock", std::process::id());
    let path = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&path);
    let listener = || tokio::net::UnixListener::bind(&path).unwrap();
    let endpoint = format!("unix://{}", path.display());
    let welcome = json!({
        "type": "welcome",
        "protocol": CONTROL_PROTOCOL,
        "api_revision": 13,
        "limits": {"max_message_bytes": 65536},
        "client": {"id": "client:1", "reconnect_token": "next"},
    });
    let welcome = Message::Text(welcome.to_string().into());
    let server = tokio::spawn(serve(listener(), welcome));
    let connect = |token| Control::connect(&endpoint, token, None);
    let mut control = connect(Some("last")).await.unwrap();
    assert_eq!(control.client_id, "client:1");
    // Not the client its token was given to, unless the server says so.
    assert!(!control.resumed);
    assert_eq!(control.reconnect_token.as_deref(), Some("next"));
    // The address that answered is where a terminal's stream is sent.
    assert_eq!(control.target, Target::Unix(path.clone()));
    // Leaving, it says so, and has been heard by the time it has left.
    control.close().await;
    assert!(control.task.is_finished());
    let (hello, last) = server.await.unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["reconnect_token"], "last");
    // It offers the one revision it speaks, and no older.
    assert_eq!(
        (&hello["api_revision"], &hello["api_min_revision"]),
        (&json!(13), &json!(13))
    );
    let Some(Message::Close(Some(frame))) = last else {
        panic!("the server was sent no close frame");
    };
    assert_eq!(frame.code, CloseCode::Normal);
    std::fs::remove_file(&path).unwrap();
    // A server that cannot serve the client closes instead of welcoming it. One too
    // old to know that welcomes it at a revision it did not offer. Both are told
    // of in the same words.
    let why = "server speaks 0 to 12, client needs 13 to 13; update the server";
    let refusal = Message::Close(close(1008, &format!("no common API revision: {why}")));
    let old = json!({"type": "welcome", "protocol": CONTROL_PROTOCOL, "api_server_revision": 12});
    for answer in [refusal, Message::Text(old.to_string().into())] {
        let server = tokio::spawn(serve(listener(), answer));
        let Err(error) = connect(None).await else {
            panic!("welcomed by a server too old");
        };
        let refused = Ended::Incompatible(why.into());
        assert_eq!(error.downcast_ref(), Some(&refused));
        server.await.unwrap();
        std::fs::remove_file(&path).unwrap();
    }
}
