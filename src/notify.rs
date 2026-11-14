//! Word to the desktop of what a pane has to say while nobody is looking at it, by
//! way of the notification service on the session's D-Bus. Nothing here waits on
//! the window: one that is out of sight draws nothing, and is just the one whose
//! panes need speaking for.
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use tokio::time::{Instant, sleep_until, timeout};
use zbus::{MatchRule, MessageStream, wire::Variant};

/// What the desktop knows this program as.
pub const APP_ID: &str = "dev.rex-lite.client";
const NAME: &str = "Rex Lite";
const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// How long a note waits to be sent, for the window to say its pane is being looked
/// at. It is also the least time between two notes of one pane.
const HOLD: Duration = Duration::from_millis(500);
/// How long the same words from the same pane go without saying twice.
const REPEAT: Duration = Duration::from_secs(10);
/// How long the service has to answer.
const PATIENCE: Duration = Duration::from_secs(3);
/// The most of a note's text that is passed on.
const MAX_TEXT: usize = 400;

/// What a pane has to say.
pub struct Note {
    pub pane: String,
    /// The tab it is in.
    pub tab: String,
    /// Where it is said from: a program can claim to be anything but that.
    pub summary: String,
    pub body: String,
}

/// Where a server's connection sends what its panes have to say.
#[derive(Clone)]
pub struct Herald {
    notes: flume::Sender<Note>,
    /// Something has been said since the window last asked.
    news: Arc<AtomicBool>,
    /// The server's name. Home has none.
    server: String,
}

impl Herald {
    /// Home's, and where its notes and those of the others come out.
    pub fn new() -> (Self, flume::Receiver<Note>) {
        // A program can say things faster than they can be passed on.
        let (notes, out) = flume::bounded(256);
        let (news, server) = (Arc::default(), String::new());
        let herald = Self {
            notes,
            news,
            server,
        };
        (herald, out)
    }

    /// The same for another server, by its name.
    pub fn of(&self, server: &str) -> Self {
        let server = server.to_owned();
        Self {
            server,
            ..self.clone()
        }
    }

    /// Whether anything has been said, by any server, since this was last asked.
    pub fn news(&self) -> bool {
        self.news.swap(false, Ordering::AcqRel)
    }

    /// Pass on what a pane has to say. Its tab is given by ID and by name, and the
    /// session by name.
    pub fn say(&self, pane: &str, tab: &str, name: &str, session: &str, body: &str) {
        let place = match self.server.as_str() {
            "" => format!("{name} — {session}"),
            server => format!("{name} — {session} on {server}"),
        };
        let note = Note {
            pane: pane.to_owned(),
            tab: tab.to_owned(),
            summary: plain(&place),
            body: plain(body),
        };
        let _ = self.notes.try_send(note);
        self.news.store(true, Ordering::Release);
    }
}

/// Text from a program, made fit to show outside its terminal: on one line, with
/// nothing that would turn the text around it about, and no longer than a glance takes.
fn plain(text: &str) -> String {
    let turns = |c| matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    let kept = text.chars().filter(|c| !turns(*c));
    let mut plain: String = kept
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_TEXT + 1)
        .collect();
    if let Some((end, _)) = plain.char_indices().nth(MAX_TEXT) {
        plain.truncate(end);
        plain.push('…');
    }
    plain
}

/// What the window tells the worker.
enum Request {
    /// A pane is on show in the window, and the window is the one in use.
    Watched(String),
    /// These tabs, and no others, are to be left unspoken for.
    Mute(HashSet<String>),
}

/// The window's end of the worker that speaks to the desktop.
pub struct Desktop {
    requests: flume::Sender<Request>,
    picked: flume::Receiver<String>,
}

impl Desktop {
    /// Start passing on what comes out of a herald. `wake` is called, from another
    /// thread, when the user has picked a note: with the leave to come to the front
    /// that the desktop gave with the picking, if it gave any.
    pub fn start(
        notes: flume::Receiver<Note>,
        wake: impl Fn(Option<&str>) + Send + 'static,
    ) -> Self {
        let (requests, asked) = flume::unbounded();
        let (pick, picked) = flume::unbounded();
        let work = move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(runtime) = runtime {
                let picked = |pane, token: Option<&str>| {
                    let _ = pick.send(pane);
                    wake(token);
                };
                runtime.block_on(Worker::default().run(notes, asked, picked));
            }
        };
        // Without the worker nothing is passed on, and nothing else is the worse for it.
        let _ = std::thread::Builder::new()
            .name("rex-notify".into())
            .spawn(work);
        Self { requests, picked }
    }

    /// A pane is being looked at: what it has said needs no passing on.
    pub fn watched(&self, pane: &str) {
        let _ = self.requests.send(Request::Watched(pane.to_owned()));
    }

    pub fn mute(&self, tabs: HashSet<String>) {
        let _ = self.requests.send(Request::Mute(tabs));
    }

    /// A pane whose note the user has picked, to be shown.
    pub fn picked(&self) -> Option<String> {
        self.picked.try_recv().ok()
    }
}

/// A note the desktop has been given.
struct Shown {
    id: u32,
    body: String,
    at: Instant,
}

#[derive(Default)]
struct Worker {
    bus: Option<Bus>,
    muted: HashSet<String>,
    /// The note each pane is waiting to have sent, and when it will be.
    held: HashMap<String, (Instant, Note)>,
    /// The last sent of each pane, until it is closed or the pane is looked at.
    shown: HashMap<String, Shown>,
    /// The leave to come to the front that came with the picking of a note.
    token: Option<(u32, String)>,
}

impl Worker {
    async fn run(
        &mut self,
        notes: flume::Receiver<Note>,
        requests: flume::Receiver<Request>,
        picked: impl Fn(String, Option<&str>),
    ) {
        loop {
            let due = self.held.values().map(|(due, _)| *due).min();
            tokio::select! {
                // In this order: a note is said before the window says that it is
                // looking at the pane, and is to be taken in before that too.
                biased;
                note = notes.recv_async() => {
                    let Ok(note) = note else { return };
                    if !self.muted.contains(&note.tab) {
                        // The latest of a pane is the one sent, when the first would have been.
                        let due = self.held.get(&note.pane).map(|(due, _)| *due);
                        let due = due.unwrap_or_else(|| Instant::now() + HOLD);
                        self.held.insert(note.pane.clone(), (due, note));
                    }
                }
                request = requests.recv_async() => match request {
                    Ok(Request::Watched(pane)) => {
                        self.held.remove(&pane);
                        if let Some(shown) = self.shown.remove(&pane) {
                            self.call::<()>("CloseNotification", &(shown.id,)).await;
                        }
                    }
                    Ok(Request::Mute(tabs)) => {
                        self.held.retain(|_, (_, note)| !tabs.contains(&note.tab));
                        self.muted = tabs;
                    }
                    // The window has closed.
                    Err(_) => return,
                },
                // Neither of these two comes up while it has nothing to wait on.
                Some(()) = async { sleep_until(due?).await; Some(()) } => {
                    let now = Instant::now();
                    let due = self.held.extract_if(|_, (due, _)| *due <= now);
                    for (_, (_, note)) in due.collect::<Vec<_>>() {
                        self.send(note).await;
                    }
                }
                Some(signal) = async { Some(self.bus.as_mut()?.signals.next().await) } => {
                    match signal {
                        Some(Ok(signal)) => self.signalled(&signal, &picked),
                        _ => self.lost(),
                    }
                }
            }
        }
    }

    /// The bus is gone, and what it was given with it. It is found again when next
    /// there is something to say.
    fn lost(&mut self) {
        self.bus = None;
        self.shown.clear();
    }

    /// Give the desktop a note, in place of the one its pane gave before.
    async fn send(&mut self, note: Note) {
        let before = self.shown.get(&note.pane);
        if before.is_some_and(|shown| shown.body == note.body && shown.at.elapsed() < REPEAT) {
            return;
        }
        if self.bus.is_none() {
            self.bus = timeout(PATIENCE, Bus::open()).await.ok().flatten();
        }
        let Some(bus) = &self.bus else { return };
        // A service that takes markup would take a program's text for some.
        let body = match bus.markup {
            true => note.body.replace('&', "&amp;").replace('<', "&lt;"),
            false => note.body.clone(),
        };
        let hints = HashMap::from([("desktop-entry", Variant::from(APP_ID))]);
        let replaces = before.map_or(0, |shown| shown.id);
        let notice = (
            NAME,
            replaces,
            "",
            &note.summary,
            &body,
            // A list, which an array of two would not be sent as.
            &["default", "Show"][..],
            hints,
            -1,
        );
        if let Some(id) = self.call("Notify", &notice).await {
            let (body, at) = (note.body, Instant::now());
            self.shown.insert(note.pane, Shown { id, body, at });
        }
    }

    /// Ask something of the service. One that says no has not lost the bus.
    async fn call<R>(
        &mut self,
        method: &str,
        body: &(impl zbus::wire::export::serde::Serialize + zbus::DynamicType),
    ) -> Option<R>
    where
        R: for<'a> zbus::wire::DynamicDeserialize<'a>,
    {
        let bus = self.bus.as_ref()?;
        let call = bus
            .connection
            .call_method(Some(SERVICE), PATH, Some(SERVICE), method, body);
        match timeout(PATIENCE, call).await {
            Ok(Ok(reply)) => reply.body().deserialize().ok(),
            Ok(Err(zbus::Error::MethodError(..) | zbus::Error::FDO(_))) => None,
            _ => {
                self.lost();
                None
            }
        }
    }

    /// Take in what the service says of the notes it was given.
    fn signalled(&mut self, signal: &zbus::Message, picked: &impl Fn(String, Option<&str>)) {
        let header = signal.header();
        let body = signal.body();
        match header.member().map(|member| member.as_str()) {
            // It comes just ahead of the news of the picking.
            Some("ActivationToken") => self.token = body.deserialize().ok(),
            Some("ActionInvoked") => {
                let Ok((id, _)) = body.deserialize::<(u32, String)>() else {
                    return;
                };
                let token = self.token.take().filter(|(of, _)| *of == id);
                if let Some((pane, _)) = self.shown.iter().find(|(_, shown)| shown.id == id) {
                    let token = token.as_ref().map(|(_, token)| token.as_str());
                    picked(pane.clone(), token);
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                    self.shown.retain(|_, shown| shown.id != id);
                }
            }
            _ => {}
        }
    }
}

/// The session's bus, and what the notification service on it says.
struct Bus {
    connection: zbus::Connection,
    signals: MessageStream,
    /// The service reads a note's body as markup.
    markup: bool,
}

impl Bus {
    async fn open() -> Option<Self> {
        let connection = zbus::Connection::session().await.ok()?;
        let rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .interface(SERVICE)
            .build()
            .ok()?;
        let signals = MessageStream::for_match_rule(rule, &connection, None)
            .await
            .ok()?;
        let able = connection
            .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &())
            .await
            .ok()?;
        let able: Vec<String> = able.body().deserialize().ok()?;
        let markup = able.iter().any(|able| able == "body-markup");
        Some(Self {
            connection,
            signals,
            markup,
        })
    }
}

#[cfg(test)]
#[path = "../tests/unit/notify.rs"]
mod tests;
