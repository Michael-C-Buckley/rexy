//! A thin Rex client: one I/O runtime, bounded actions, separate terminal CPU workers.
//!
//! The model of a server is kept by following it. One request on connecting says
//! what sessions there are, and after that the server's news says what changes.
//! A session's view is asked for again only when news or an answer names a revision
//! of it newer than the one held. Nothing asked is waited for where input is taken:
//! each answer is dealt with when it comes.
use crate::{
    model::{
        self, Action, Activity, Border, Direction, Division, Edit, Host, Pane, Session, View, Wait,
        Window,
    },
    notify::Herald,
    program::Program,
    ssh,
    terminal::{Notify, Request, Scroll, TerminalHandle},
    wire::{self, Control, ENDED, Ended, Event, News, Rpc},
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

/// How long to wait before each try at something that keeps failing.
const RETRY: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(15)];
/// How long another client's asking waits for the window to take it.
const UNTAKEN: Duration = Duration::from_secs(3);
const FULL: [f32; 4] = [0., 0., 1., 1.];
const TERMINAL: &str = "com.superlogical.terminal";
const TITLE: &str = "com.superlogical.terminal.title";
const PROCESS: &str = "com.superlogical.terminal.process";
const STATUS: &str = "com.superlogical.terminal.program_status";
const FOLDER: &str = "com.superlogical.terminal.pwd";
const COPIED: &str = "com.superlogical.terminal.clipboard_read";
/// The most a program may copy, and the most of it to ask for at a time.
const MAX_COPY_BYTES: usize = 8 * 1024 * 1024;
const COPY_PART_BYTES: usize = 960 * 1024;
static NULL: Value = Value::Null;

/// Coalesces repaint requests until the UI takes its next snapshot.
struct Wake {
    pending: AtomicBool,
    callback: Notify,
}
impl Wake {
    fn notify(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            (self.callback)();
        }
    }
}

#[derive(Default)]
struct State {
    view: View,
    /// The terminals being fed what their panes show, which take what is typed.
    terminals: HashMap<String, TerminalHandle>,
    /// Those fed no more, since a stream or the connection failed. Each is kept
    /// for its last screen, to be read, looked through and copied from, until
    /// its pane is fed again or goes from sight. Nothing is typed into one.
    last: HashMap<String, TerminalHandle>,
    /// How many times the user has asked for something to be shown, or to be left.
    /// Each asking goes to the worker with its number, so that what the server
    /// shows moves what is on show only once it has answered the last of them.
    picks: u64,
}

impl State {
    /// Stop counting a pane's terminal among those fed. One that has drawn is
    /// kept for its last screen.
    fn rest(&mut self, pane: &str) {
        if let Some(terminal) = self.terminals.remove(pane)
            && terminal.latest().is_some()
        {
            self.last.insert(pane.into(), terminal);
        }
    }
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    wake: Arc<Wake>,
    herald: Herald,
}
impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn change(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.lock());
        self.wake.notify();
    }
    fn status(&self, message: impl Into<String>, connected: bool) {
        self.change(|s| {
            s.view.status = message.into();
            s.view.connected = connected;
        });
    }
    fn fail(&self, error: String) {
        self.change(|s| s.view.error = Some(error));
    }
    fn terminal(&self, pane: &str) -> Option<TerminalHandle> {
        self.lock().terminals.get(pane).cloned()
    }
}

pub struct Client {
    actions: mpsc::Sender<(Action, u64)>,
    shared: Shared,
    /// Closes when the network worker has ended.
    done: flume::Receiver<()>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // The worker stops when its queue closes, first telling a server it is
        // connected to that this client has gone. The process may be about to end,
        // so wait a moment for that to be said. With no connection there is nothing to say.
        drop(std::mem::replace(&mut self.actions, mpsc::channel(1).0));
        if self.shared.lock().view.connected {
            let wait = wire::CLOSE_TIMEOUT + Duration::from_millis(150);
            let _ = self.done.recv_timeout(wait);
        }
    }
}

impl Client {
    /// Connect to a server at the first of its endpoints that answers, and keep
    /// connected. None of its sessions is shown until one is asked for.
    /// What its panes have to say to someone not looking goes to `herald`.
    /// `keymap` is what the server's keymap is asked with: what a window can do
    /// itself, and by which keys. A client with none asks for no keymap.
    pub fn spawn(
        endpoints: Vec<String>,
        wake: Notify,
        herald: Herald,
        keymap: Option<Value>,
    ) -> Self {
        let (ended, done) = flume::bounded(0);
        let shared = Shared {
            state: Arc::default(),
            wake: Arc::new(Wake {
                pending: AtomicBool::new(false),
                callback: wake,
            }),
            herald,
        };
        let (actions, receiver) = mpsc::channel(256);
        let worker = shared.clone();
        let started = std::thread::Builder::new()
            .name("rex-io".into())
            .spawn(move || {
                // Dropped last, once the runtime and all it carried are gone.
                let _ended = ended;
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run(endpoints, keymap, receiver, worker)),
                    Err(error) => {
                        worker.status(format!("Cannot start network runtime: {error}"), false)
                    }
                }
            });
        if let Err(error) = started {
            shared.status(format!("Cannot start network worker: {error}"), false);
        }
        Self {
            actions,
            shared,
            done,
        }
    }

    pub fn snapshot(&self) -> View {
        // Reset before reading: an update arriving during the copy schedules another paint.
        self.shared.wake.pending.store(false, Ordering::Release);
        let mut state = self.shared.lock();
        let mut view = state.view.clone();
        state.view.error = None;
        state.view.made = None;
        state.view.copied = None;
        state.view.asked.clear();
        state.view.rang.clear();
        for pane in view.panes_mut() {
            if let Some(terminal) = state.terminals.get(&pane.id) {
                pane.frame = terminal.latest();
                pane.status = match terminal.status() {
                    Some(error) => error,
                    None if pane.frame.is_some() => String::new(),
                    None => "Restoring terminal…".into(),
                };
            } else if let Some(terminal) = state.last.get(&pane.id) {
                // Its last screen, with what has been selected on it since.
                pane.frame = terminal.latest();
            }
        }
        view
    }

    pub fn try_send(&self, action: Action) -> Result<(), String> {
        // Queue under the state lock. A failed connection is marked disconnected
        // under the same lock before its queue is discarded, so input accepted
        // on it can never reach its replacement.
        let mut state = self.shared.lock();
        if !state.view.connected && !matches!(action, Action::Reconnect) {
            return Err("Disconnected; input was not sent".into());
        }
        // The session and the tab this asks to be shown, where it says which.
        let shown = match &action {
            Action::SelectSession(session) | Action::CreateWindow(session) => {
                Some((session.clone(), None))
            }
            Action::SelectWindow { session, window } => {
                Some((session.clone(), Some(window.clone())))
            }
            _ => None,
        };
        // Asking for something to be shown has a number. Nothing else has.
        let pick = shown.is_some() || matches!(action, Action::CreateSession(_));
        let picks = state.picks + u64::from(pick);
        self.actions
            .try_send((action, if pick { picks } else { 0 }))
            .map_err(|e| format!("Input not sent: {e}"))?;
        state.picks = picks;
        // Show it at once; the server's answer follows.
        let view = &mut state.view;
        if let Some((id, window)) = shown
            && view.sessions.iter().any(|session| session.id == id)
        {
            if window.is_some() || view.selected_session.as_ref() != Some(&id) {
                view.selected_window = window;
            }
            view.selected_session = Some(id);
            view.settle(false);
            drop(state);
            self.shared.wake.notify();
        }
        Ok(())
    }

    /// The session and the tab that hold a pane, if this server has the pane.
    pub fn place(&self, pane: &str) -> Option<(String, String)> {
        let state = self.shared.lock();
        let (session, tab, _) = state.view.find(pane)?;
        Some((session.id.clone(), tab.id.clone()))
    }

    /// Stop showing this server's session. Its programs keep running there.
    pub fn leave(&self) {
        let mut state = self.shared.lock();
        state.picks += 1;
        state.view.selected_session = None;
        state.view.selected_window = None;
        // Even with nothing on show: what it was last asked may be on its way to being.
        if state.view.connected {
            let _ = self.actions.try_send((Action::Leave, state.picks));
        }
    }

    /// The terminal of a pane on show, or the one kept for its last screen, for
    /// what is done to it here and never sent to the server: moving through its
    /// history, selecting, and finding text.
    pub fn terminal(&self, pane: &str) -> Result<TerminalHandle, String> {
        let state = self.shared.lock();
        let terminal = state.terminals.get(pane).or_else(|| state.last.get(pane));
        terminal
            .cloned()
            .ok_or_else(|| "Terminal is not ready".into())
    }
}

/// How long to wait before trying again what has failed with `error`, having failed
/// `failures` times running before, or `None` to try it no more unasked. What is `near`, on this machine
/// or already joined, is tried for as long as it takes, slowly. Another machine is
/// left alone after a few tries: one that keeps refusing a login may come to
/// refuse the address it comes from.
fn retry(error: &anyhow::Error, failures: usize, near: bool) -> Option<Duration> {
    // A server this client cannot talk to, and a terminal that has closed, will
    // be no different next time.
    if let Some(Ended::Incompatible(_) | Ended::BlockClosed) = error.downcast_ref() {
        return None;
    }
    let slowly = RETRY.last().filter(|_| near);
    RETRY.get(failures).or(slowly).copied()
}

/// What outlasts a connection to a server, for the next to take up.
#[derive(Default)]
struct Link {
    /// The server knows this client again by it, in the session it had joined.
    token: Option<String>,
    /// The session this client has joined, from asking to until told it has left:
    /// the one on show.
    attached: Option<String>,
    /// The address that answered last, of the several a host name can stand for.
    target: Option<wire::Target>,
    /// What is known of every terminal on the server, shown or not.
    blocks: HashMap<String, Block>,
    /// What each connection asks the server's keymap with, if this client has keys.
    keymap: Option<Value>,
}

async fn run(
    endpoints: Vec<String>,
    keymap: Option<Value>,
    mut actions: mpsc::Receiver<(Action, u64)>,
    shared: Shared,
) {
    let socket = |endpoint: &String| {
        matches!(
            wire::targets(endpoint).as_deref(),
            Ok([wire::Target::Unix(_)])
        )
    };
    let near = endpoints.iter().all(socket);
    let mut link = Link {
        keymap,
        ..Link::default()
    };
    // Tries in a row that never connected.
    let mut failures = 0;
    loop {
        shared.status("Connecting…", false);
        let result = run_connected(&endpoints, &mut link, &mut actions, &shared).await;
        if actions.is_closed() {
            return;
        }
        if shared.lock().view.connected {
            failures = 0;
        }
        let delay = match &result {
            // The user asked for it to be made again, which is done at once.
            Ok(()) => Some(Duration::ZERO),
            Err(error) => retry(error, failures, near),
        };
        failures += 1;
        shared.change(|s| {
            // Each last screen is kept for context, and no old input is sent again.
            let fed: Vec<String> = s.terminals.keys().cloned().collect();
            fed.iter().for_each(|pane| s.rest(pane));
            for pane in s.view.panes_mut() {
                pane.status = "Disconnected".into();
                pane.owner = false;
            }
            s.view.connected = false;
            s.view.status = match (result, delay) {
                (Ok(()), _) => "Reconnecting…".into(),
                (Err(error), Some(_)) => format!("Disconnected: {error:#} · retrying"),
                (Err(error), None) => format!("Disconnected: {error:#}"),
            };
        });
        while actions.try_recv().is_ok() {} // Commands belong to the failed connection.
        // Any action is a request to try now.
        let asked = match delay {
            Some(delay) => tokio::time::timeout(delay, actions.recv()).await,
            None => Ok(actions.recv().await),
        };
        if let Ok(None) = asked {
            return;
        }
    }
}

/// Reach the server at the first endpoint that answers. An `ssh://` endpoint is
/// reached through a tunnel, which lasts as long as it is held.
async fn connect(endpoints: &[String], link: &mut Link) -> Result<(Option<ssh::Tunnel>, Control)> {
    let mut failure = anyhow!("This host has no address");
    for endpoint in endpoints {
        let attempt = async {
            let (tunnel, endpoint) = if endpoint.starts_with("ssh://") {
                let tunnel = ssh::Tunnel::open(endpoint).await?;
                let local = tunnel.endpoint();
                (Some(tunnel), local)
            } else {
                (None, endpoint.clone())
            };
            let (token, last) = (link.token.as_deref(), link.target.as_ref());
            let control = Control::connect(&endpoint, token, last).await?;
            anyhow::Ok((tunnel, control))
        };
        match attempt.await {
            Ok(connected) => return Ok(connected),
            Err(error) => failure = error,
        }
    }
    Err(failure)
}

async fn run_connected(
    endpoints: &[String],
    link: &mut Link,
    actions: &mut mpsc::Receiver<(Action, u64)>,
    shared: &Shared,
) -> Result<()> {
    let (_tunnel, mut control) = connect(endpoints, link).await?;
    link.token = control.reconnect_token.take();
    link.target = Some(control.target.clone());
    let mut connection = Connection::open(&control, link, shared).await?;
    let mut connected = false;
    loop {
        // The UI is told of the connection once the tabs of every session have come,
        // so that it is never shown half a server. News waits its turn until then.
        if !connected && connection.views.values().all(|held| !held.asking) {
            connected = true;
            while actions.try_recv().is_ok() {} // Requests to try again have been answered.
            connection.settled = {
                let mut state = shared.lock();
                state.view.status.clear();
                state.view.connected = true;
                state.picks
            };
            connection.show().await;
        }
        tokio::select! {
            // Its worker alone knows why a connection has ended. Nothing else here
            // ends one, so that this is what is said of it.
            result = &mut control.task => {
                result.context("Rex control worker stopped")??;
                bail!(ENDED);
            }
            Some(event) = control.events.recv(), if connected => connection.event(event).await,
            // Nothing comes of what was asked of a connection that has ended.
            Some(Some((reply, answer))) = connection.replies.next() => {
                connection.answered(reply, answer).await;
            }
            action = actions.recv(), if connected => {
                let Some((action, pick)) = action else {
                    // The client is going. The server need not keep its place.
                    control.close().await;
                    return Ok(());
                };
                if matches!(action, Action::Reconnect) {
                    return Ok(());
                }
                if let Err(error) = connection.perform(action, pick).await {
                    shared.fail(format!("{error:#}"));
                }
            }
        }
    }
}

type Answer = Result<Value, String>;

/// What an answer on its way from the server is to.
enum Reply {
    /// Input, which is said to have failed if it has.
    Input,
    /// A change to a session, or the making of one, whose answer says what revision
    /// the session is left at. With it are the session, the number of the asking if
    /// the user asked by it for something to be shown, and a pane it brings out.
    Change(String, u64, Option<String>),
    /// A session's view.
    View(String),
    /// Joining a session, and leaving one.
    Joined(String),
    Left,
    /// The host list.
    Hosts,
    /// What this client's keys do.
    Keymap,
    /// One of the server's own actions, which may ask this client for more.
    Performed,
    /// A terminal's title, its processes, what its programs report and the
    /// directory it is in, in a list.
    /// `known` says an earlier connection knew the terminal.
    Learned {
        block: String,
        known: bool,
    },
}

/// The view held of a session, by how current it is.
#[derive(Default)]
struct Held {
    /// Its revision, once there is one.
    revision: Option<u64>,
    /// The newest revision heard of.
    heard: u64,
    /// A view has been asked for and has yet to come.
    asking: bool,
}

impl Held {
    /// Whether the view held is as new as any heard of.
    fn current(&self) -> bool {
        self.revision.is_some_and(|held| held >= self.heard)
    }

    /// Hear of a revision, and say whether to ask for the view: when the one held
    /// is older, and none is on its way that may prove not to be.
    fn want(&mut self, revision: u64) -> bool {
        self.heard = self.heard.max(revision);
        let ask = !self.asking && !self.current();
        self.asking |= ask;
        ask
    }

    /// Be handed a view, and say whether to take it: when it is newer than the one
    /// held. Views may come in any order.
    fn take(&mut self, revision: u64) -> bool {
        let newer = self.revision.is_none_or(|held| revision > held);
        if newer {
            self.revision = Some(revision);
        }
        newer
    }
}

/// One control connection, and where it stands with the server. Dropping it stops
/// every terminal stream.
struct Connection<'a> {
    shared: &'a Shared,
    link: &'a mut Link,
    rpc: Rpc,
    client_id: String,
    /// The address that answered, of the several a host name can stand for.
    /// Terminal streams go straight to it.
    target: wire::Target,
    /// The answers on their way.
    replies: FuturesUnordered<BoxFuture<'static, Option<(Reply, Answer)>>>,
    /// The sessions there are, and how current the view of each is.
    views: HashMap<String, Held>,
    /// The terminals asked about on this connection. `Some` holds the news of one
    /// that came before the answers, which may be older than it, to be taken in
    /// after them.
    asking: HashMap<String, Option<Vec<News>>>,
    /// The hash of the host list held.
    hosts: String,
    /// The hash of the configuration that the keymap held came of.
    keys: String,
    /// A request to join or to leave a session is on its way.
    joining: bool,
    /// The number of the last asking to be shown something that the server has
    /// answered, and how many before it are yet to be.
    settled: u64,
    unsettled: usize,
    /// A session the user asked to be made, and the number of that asking: shown
    /// once it is listed, if nothing has been asked since.
    making: Option<(String, u64)>,
    /// A pane this client has made or brought out, reported once it is on show.
    made: Option<String>,
    /// One stream per terminal of the tab on show.
    streams: HashMap<String, JoinHandle<()>>,
    /// The fetching of the last text a program copied.
    copying: Option<JoinHandle<()>>,
    /// Where a border was last asked to go, for `divide` to ask of the server.
    borders: watch::Sender<Vec<Value>>,
}

#[derive(Default)]
struct Block {
    session: String,
    title: String,
    /// The icon of the program in its foreground, if it has one.
    process: Option<Program>,
    /// The process of the shell it started with, if it started with one, and of
    /// what is in its foreground. Where they are one, the shell is at its prompt.
    shell: Option<u64>,
    foreground: Option<u64>,
    /// That shell's name, and the directory it is in, if the server says.
    prompt: String,
    folder: String,
    /// How far along a program in it says it is, by the terminal's own means.
    progress: Option<u8>,
    /// What its programs report of themselves, each record by its name. The
    /// program's own has none, and so comes first, and each part's after its whole.
    records: BTreeMap<String, Value>,
}

impl Block {
    /// What its programs say they are doing: the most pressing of it, the line
    /// said of that, and the icon of one still at it.
    fn says(&self) -> (Activity, &str, Option<Program>) {
        let (activity, record, app) = reported(self.records.values());
        (activity, record["msg"].as_str().unwrap_or_default(), app)
    }

    /// What it has to say to someone not looking, if that is other than it was `before`.
    fn news(&self, before: Activity) -> Option<String> {
        let (activity, message, _) = self.says();
        let word = activity.word().filter(|_| activity != before)?;
        Some(model::line(word, message))
    }

    /// Take in news of it.
    fn note(&mut self, news: News) {
        match news {
            News::Title(title) => self.title = title,
            News::Program { program, pid } => (self.process, self.foreground) = (program, pid),
            News::Status(records) => {
                for (id, record) in records {
                    match record {
                        Some(record) => self.records.insert(id, record),
                        None => self.records.remove(&id),
                    };
                }
            }
            News::Folder(folder) => self.folder = folder,
            News::Progress(progress) => self.progress = progress,
            // News of a moment, which changes nothing that is kept.
            News::Copied { .. } | News::Bell | News::Notice { .. } => {}
        }
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        for task in self.streams.values().chain(&self.copying) {
            task.abort();
        }
        // A terminal whose answers never came is not one the next connection knows.
        let unanswered = |id: &String| matches!(self.asking.get(id), Some(Some(_)));
        self.link.blocks.retain(|id, _| !unanswered(id));
    }
}

impl<'a> Connection<'a> {
    /// Begin on a new connection: learn what sessions there are, join the one on
    /// show, and ask for what the server's news will not tell.
    async fn open(control: &Control, link: &'a mut Link, shared: &'a Shared) -> Result<Self> {
        let rpc = control.rpc.clone();
        let shown = shared.lock().view.selected_session.clone();
        // A client the server knows again is still in the session it had joined,
        // and owns what it owned there. It leaves that one unless it is still the
        // one on show. A new client is in none.
        let before = link.attached.clone().filter(|_| control.resumed);
        if let Some(before) = before.filter(|before| Some(before) != shown.as_ref()) {
            rpc.call("session.detach", json!({"session_id": before}))
                .await?;
        }
        link.attached = shown.clone();
        // Events of the whole server, so that tabs not on show keep their titles too.
        let request =
            json!({"event_scope": "all", "attach_session_ids": shown.iter().collect::<Vec<_>>()});
        let boot = rpc.call("session.bootstrap", request).await?;
        let joined = &boot["attached_sessions"][0]["view"];
        let listed = items(&boot["attach_all"]["sessions"]).filter_map(|session| {
            Some((session["session_id"].as_str()?, session["label"].as_str()))
        });
        let views: HashMap<_, _> = listed
            .clone()
            .map(|(id, _)| (id.to_owned(), Held::default()))
            .collect();
        link.blocks
            .retain(|_, block| views.contains_key(&block.session));
        shared.change(|s| {
            let mut held = std::mem::take(&mut s.view.sessions);
            for (id, name) in listed {
                // One held from before keeps its tabs until they are seen anew.
                let at = held.iter().position(|session| session.id == id);
                let known = at.map(|at| held.swap_remove(at)).unwrap_or_default();
                s.view.sessions.push(Session {
                    id: id.into(),
                    name: label(name, "Untitled"),
                    ..known
                });
            }
        });
        let mut connection = Self {
            shared,
            link,
            rpc,
            client_id: control.client_id.clone(),
            target: control.target.clone(),
            replies: FuturesUnordered::new(),
            views,
            asking: HashMap::new(),
            hosts: String::new(),
            keys: String::new(),
            joining: false,
            settled: 0,
            unsettled: 0,
            making: None,
            made: None,
            streams: HashMap::new(),
            copying: None,
            borders: divide(control.rpc.clone(), shared.clone()),
        };
        match shown {
            Some(shown) if joined.is_object() => connection.view(&shown, joined).await,
            // The session on show has ended, or there was none.
            _ => connection.link.attached = None,
        }
        let sessions: Vec<String> = connection.views.keys().cloned().collect();
        for session in sessions {
            connection.want(&session, 0).await;
        }
        connection.ask(Reply::Hosts, "hosts.list", json!({})).await;
        connection.ask_keymap().await;
        Ok(connection)
    }

    /// Ask the server something and go on. Its answer comes to `answered`, with
    /// `reply` to say what it is to.
    async fn ask(&mut self, reply: Reply, method: &str, payload: Value) {
        // A connection that has ended takes no more, as the loop will find.
        if let Ok(response) = self.rpc.enqueue(method, payload).await {
            let answer = async move { Some((reply, response.await.ok()?)) };
            self.replies.push(Box::pin(answer));
        }
    }

    /// Ask what this client's keys do: its own, as changed by the account's
    /// configuration.
    async fn ask_keymap(&mut self) {
        if let Some(request) = self.link.keymap.clone() {
            self.ask(Reply::Keymap, "client.keymap", request).await;
        }
    }

    /// Ask it several things, to be answered together: by a list, with nothing in
    /// the place of what it would not say.
    async fn ask_all(&mut self, reply: Reply, requests: [(&str, Value); 4]) {
        let mut responses = Vec::new();
        for (method, payload) in requests {
            let Ok(response) = self.rpc.enqueue(method, payload).await else {
                return;
            };
            responses.push(response);
        }
        self.replies.push(Box::pin(async move {
            let mut said = Vec::new();
            for response in responses {
                said.push(response.await.ok()?.unwrap_or_default());
            }
            Some((reply, Ok(said.into())))
        }));
    }

    /// Ask for a session's view, if a revision of it is heard of that is newer than
    /// the one held.
    async fn want(&mut self, session: &str, revision: u64) {
        let held = self.views.get_mut(session);
        if held.is_some_and(|held| held.want(revision)) {
            let request = json!({"session_id": session});
            self.ask(Reply::View(session.into()), "session.view", request)
                .await;
        }
    }

    /// Take a view of a session, if it is newer than the one held: its tabs and
    /// panes, and the terminals in them.
    async fn view(&mut self, session: &str, view: &Value) {
        let revision = view["revision"].as_u64().unwrap_or(0);
        let held = self.views.get_mut(session);
        if !held.is_some_and(|held| held.take(revision)) {
            return;
        }
        let mut terminals = Vec::new();
        let windows = parse_windows(view, &self.client_id, &mut terminals);
        // Terminals that have left it are forgotten, and those not seen before on
        // this connection asked about: news of them may have been missed. There is
        // no asking all of it at once, nor of several terminals.
        let blocks = &mut self.link.blocks;
        blocks.retain(|id, block| block.session != session || terminals.contains(id));
        self.asking.retain(|id, _| blocks.contains_key(id));
        for id in terminals {
            let known = self.link.blocks.contains_key(&id);
            self.link.blocks.entry(id.clone()).or_default().session = session.into();
            if !self.asking.contains_key(&id) {
                let request = json!({"session_id": session, "block_id": id, "args": {}});
                let asked = [TITLE, PROCESS, STATUS, FOLDER];
                let asked = asked.map(|method| (method, request.clone()));
                self.asking.insert(id.clone(), Some(Vec::new()));
                let reply = Reply::Learned { block: id, known };
                self.ask_all(reply, asked).await;
            }
        }
        self.shared.change(|s| {
            if let Some(held) = s.view.sessions.iter_mut().find(|held| held.id == session) {
                held.name = label(view["label"].as_str(), "Untitled");
                held.active = view["active_window_id"].as_str().map(str::to_owned);
                held.windows = windows;
            }
            fill(&mut s.view.sessions, &self.link.blocks);
        });
    }

    /// Take in what the server says has changed.
    async fn event(&mut self, event: Event) {
        match event {
            Event::Viewed { session, revision } => return self.want(&session, revision).await,
            Event::Hosts { hash } => {
                if hash != self.hosts {
                    self.ask(Reply::Hosts, "hosts.list", json!({})).await;
                }
                return;
            }
            Event::Keymap { hash } => {
                if hash != self.keys {
                    self.ask_keymap().await;
                }
                return;
            }
            Event::Asked(asked) => {
                // A window that is out of sight takes nothing. What it has not
                // taken after a while is refused for it, and so never done late.
                let (shared, rpc, call) =
                    (self.shared.clone(), self.rpc.clone(), asked.call.clone());
                self.shared.change(|s| s.view.asked.push(asked));
                tokio::spawn(async move {
                    tokio::time::sleep(UNTAKEN).await;
                    let waits = |held: &mut model::Asked| held.call == call;
                    let untaken = shared.lock().view.asked.extract_if(.., waits).count() > 0;
                    if let Some(call) = call.filter(|_| untaken) {
                        let _ = rpc
                            .enqueue("client.reply", reply(&call, Some("refused")))
                            .await;
                    }
                });
                return;
            }
            Event::Block { block, news } => {
                return match news {
                    News::Bell => self.rang(block),
                    News::Notice { title, body } => self.tell(&block, &model::line(&title, &body)),
                    News::Copied { text, id } => self.copied(block, text, id),
                    news => match self.asking.get_mut(&block) {
                        Some(Some(held)) => held.push(news),
                        _ => self.note(&block, true, |terminal| terminal.note(news)),
                    },
                };
            }
            // Its name is told, and its tabs are to be asked for.
            Event::Created { ref session, .. } => {
                self.views.entry(session.clone()).or_default();
                self.want(session, 0).await;
            }
            Event::Destroyed { ref session } => {
                self.views.remove(session);
                self.link
                    .blocks
                    .retain(|_, block| &block.session != session);
            }
            Event::Moved { .. } => {}
        }
        self.shared.change(|s| list(&mut s.view.sessions, &event));
        self.show().await;
    }

    /// Take the server's answer to something asked of it.
    async fn answered(&mut self, reply: Reply, answer: Answer) {
        match (reply, answer) {
            (Reply::Input, Ok(_)) => return,
            (Reply::Input, Err(error)) => {
                return self.shared.fail(format!("Input failed: {error}"));
            }
            (Reply::Hosts, answer) => {
                // A server too old to keep a host list has none, and a list that
                // did not come leaves the one held: servers are dropped by it.
                let Ok(listed) = answer else {
                    return;
                };
                let hosts = items(&listed["hosts"]).filter_map(|host| {
                    let endpoints = items(&host["endpoints"]).filter_map(Value::as_str);
                    Some(Host {
                        label: host["label"].as_str()?.into(),
                        endpoints: endpoints.map(str::to_owned).collect(),
                    })
                });
                let hosts = hosts.collect();
                self.hosts = listed["hash"].as_str().unwrap_or_default().into();
                return self.shared.change(|s| s.view.hosts = hosts);
            }
            (Reply::Keymap, Ok(keymap)) => {
                self.keys = keymap["config"]["hash"].as_str().unwrap_or_default().into();
                return self
                    .shared
                    .change(|s| s.view.keymap = Some(Arc::new(keymap)));
            }
            // This client's own keys go on as they are.
            (Reply::Keymap, Err(error)) => {
                return self
                    .shared
                    .fail(format!("The server's keys were not read: {error}"));
            }
            (Reply::Performed, Ok(done)) => {
                // Its configuration may ask this client for actions of its own, in turn.
                let asked = items(&done["client_actions"]).filter_map(|asked| {
                    Some(model::Asked {
                        call: None,
                        name: asked["action"].as_str()?.into(),
                        args: asked["args"].clone(),
                        session: None,
                        pane: None,
                    })
                });
                let asked: Vec<_> = asked.collect();
                return self.shared.change(|s| s.view.asked.extend(asked));
            }
            (Reply::Performed, Err(error)) => return self.shared.fail(error),
            (Reply::Learned { block, known }, answer) => {
                // Unless the terminal has gone meanwhile.
                let Some(held) = self.asking.get_mut(&block).and_then(Option::take) else {
                    return;
                };
                let said = answer.unwrap_or_default();
                let [title, process, status, folder] = [0, 1, 2, 3].map(|at| &said[at]);
                let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
                // How a terminal stood when first seen is not news. How one that the
                // last connection knew has changed since that was lost is.
                return self.note(&block, known, |terminal| {
                    terminal.title = text(&title["title"]);
                    terminal.process = wire::program(&process["foreground"]);
                    terminal.shell = wire::shell(&process["child"]);
                    terminal.prompt = text(&process["child"]["name"]);
                    terminal.folder = text(&folder["pwd"]);
                    terminal.foreground = process["foreground"]["pid"].as_u64();
                    let records = items(&status["records"]);
                    terminal.records = records
                        .map(|record| (text(&record["id"]), record.clone()))
                        .collect();
                    held.into_iter().for_each(|news| terminal.note(news));
                });
            }
            (Reply::Change(session, pick, made), answer) => {
                self.settled = self.settled.max(pick);
                self.unsettled -= usize::from(pick > 0);
                match answer {
                    Ok(changed) => {
                        // A session it made is shown once listed, and a pane once on show.
                        let text = |value: &Value| value.as_str().map(str::to_owned);
                        let session_made = text(&changed["session_id"]).map(|id| (id, pick));
                        self.making = session_made.or(self.making.take());
                        self.made = made.or(text(&changed["block_ids"][0])).or(self.made.take());
                        let revision = changed["revision"].as_u64().unwrap_or(0);
                        self.want(&session, revision).await;
                    }
                    // What was put on show in hope of it goes back, in `show`, to
                    // what the server has.
                    Err(error) => self.shared.fail(error),
                }
            }
            (Reply::View(session), answer) => {
                if let Some(held) = self.views.get_mut(&session) {
                    held.asking = false;
                }
                // A session that ended a moment ago has none, as the news will say.
                if let Ok(view) = answer {
                    self.view(&session, &view).await;
                    // A newer one may have been heard of while this was on its way.
                    self.want(&session, 0).await;
                }
            }
            (Reply::Joined(session), answer) => {
                self.joining = false;
                match answer {
                    Ok(joined) => self.view(&session, &joined["view"]).await,
                    Err(error) => {
                        // Not in it after all, and not to be shown as if it were.
                        self.link.attached = None;
                        self.shared.change(|s| {
                            if s.view.selected_session.as_ref() == Some(&session) {
                                s.view.selected_session = None;
                            }
                            s.view.error = Some(error);
                        });
                    }
                }
            }
            (Reply::Left, _) => {
                self.joining = false;
                self.link.attached = None;
            }
        }
        self.show().await;
    }

    /// Bring what is on show, the session joined and the terminals streamed into
    /// line with the model and with what the user last asked for.
    async fn show(&mut self) {
        {
            let mut state = self.shared.lock();
            let State { view, picks, .. } = &mut *state;
            let listed = |id: &String| view.sessions.iter().any(|session| &session.id == id);
            if let Some((session, pick)) = self.making.take_if(|(session, _)| listed(session))
                && pick == *picks
            {
                (view.selected_session, view.selected_window) = (Some(session), None);
            }
            // The server's tab is the one to show once it has answered the last
            // thing asked of it here, and the view held is of no older a time.
            let current = |id: &String| self.views.get(id).is_some_and(Held::current);
            let current = view.selected_session.as_ref().is_some_and(current);
            let settled = *picks == self.settled && self.unsettled == 0;
            view.settle(settled && current);
            let seen = |id: &String| {
                view.tabs()
                    .any(|(_, tab)| tab.panes.iter().any(|p| &p.id == id))
            };
            if let Some(made) = self.made.take_if(|made| seen(made)) {
                view.made = Some(made);
            }
        }
        self.shared.wake.notify();
        self.join().await;
        self.reconcile().await;
    }

    /// Join the session on show and leave any other, a request at a time: asked
    /// together, a leaving and a joining of one session may be taken in either order.
    async fn join(&mut self) {
        let shown = self.shared.lock().view.selected_session.clone();
        if self.joining || self.link.attached == shown {
            return;
        }
        match (self.link.attached.clone(), shown) {
            (Some(old), _) => {
                let request = json!({"session_id": old});
                self.ask(Reply::Left, "session.detach", request).await;
            }
            (None, Some(new)) => {
                self.link.attached = Some(new.clone());
                let request = json!({"session_id": new});
                self.ask(Reply::Joined(new), "session.attach", request)
                    .await;
            }
            (None, None) => return,
        }
        self.joining = true;
    }

    /// Stream exactly the terminals of the tab on show, once its session is joined.
    /// One that is another client's is asked for, to be this client's when that
    /// one lets it go, as one that has left the server has already.
    async fn reconcile(&mut self) {
        let (session, visible) = {
            let mut state = self.shared.lock();
            // Only a session this connection has joined can be streamed.
            let joined = !self.joining && state.view.selected_session == self.link.attached;
            let window = state.view.current_window().filter(|_| joined);
            let panes = window.into_iter().flat_map(|w| &w.panes);
            let terminals = panes.filter(|pane| self.link.blocks.contains_key(&pane.id));
            let visible: HashMap<String, bool> = terminals
                .map(|pane| (pane.id.clone(), pane.owner))
                .collect();
            state.terminals.retain(|id, _| visible.contains_key(id));
            state.last.retain(|id, _| visible.contains_key(id));
            (state.view.selected_session.clone(), visible)
        };
        self.streams.retain(|id, task| {
            let keep = visible.contains_key(id);
            if !keep {
                task.abort();
            }
            keep
        });
        let Some(session) = session else {
            return;
        };
        let mut others = Vec::new();
        for (pane, owner) in visible {
            if self.streams.contains_key(&pane) {
                continue;
            }
            if !owner {
                others.push(pane.clone());
            }
            let stream = stream_pane(
                self.target.clone(),
                self.rpc.clone(),
                self.shared.clone(),
                session.clone(),
                pane.clone(),
            );
            self.streams.insert(pane, tokio::spawn(stream));
        }
        if !others.is_empty() {
            // Apart from all else that goes on here, which does not wait for it.
            let rpc = self.rpc.clone();
            tokio::spawn(async move {
                let own = "session.own_block";
                let claim = |panes: Value, mode: &str| json!({"session_id": session, "block_ids": panes, "mode": mode});
                let owned = rpc.call(own, claim(json!(others), "request")).await?;
                let present = rpc.call("client.list", json!({})).await?;
                let gone = abandoned(&owned["blocks"], &present["clients"]);
                if !gone.is_empty() {
                    rpc.call(own, claim(json!(gone), "take")).await?;
                }
                anyhow::Ok(())
            });
        }
    }

    /// Change what is known of a terminal, and show it. What its programs newly
    /// have to say by that is passed on if `loud`.
    fn note(&mut self, id: &str, loud: bool, change: impl FnOnce(&mut Block)) {
        let Some(block) = self.link.blocks.get_mut(id) else {
            return;
        };
        let before = block.says().0;
        change(block);
        let said = block.news(before).filter(|_| loud);
        self.shared
            .change(|s| fill(&mut s.view.sessions, &self.link.blocks));
        if let Some(said) = said {
            self.tell(id, &said);
        }
    }

    /// Pass on what a pane's programs have to say, if anything, for whoever is not
    /// looking at it.
    fn tell(&self, pane: &str, said: &str) {
        let state = self.shared.lock();
        let held = state.view.find(pane).filter(|_| !said.is_empty());
        if let Some((session, tab, _)) = held {
            let herald = &self.shared.herald;
            herald.say(pane, &tab.id, tab.name(), &session.name, said);
            // A window that is looking at the pane draws, and so says that it is.
            self.shared.wake.notify();
        }
    }

    /// A terminal's bell has rung. One that rings and rings is heard as one.
    fn rang(&self, pane: String) {
        self.tell(&pane, Activity::Rang.word().unwrap_or_default());
        self.shared.change(|s| {
            s.view.rang.insert(pane);
        });
    }

    /// Hand the UI the text a program has copied, if its pane is on show. Text too
    /// long for the server to have sent with the news is asked for apart from all
    /// else that goes on here, which does not wait for it. A later copy takes the
    /// place of one still being asked for.
    fn copied(&mut self, pane: String, text: Option<String>, id: u64) {
        let shown = |_: &&Block| self.streams.contains_key(&pane);
        let Some(block) = self.link.blocks.get(&pane).filter(shown) else {
            return;
        };
        let (rpc, shared, session) = (self.rpc.clone(), self.shared.clone(), block.session.clone());
        let fetch = tokio::spawn(async move {
            let text = match text {
                Some(text) => Ok(text),
                None => copy(&rpc, &session, &pane, id).await,
            };
            match text {
                Ok(text) => shared.change(|s| s.view.copied = Some((pane, text))),
                Err(error) => shared.fail(format!("Nothing was copied: {error:#}")),
            }
        });
        if let Some(older) = self.copying.replace(fetch) {
            older.abort();
        }
    }

    /// Take a pane on show for this client, unless it has it, so that the pane is
    /// the size this window gives it. The asking goes ahead of the input it is for,
    /// and the server takes the two in that order. It is made again until a view
    /// says the pane is taken, which does no harm. Only one who has joined the
    /// session may ask.
    async fn claim(&mut self, session: &str, pane: &str) {
        let joined = !self.joining && self.link.attached.as_deref() == Some(session);
        let another = |held: &Pane| joined && !held.owner;
        if self.shared.lock().view.pane(pane).is_some_and(another) {
            let request = json!({"session_id": session, "block_ids": [pane], "mode": "take"});
            self.ask(Reply::Input, "session.own_block", request).await;
        }
    }

    /// The session that holds a pane, which is the one to name in asking anything
    /// of the pane, whatever has come on show since.
    fn session(&self, pane: &str) -> Result<String> {
        let state = self.shared.lock();
        let (session, ..) = state.view.find(pane).context("That pane has closed")?;
        Ok(session.id.clone())
    }

    /// Do what the UI asked, which is to ask it of the server and go on: typing
    /// never waits on a round trip. `pick` is the asking's number, if by it the user
    /// asked for something to be shown.
    async fn perform(&mut self, action: Action, pick: u64) -> Result<()> {
        // A change to a pane names the pane, and its session is found from that.
        let (session, method, mut payload, made) = match action {
            // What is on show is as asked already, and `show` joins or leaves.
            Action::SelectSession(_) | Action::Leave => {
                self.settled = self.settled.max(pick);
                self.show().await;
                return Ok(());
            }
            Action::CreateSession(name) => {
                let mut payload = json!({"initial_windows": [{"layout": shell()}]});
                if let Some(name) = name {
                    payload["label"] = json!(name);
                }
                (String::new(), "session.create", payload, None)
            }
            Action::SelectWindow { session, window } => {
                let payload = json!({"window_id": window});
                (session, "session.focus_window", payload, None)
            }
            Action::CreateWindow(session) => {
                let payload = json!({"layout": shell(), "focus": true});
                (session, "session.new_window", payload, None)
            }
            Action::Edit {
                session,
                window,
                edit,
            } => {
                let (method, mut payload) = match (edit, window.is_some()) {
                    (Edit::Rename(name), true) => {
                        ("session.set_window_label", json!({"label": name}))
                    }
                    (Edit::Rename(name), false) => ("session.set_label", json!({"label": name})),
                    (Edit::MoveBefore(before), true) => {
                        ("session.move_window", json!({"before_window_id": before}))
                    }
                    (Edit::MoveBefore(before), false) => {
                        ("session.move", json!({"before_session_id": before}))
                    }
                    (Edit::Close, true) => ("session.close_window", json!({})),
                    (Edit::Close, false) => ("session.destroy", json!({})),
                };
                if let Some(window) = window {
                    payload["window_id"] = json!(window);
                }
                (session, method, payload, None)
            }
            Action::Split { pane, direction } => {
                let payload = json!({
                    "anchor_id": pane,
                    "direction": match direction {
                        Direction::Right => "horizontal",
                        Direction::Down => "vertical",
                    },
                    "side": "after",
                    "layout": shell(),
                    "focus": true,
                });
                (self.session(&pane)?, "session.new_split", payload, None)
            }
            Action::Divide(borders) => {
                let request = |border: &Division| {
                    anyhow::Ok(json!({
                        "session_id": self.session(&border.before)?,
                        "direction": if border.upright { "horizontal" } else { "vertical" },
                        "before_block_id": border.before,
                        "after_block_id": border.after,
                        "ratio": border.ratio,
                    }))
                };
                let requests = borders.iter().map(request).collect::<Result<_>>()?;
                let _ = self.borders.send(requests);
                return Ok(());
            }
            Action::Zoom { pane } => {
                let session = self.session(&pane)?;
                // Only an owner may resize it to the room it is given.
                self.claim(&session, &pane).await;
                let payload = json!({"block_id": pane});
                (session, "session.zoom_block", payload, None)
            }
            Action::ClosePane { pane } => {
                let payload = json!({"block_id": pane});
                (self.session(&pane)?, "block.close", payload, None)
            }
            Action::Reveal { pane } => {
                // Focus ends a zoom that hides the pane, and brings back a floating
                // layer that was put away.
                let payload = json!({"block_id": pane});
                let session = self.session(&pane)?;
                (session, "session.focus_block", payload, Some(pane))
            }
            Action::FocusPane { pane } => {
                let session = self.session(&pane)?;
                self.claim(&session, &pane).await;
                let payload = json!({"block_id": pane});
                (session, "session.focus_block", payload, None)
            }
            Action::Perform {
                name,
                args,
                origin,
                pane,
            } => {
                // A terminal's action is done to a pane, in the pane's session.
                // One of the configuration's is given what there is.
                let session = match &pane {
                    Some(pane) => Some(self.session(pane)?),
                    None => self.shared.lock().view.selected_session.clone(),
                };
                let mut payload = json!({"action": name, "origin": origin});
                let (session, pane) = (json!(session), json!(pane));
                for (key, value) in [("args", args), ("session_id", session), ("block_id", pane)] {
                    if !value.is_null() {
                        payload[key] = value;
                    }
                }
                self.ask(Reply::Performed, "client.action", payload).await;
                return Ok(());
            }
            Action::Reply { call, error } => {
                // Whoever asked has stopped waiting if this comes too late, and
                // there is nobody to tell.
                let _ = self.rpc.enqueue("client.reply", reply(&call, error)).await;
                return Ok(());
            }
            input => return self.input(input).await,
        };
        // A session yet to be made has no name to go by.
        if !session.is_empty() {
            payload["session_id"] = json!(session);
        }
        self.unsettled += usize::from(pick > 0);
        let reply = Reply::Change(session, pick, made);
        self.ask(reply, method, payload).await;
        self.show().await;
        Ok(())
    }

    /// Queue input for a terminal, in the order it came.
    async fn input(&mut self, action: Action) -> Result<()> {
        let event =
            |kind: &str, payload: Value| json!({"event": {"type": kind, "payload": payload}});
        // Input that engages the terminal takes ownership and returns to live
        // output. A size engages nothing, and is taken by a terminal not yet seen.
        let (pane, method, mut payload, engage) = match action {
            Action::Resize { pane, size } => {
                ensure!(
                    size.cols > 0 && size.rows > 0,
                    "Terminal dimensions must be nonzero"
                );
                // The server holds the size asked by a client that does not own the
                // terminal, for when it does.
                let args = json!({
                    "columns": size.cols,
                    "rows": size.rows,
                    "cell_width_px": size.cell_width,
                    "cell_height_px": size.cell_height,
                });
                let method = "com.superlogical.terminal.resize";
                (pane, method, json!({"args": args}), None)
            }
            Action::Key {
                pane,
                key,
                code,
                modifiers,
                pressed,
                repeat,
                prefix,
            } => {
                let stroke = match (pressed, repeat) {
                    (false, _) => "release",
                    (true, false) => "press",
                    (true, true) => "repeat",
                };
                let mut payload = json!({"key": key_payload(&key, &code, modifiers, stroke)});
                if !prefix.is_empty() {
                    payload["prefix"] = json!(prefix);
                }
                // A modifier held alone asks nothing of the terminal.
                let engage = pressed && !model::is_modifier(&code);
                (pane, "client.key", payload, Some(engage))
            }
            Action::Text { pane, text } => {
                let payload = json!({"key": key_payload(&text, "unidentified", 0, "press")});
                (pane, "client.key", payload, Some(true))
            }
            Action::Paste { pane, text } => {
                let payload = event("paste", json!({"text": text}));
                (pane, "block.event", payload, Some(true))
            }
            Action::Mouse {
                pane,
                kind,
                x,
                y,
                button,
                modifiers,
            } => {
                // Shift is the local override for mouse capture; programs never see it.
                let mods = modifier_names(modifiers & !model::SHIFT);
                let mut mouse = json!({"action": kind, "x": x, "y": y, "mods": mods});
                if let Some(button) = button {
                    mouse["button"] = json!(button);
                }
                (
                    pane,
                    "block.event",
                    event("mouse", mouse),
                    Some(kind == "press"),
                )
            }
            Action::Wheel { pane, x, y, lines } => {
                let wheel = json!({"x": x, "y": y, "delta_x": 0, "delta_y": lines});
                (pane, "block.event", event("wheel", wheel), Some(false))
            }
            // The server passes it on only to a program that asked to be told.
            Action::Focus { pane, gained } => {
                let focus = json!({"action": if gained { "in" } else { "out" }});
                (pane, "block.event", event("focus", focus), Some(false))
            }
            // Done by `perform`, or by the connection loop.
            _ => return Ok(()),
        };
        // The session is the terminal's own, whatever has come on show since the
        // input was made. What is no terminal, or is gone, takes none.
        let Some(block) = self.link.blocks.get(&pane) else {
            return Ok(());
        };
        let session = block.session.clone();
        payload["session_id"] = json!(session);
        payload["block_id"] = json!(pane);
        if let Some(engage) = engage {
            let ready = self
                .shared
                .terminal(&pane)
                .filter(|terminal| terminal.latest().is_some() && terminal.status().is_none());
            let Some(terminal) = ready else {
                // A release or a motion may trail a pane that has left the screen.
                // Only input that would have done something is worth a complaint.
                ensure!(!engage, "Terminal is not ready; input was not sent");
                return Ok(());
            };
            if engage {
                self.claim(&session, &pane).await;
                terminal.ask(Request::Scroll(Scroll::Bottom));
            }
        }
        self.ask(Reply::Input, method, payload).await;
        Ok(())
    }
}

/// The answer to another client's asking: that what it asked was done, or the
/// code of an error and the words for it.
fn reply(call: &str, error: Option<&str>) -> Value {
    let Some(code) = error else {
        return json!({"call_id": call, "result": null});
    };
    let message = match code {
        "refused" => "Rex Lite cannot do that just now",
        _ => "Rex Lite has no such action",
    };
    json!({"call_id": call, "error": {"code": code, "message": message}})
}

/// The whole of a long text that a program has copied, asked for a part at a time.
async fn copy(rpc: &Rpc, session: &str, pane: &str, id: u64) -> Result<String> {
    let mut text = String::new();
    loop {
        let args = json!({"id": id, "offset": text.len(), "max_bytes": COPY_PART_BYTES});
        let request = json!({"session_id": session, "block_id": pane, "args": args});
        let part = rpc.call(COPIED, request).await?;
        let more = part["data"].as_str().unwrap_or_default();
        text.push_str(more);
        ensure!(text.len() <= MAX_COPY_BYTES, "it is too long");
        if part["eof"] == true {
            return Ok(text);
        }
        ensure!(!more.is_empty(), "the server sent only part of it");
    }
}

/// Keep one pane's terminal fed for as long as it is on show, restoring a fresh
/// snapshot whenever its stream fails: soon at first, and seldom if it keeps failing.
async fn stream_pane(
    target: wire::Target,
    rpc: Rpc,
    shared: Shared,
    session: String,
    pane: String,
) {
    let mut failures = 0;
    loop {
        let began = std::time::Instant::now();
        let Err(error) = stream(&target, &rpc, &shared, &session, &pane).await;
        // One that lasted was working. One that draws and fails, over and over, is not.
        if began.elapsed() > RETRY[1] {
            failures = 0;
        }
        let wait = retry(&error, failures, true);
        failures += 1;
        shared.change(|s| {
            s.rest(&pane);
            if let Some(pane) = s.view.pane_mut(&pane) {
                pane.status = match wait {
                    Some(_) => format!("Reconnecting terminal: {error:#}"),
                    None => format!("{error:#}"),
                };
            }
        });
        // A terminal that has closed has nothing to restore, and the next view
        // is without its pane.
        let Some(wait) = wait else {
            return;
        };
        tokio::time::sleep(wait).await;
    }
}

async fn stream(
    target: &wire::Target,
    rpc: &Rpc,
    shared: &Shared,
    session: &str,
    pane: &str,
) -> Result<Infallible> {
    let request = json!({
        "session_id": session,
        "block_id": pane,
        "data_ref_id": "terminal",
        "subprotocol": wire::DATA_PROTOCOL,
    });
    let response = rpc.call("data.connect", request).await?;
    let ticket = response["ticket"]
        .as_str()
        .context("Missing terminal ticket")?;
    let mut socket = wire::socket(target, "/api/data", wire::DATA_PROTOCOL).await?;
    let nonce = wire::handshake(&mut socket, ticket).await?;
    // The worker says when it has drawn, and so when it has stopped. One that
    // stops while its terminal is quiet would otherwise go unnoticed here, and
    // its pane take no input, until the next output came.
    let (wake, drawn) = (shared.wake.clone(), Arc::new(tokio::sync::Notify::new()));
    let said = drawn.clone();
    let repaint = move || {
        wake.notify();
        said.notify_one();
    };
    let terminal = TerminalHandle::new(nonce, Arc::new(repaint))?;
    shared.change(|s| {
        s.terminals.insert(pane.into(), terminal.clone());
        s.last.remove(pane);
    });
    loop {
        tokio::select! {
            bytes = wire::next_binary(&mut socket) => terminal.feed(bytes?).await?,
            _ = drawn.notified() => {
                if let Some(error) = terminal.status() {
                    bail!(error);
                }
            }
        }
    }
}

fn items(value: &Value) -> impl Iterator<Item = &Value> + Clone {
    value.as_array().into_iter().flatten()
}

fn label(value: Option<&str>, fallback: &str) -> String {
    value.filter(|s| !s.is_empty()).unwrap_or(fallback).into()
}

/// The entry for one block in a server block list, or null.
fn block<'a>(blocks: &'a Value, id: &str) -> &'a Value {
    items(blocks).find(|b| b["block_id"] == id).unwrap_or(&NULL)
}

/// The panes to take, of those the server says are `owned` by whom: the ones whose
/// owner is not among the clients `present` on it. A pane stays with an owner that
/// has left the server without ever joining the session, as one that only made the
/// session has, and nobody is left to let go of it. Nothing is taken from a client
/// that is there, nor when who is there is not known.
fn abandoned<'a>(owned: &'a Value, present: &Value) -> Vec<&'a Value> {
    let here = |owner: &str| items(present).any(|client| client["client_id"] == owner);
    let gone = |block: &&Value| block["owner"].as_str().is_some_and(|owner| !here(owner));
    let gone = items(owned).filter(gone).map(|block| &block["block_id"]);
    gone.filter(|_| present.is_array()).collect()
}

/// Bring the list of sessions up to date with news of it. Told the same thing
/// twice, it is as it was when told once.
fn list(sessions: &mut Vec<Session>, event: &Event) {
    let at = |sessions: &[Session], id: &str| sessions.iter().position(|s| s.id == id);
    match event {
        Event::Created { session, name } if at(sessions, session).is_none() => {
            sessions.push(Session {
                id: session.clone(),
                name: label(Some(name), "Untitled"),
                ..Default::default()
            });
        }
        Event::Destroyed { session } => sessions.retain(|held| &held.id != session),
        Event::Moved { session, before } => {
            let Some(from) = at(sessions, session) else {
                return;
            };
            let moved = sessions.remove(from);
            let to = before.as_ref().and_then(|before| at(sessions, before));
            sessions.insert(to.unwrap_or(sessions.len()), moved);
        }
        _ => {}
    }
}

/// Move borders as they are asked to be moved, one request at a time: a drag asks
/// faster than a server answers, and every move reflows the terminals either side.
/// Only where borders were last asked to go is asked of the server, once it has
/// answered for the time before. The view that comes of it is in the server's news.
fn divide(rpc: Rpc, shared: Shared) -> watch::Sender<Vec<Value>> {
    let (borders, mut asked) = watch::channel(Vec::new());
    tokio::spawn(async move {
        // Until the connection is dropped, and the asking with it.
        while asked.changed().await.is_ok() {
            let requests = asked.borrow_and_update().clone();
            for request in requests {
                // What the server would not do is said. That the connection ended
                // is said already, above its last screen.
                if let Err(error) = rpc.call("session.resize_pane", request).await
                    && error.to_string() != ENDED
                {
                    shared.fail(format!("{error:#}"));
                }
            }
        }
    });
    borders
}

/// The panes under one node of a layout, each with the place the server gives it:
/// a fraction of the tab. With them, the border of every split under it. A layout
/// is no deeper than the JSON it came in, which the parser keeps to 128 levels.
fn layout(node: &Value, panes: &mut Vec<(String, [f32; 4])>, borders: &mut Vec<Border>) {
    let Some(room) = rect(&node["rect"]) else {
        return;
    };
    if let Some(id) = node["block_id"].as_str() {
        return panes.push((id.into(), room));
    }
    let split = &node["split"];
    let before = panes.len();
    layout(&split["before"], panes, borders);
    let after = panes.len();
    layout(&split["after"], panes, borders);
    // The first pane of each side names the split, and the second side begins at
    // the border. A side with no pane has no border to it.
    if let (Some((first, _)), Some((second, place))) =
        (panes[..after].get(before), panes.get(after))
    {
        // A direction not known is drawn as this one.
        let upright = split["direction"] != "vertical";
        borders.push(Border {
            before: first.clone(),
            after: second.clone(),
            upright,
            room,
            at: place[usize::from(!upright)],
        });
    }
}

/// The tabs of a view. The panes of them that are terminals are named in `terminals`.
fn parse_windows(view: &Value, client_id: &str, terminals: &mut Vec<String>) -> Vec<Window> {
    items(&view["windows"])
        .filter_map(|window| {
            let id: String = window["window_id"].as_str()?.into();
            let zoom = window["zoomed_block_id"].as_str();
            let (mut panes, mut hidden, mut borders) = (Vec::new(), Vec::new(), Vec::new());
            for layer in items(&window["layers"]) {
                let (mut placed, mut split) = (Vec::new(), Vec::new());
                layout(&layer["layout"], &mut placed, &mut split);
                // Only the layer that fills the tab has borders to drag, while its
                // panes are all on show and no other floats over them.
                if layer["kind"] == "tiled" && zoom.is_none() {
                    borders = split;
                } else if layer["state"] != "minimized" && !placed.is_empty() {
                    borders.clear();
                }
                for (id, bounds) in placed {
                    let block = block(&layer["blocks"], &id);
                    // What is not a terminal keeps its place, and says what it is.
                    let creator = block["creator_name"].as_str();
                    let other = creator.filter(|creator| *creator != TERMINAL);
                    if other.is_none() {
                        terminals.push(id.clone());
                    }
                    // A zoomed pane fills the tab, and the rest are out of sight behind
                    // it. So are those of a floating layer that has been put away.
                    let (panes, bounds) = match zoom {
                        _ if layer["state"] == "minimized" => (&mut hidden, bounds),
                        Some(zoom) if zoom == id => (&mut panes, FULL),
                        Some(_) => (&mut hidden, bounds),
                        None => (&mut panes, bounds),
                    };
                    let [x, y, width, height] = bounds;
                    panes.push(Pane {
                        id,
                        title: label(block["label"].as_str(), other.unwrap_or("Terminal")),
                        program: Program::default(),
                        activity: Activity::Idle,
                        message: String::new(),
                        task: String::new(),
                        progress: None,
                        x,
                        y,
                        width,
                        height,
                        frame: None,
                        status: match other {
                            Some(other) => format!("{other}: not a terminal"),
                            None => "Connecting terminal…".into(),
                        },
                        owner: block["owner"] == client_id,
                        idle: false,
                    });
                }
            }
            // A tab nobody named is labelled by the server with the end of its ID.
            let label = window["label"].as_str().filter(|label| {
                let unnamed = label.strip_prefix("Window ");
                !label.is_empty() && !unnamed.is_some_and(|tail| id.ends_with(tail))
            });
            Some(Window {
                label: label.map(str::to_owned),
                focus: window["focused_block_id"].as_str().map(str::to_owned),
                zoomed: zoom.is_some(),
                id,
                panes,
                hidden,
                borders,
            })
        })
        .collect()
}

/// What one record of a program's says it is doing.
fn state(record: &Value) -> Activity {
    match record["state"].as_str() {
        Some("working") => Activity::Working,
        Some("done") => Activity::Done,
        Some("error") => Activity::Error,
        Some("blocked") => Activity::Blocked(match record["kind"].as_str() {
            Some("permission") => Wait::Approval,
            Some("question") => Wait::Question,
            Some("auth") => Wait::SignIn,
            _ => Wait::Input,
        }),
        _ => Activity::Idle,
    }
}

/// What a terminal's programs say they are doing, of all that each has reported:
/// the most pressing of it, the record that says so, and the icon of one still
/// at it.
fn reported<'a>(
    records: impl Iterator<Item = &'a Value> + Clone,
) -> (Activity, &'a Value, Option<Program>) {
    // Of several as pressing, the first: a program comes before its parts.
    let records = || records.clone().map(|record| (state(record), record));
    let top = records().min_by_key(|(state, _)| Reverse(*state));
    // What a program that has finished left behind says nothing of what runs now.
    let live = records().filter(|(state, _)| !matches!(state, Activity::Done | Activity::Error));
    let app = Program::named(live.filter_map(|(_, record)| record["effective_app"].as_str()));
    let (top, record) = top.unwrap_or((Activity::Idle, &Value::Null));
    (top, record, app)
}

/// Give every pane what is known of its terminal.
fn fill(sessions: &mut [Session], blocks: &HashMap<String, Block>) {
    let windows = sessions.iter_mut().flat_map(|s| &mut s.windows);
    for pane in windows.flat_map(Window::every_mut) {
        if let Some(block) = blocks.get(&pane.id) {
            pane.idle = block.shell.is_some() && block.shell == block.foreground;
            // At its shell's prompt it goes by the shell and where that is: the
            // title a prompt sets tells who is at what machine first, and at
            // length. One with no title keeps its label.
            if pane.idle && !block.folder.is_empty() {
                let named = format!("{} {}", block.prompt, model::folder(&block.folder));
                named.trim_start().clone_into(&mut pane.title);
            } else if !block.title.is_empty() {
                pane.title.clone_from(&block.title);
            }
            // What it is doing, by the record that speaks for it: the line said of
            // that, what it is at, and how far along. Failing the last, how far the
            // terminal itself was told.
            let (activity, record, app) = reported(block.records.values());
            let said = |key: &str| record[key].as_str().unwrap_or_default();
            pane.program = app.or(block.process).unwrap_or_default();
            pane.activity = activity;
            said("msg").clone_into(&mut pane.message);
            said("title").clone_into(&mut pane.task);
            pane.progress = wire::percent(&record["progress"]).or(block.progress);
        }
    }
}

fn rect(value: &Value) -> Option<[f32; 4]> {
    let out: [f32; 4] = serde_json::from_value(value.clone()).ok()?;
    (out.iter().all(|x| x.is_finite()) && out[2] > 0. && out[3] > 0.).then_some(out)
}

fn modifier_names(bits: u8) -> String {
    [
        (model::SHIFT, "shift"),
        (model::CTRL, "ctrl"),
        (model::ALT, "alt"),
        (model::SUPER, "super"),
    ]
    .into_iter()
    .filter_map(|(bit, name)| (bits & bit != 0).then_some(name))
    .collect::<Vec<_>>()
    .join("+")
}

/// A key as the server is told of it. `key` is what it types, `code` the name of
/// the key itself, and `stroke` is `press`, `repeat` or `release`.
fn key_payload(key: &str, code: &str, bits: u8, stroke: &str) -> Value {
    // Releases and Control/Super chords do not type anything.
    let text = if stroke != "release" && bits & (model::CTRL | model::SUPER) == 0 {
        key
    } else {
        ""
    };
    let shift = bits & model::SHIFT != 0;
    // Shift is used up in typing a capital or a symbol. In Shift+Tab it is not, nor
    // in Shift+Enter or Shift+Space: those type what they would without it.
    let shifted = shift && text.chars().any(|c| !c.is_control() && c != ' ');
    json!({
        "key": code,
        "mods": modifier_names(bits),
        "consumed_mods": if shifted { "shift" } else { "" },
        "text": text,
        "unshifted_codepoint": unshifted(key, code, shift),
        "composing": false,
        "action": stroke,
    })
}

/// The character a key types without Shift, or 0. What it typed says, unless Shift
/// was held. Then a letter is itself in small, and any other key is taken for the
/// one of a US keyboard, which is all that is known of it.
fn unshifted(key: &str, code: &str, shift: bool) -> u32 {
    let mut chars = key.chars();
    let typed = match (chars.next(), chars.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    };
    let legend = || {
        Some(match code {
            "space" => ' ',
            "tab" => '\t',
            "enter" => '\n',
            "backquote" => '`',
            "backslash" | "intl_backslash" => '\\',
            "bracket_left" => '[',
            "bracket_right" => ']',
            "comma" => ',',
            "equal" => '=',
            "minus" => '-',
            "period" => '.',
            "quote" => '\'',
            "semicolon" => ';',
            "slash" => '/',
            _ => {
                let name = code.strip_prefix("digit_");
                let mut name = name.or_else(|| code.strip_prefix("key_"))?.chars();
                name.next().filter(|_| name.next().is_none())?
            }
        })
    };
    let small = |c: char| c.to_lowercase().next().unwrap_or(c);
    let unshifted = match typed {
        Some(typed) if !shift || typed.is_alphabetic() => Some(small(typed)),
        _ => legend().or(typed),
    };
    unshifted.map_or(0, u32::from)
}

fn shell() -> Value {
    json!({"block": {"flavor": "com.superlogical.terminal.shell"}})
}

/// Read-only connectivity probe used by --probe: no sessions are created and no input is sent.
pub fn probe(endpoint: String, timeout: Duration) -> Result<String> {
    let client = Client::spawn(vec![endpoint], Arc::new(|| {}), Herald::new().0, None);
    let started = std::time::Instant::now();
    loop {
        let view = client.snapshot();
        // As a window does of the server it shows.
        if let Some(first) = view.unshown() {
            let _ = client.try_send(first);
            continue;
        }
        let frames = view.panes().filter(|p| p.frame.is_some()).count();
        if view.connected && (view.windows().is_empty() || frames > 0) {
            return Ok(format!(
                "Connected; {} sessions, {} tabs, {} decoded terminal frames",
                view.sessions.len(),
                view.windows().len(),
                frames
            ));
        }
        let pane_statuses = view
            .panes()
            .map(|p| p.status.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        ensure!(
            started.elapsed() < timeout,
            "Probe timed out: {} · {}",
            view.status,
            pane_statuses
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
#[path = "../tests/unit/backend.rs"]
mod tests;
