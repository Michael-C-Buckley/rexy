//! A thin Rex client: one I/O runtime, bounded actions, separate terminal CPU workers.
use crate::{
    model::{self, Action, Activity, Direction, Edit, Host, Pane, Session, View, Wait, Window},
    notify::Herald,
    program::Program,
    ssh,
    terminal::{Notify, TerminalHandle},
    wire::{self, Control, ENDED, Event, Response, Rpc},
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

const RETRY_DELAY: Duration = Duration::from_secs(2);
/// How long to wait before each try at a server that will not connect. After these
/// it is left alone until the user asks: a machine that keeps refusing a login
/// may come to refuse the address it comes from.
const RECONNECT: [Duration; 2] = [RETRY_DELAY, Duration::from_secs(15)];
const FULL: [f32; 4] = [0., 0., 1., 1.];
const TITLE: &str = "com.superlogical.terminal.title";
const PROCESS: &str = "com.superlogical.terminal.process";
const STATUS: &str = "com.superlogical.terminal.program_status";
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
    terminals: HashMap<String, TerminalHandle>,
    /// The UI is showing another server, so no session is picked unasked.
    away: bool,
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
    fn owns(&self, pane: &str) -> bool {
        self.lock().view.pane(pane).is_some_and(|p| p.owner)
    }
}

pub struct Client {
    actions: mpsc::Sender<Action>,
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
    /// connected. Unless `away`, its first session is shown without being asked for.
    /// What its panes have to say to someone not looking goes to `herald`.
    pub fn spawn(endpoints: Vec<String>, away: bool, wake: Notify, herald: Herald) -> Self {
        let (ended, done) = flume::bounded(0);
        let state = State {
            away,
            ..Default::default()
        };
        let shared = Shared {
            state: Arc::new(Mutex::new(state)),
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
                    Ok(runtime) => runtime.block_on(run(endpoints, receiver, worker)),
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
        state.view.rang.clear();
        for pane in view.panes_mut() {
            if let Some(terminal) = state.terminals.get(&pane.id) {
                pane.frame = terminal.latest();
                pane.status = match terminal.status() {
                    Some(error) => error,
                    None if pane.frame.is_some() => String::new(),
                    None => "Restoring terminal…".into(),
                };
            }
        }
        view
    }

    pub fn try_send(&self, action: Action) -> Result<(), String> {
        // Queue under the state lock. A failed connection is marked disconnected
        // under the same lock before its queue is discarded, so input accepted
        // on it can never reach its replacement.
        let mut state = self.shared.lock();
        // Asking a server to show something is how the UI comes to it, as `leave` is
        // how it goes. Only these say where the UI is, and in the order it moved.
        let show = matches!(
            action,
            Action::SelectSession(_)
                | Action::SelectWindow { .. }
                | Action::CreateSession(_)
                | Action::CreateWindow(_)
        );
        if show {
            state.away = false;
        }
        if !state.view.connected && !matches!(action, Action::Reconnect) {
            return Err("Disconnected; input was not sent".into());
        }
        let shown = match &action {
            Action::SelectSession(session) => Some((session.clone(), None)),
            Action::SelectWindow { session, window } => {
                Some((session.clone(), Some(window.clone())))
            }
            _ => None,
        };
        self.actions
            .try_send(action)
            .map_err(|e| format!("Input not sent: {e}"))?;
        // Show it at once; the server's view follows.
        if let Some((id, window)) = shown
            && let Some(session) = state.view.sessions.iter().find(|s| s.id == id)
        {
            let window = window.or_else(|| session.active.clone());
            state.view.selected_session = Some(id);
            state.view.selected_window = window;
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
        state.away = true;
        state.view.selected_session = None;
        state.view.selected_window = None;
        // Even with nothing on show: what it was last asked may be on its way to being.
        if state.view.connected {
            let _ = self.actions.try_send(Action::Leave);
        }
    }

    /// The terminal of a pane on show, for what is done to it here and never
    /// sent to the server, such as moving through its history.
    pub fn terminal(&self, pane: &str) -> Result<TerminalHandle, String> {
        let terminal = self.shared.terminal(pane);
        terminal.ok_or_else(|| "Terminal is not ready".into())
    }
}

async fn run(endpoints: Vec<String>, mut actions: mpsc::Receiver<Action>, shared: Shared) {
    let mut token = None;
    // Tries in a row that never connected.
    let mut failures = 0usize;
    loop {
        shared.status("Connecting…", false);
        let result = run_connected(&endpoints, &mut token, &mut actions, &shared).await;
        if actions.is_closed() {
            return;
        }
        failures = if shared.lock().view.connected {
            0
        } else {
            failures + 1
        };
        let delay = RECONNECT.get(failures.saturating_sub(1));
        shared.change(|s| {
            // Keep the last immutable screen for context, but never send old input again.
            for pane in s.view.panes_mut() {
                if let Some(terminal) = s.terminals.get(&pane.id) {
                    pane.frame = terminal.latest();
                }
                pane.status = "Disconnected".into();
                pane.owner = false;
            }
            s.terminals.clear();
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
            Some(delay) => tokio::time::timeout(*delay, actions.recv()).await,
            None => Ok(actions.recv().await),
        };
        if let Ok(None) = asked {
            return;
        }
    }
}

/// Reach the server at the first endpoint that answers. An `ssh://` endpoint is
/// reached through a tunnel, which lasts as long as it is held.
async fn connect(
    endpoints: &[String],
    token: Option<&str>,
) -> Result<(Option<ssh::Tunnel>, Control)> {
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
            let control = Control::connect(&endpoint, token).await?;
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
    token: &mut Option<String>,
    actions: &mut mpsc::Receiver<Action>,
    shared: &Shared,
) -> Result<()> {
    let (_tunnel, mut control) = connect(endpoints, token.as_deref()).await?;
    *token = control.reconnect_token.take();
    let target = control.target.clone();
    let mut connection = Connection {
        target: &target,
        shared,
        rpc: control.rpc.clone(),
        client_id: control.client_id.clone(),
        attached: None,
        blocks: HashMap::new(),
        stale: HashSet::new(),
        streams: HashMap::new(),
    };
    connection.refresh().await?;
    while actions.try_recv().is_ok() {} // Requests to try again have been answered.
    shared.status("", true);
    connection.reconcile();
    let mut replies = FuturesUnordered::new();
    let mut refresh = tokio::time::interval(Duration::from_millis(75));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = false;
    loop {
        tokio::select! {
            result = &mut control.task => {
                return result.context("Rex control worker stopped")?;
            }
            // When there are no more, the task above says why.
            Some(event) = control.events.recv() => match event {
                Event::Structure => dirty = true,
                Event::Title { block, title } => connection.note(&block, |b| b.title = title),
                Event::Program { block, program, pid } => connection.note(&block, |b| {
                    b.process = program;
                    b.foreground = pid;
                }),
                Event::Status { block } => {
                    connection.stale.insert(block);
                }
                Event::Bell { block } => connection.rang(block),
                Event::Notice { block, title, body } => {
                    connection.tell(&block, &model::line(&title, &body));
                }
                Event::Copied { block, text, id } => {
                    if let Err(error) = connection.copied(block, text, id).await {
                        shared.fail(format!("Nothing was copied: {error:#}"));
                    }
                }
            },
            _ = refresh.tick(), if dirty || !connection.stale.is_empty() => {
                if std::mem::take(&mut dirty) {
                    connection.refresh().await?;
                    connection.reconcile();
                } else {
                    connection.statuses().await?;
                }
            }
            action = actions.recv(), if replies.len() < 64 => {
                let Some(action) = action else {
                    // The client is going. The server need not keep its place.
                    control.close().await;
                    return Ok(());
                };
                if matches!(action, Action::Reconnect) {
                    return Ok(());
                }
                match connection.perform(action).await {
                    Ok(Some(reply)) => replies.push(reply),
                    Ok(None) => {}
                    Err(error) => shared.fail(format!("{error:#}")),
                }
                connection.reconcile();
            }
            reply = replies.next(), if !replies.is_empty() => match reply {
                Some(Ok(Err(error))) => shared.fail(format!("Input failed: {error}")),
                Some(Err(_)) => bail!("Control connection ended; input was not retried"),
                Some(Ok(Ok(_))) | None => {}
            }
        }
    }
}

/// The state of one control connection. Dropping it stops every terminal stream.
struct Connection<'a> {
    /// The address that answered, of the several a host name can stand for.
    /// Terminal streams go straight to it.
    target: &'a wire::Target,
    shared: &'a Shared,
    rpc: Rpc,
    client_id: String,
    /// The session this connection has joined: the one on show.
    attached: Option<String>,
    /// What is known of every terminal on the server, shown or not.
    blocks: HashMap<String, Block>,
    /// Terminals whose programs have reported since they were last asked.
    stale: HashSet<String>,
    /// One stream per pane of the visible tab.
    streams: HashMap<String, JoinHandle<()>>,
}

struct Block {
    session: String,
    title: String,
    /// The icon of the program in its foreground, if it has one.
    process: Option<Program>,
    /// The process of the shell it started with, if it started with one, and of
    /// what is in its foreground. Where they are one, the shell is at its prompt.
    shell: Option<u64>,
    foreground: Option<u64>,
    /// That of a program reporting its status, which tells more, while one is.
    app: Option<Program>,
    activity: Activity,
    /// The line said of that.
    message: String,
}

impl Block {
    /// What it has to say to someone not looking, if that is other than it was `before`.
    fn news(&self, before: Activity) -> Option<String> {
        let word = self.activity.word().filter(|_| self.activity != before)?;
        Some(model::line(word, &self.message))
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        for task in self.streams.values() {
            task.abort();
        }
    }
}

impl Connection<'_> {
    async fn refresh(&mut self) -> Result<()> {
        // A server too old to keep a host list simply has none.
        if let Ok(result) = self.rpc.call("hosts.list", json!({})).await {
            let hosts = items(&result["hosts"]).filter_map(|host| {
                let endpoints = items(&host["endpoints"]).filter_map(Value::as_str);
                Some(Host {
                    label: host["label"].as_str()?.into(),
                    endpoints: endpoints.map(str::to_owned).collect(),
                })
            });
            let hosts = hosts.collect();
            self.shared.change(|s| s.view.hosts = hosts);
        }
        self.sessions(None).await
    }

    /// Adopt the server's sessions and their tabs, and join the one on show: `show`
    /// if it is given, or else the one already on show.
    async fn sessions(&mut self, show: Option<&str>) -> Result<()> {
        // Events of every session, so that tabs not on show keep their titles too.
        let scope = json!({"event_scope": "all"});
        let listed = self.rpc.call("session.attach_all", scope).await?;
        let mut sessions: Vec<_> = items(&listed["sessions"])
            .filter_map(|s| {
                Some(Session {
                    id: s["session_id"].as_str()?.into(),
                    name: label(&s["label"], "Untitled"),
                    active: None,
                    windows: Vec::new(),
                })
            })
            .collect();
        let listed = |id: &str| sessions.iter().any(|s| s.id == id);
        let shown = show.filter(|id| listed(id)).map(str::to_owned);
        let attached = {
            let state = self.shared.lock();
            let selected = state.view.selected_session.clone();
            let known = shown.clone().or(selected.filter(|id| listed(id)));
            // Failing those the first, and none at all while the UI is showing another
            // server: it may have left since it asked for this one.
            let first = || sessions.first().map(|s| s.id.clone());
            known.or_else(first).filter(|_| !state.away)
        };
        // Joining is idempotent. The other sessions are only looked at.
        let mut views = Vec::new();
        for session in &sessions {
            let method = if Some(&session.id) == attached.as_ref() {
                "session.attach"
            } else {
                "session.view"
            };
            let request = json!({"session_id": session.id});
            views.push(self.rpc.enqueue(method, request).await?);
        }
        for (session, view) in sessions.iter_mut().zip(views) {
            // One that ended a moment ago has no tabs.
            let Ok(view) = view.await.context(ENDED)? else {
                continue;
            };
            let view = view.get("view").unwrap_or(&view);
            session.active = view["active_window_id"].as_str().map(str::to_owned);
            session.windows = parse_windows(view, &self.client_id);
        }
        self.learn(&sessions).await?;
        self.statuses().await?;
        fill(&mut sessions, &self.blocks);
        self.shared.change(|s| {
            let view = &mut s.view;
            let selected = view.selected_session.as_ref();
            if shown.is_some() || !sessions.iter().any(|x| Some(&x.id) == selected) {
                // Not if the UI left for another server while the replies were on their way.
                view.selected_session = attached.clone().filter(|_| !s.away);
                view.selected_window = None;
            }
            // The tab on show is the one the server says, for every client alike.
            let session = sessions.iter().find(|x| Some(&x.id) == attached.as_ref());
            if let Some(session) = session.filter(|_| view.selected_session == attached) {
                let first = || session.windows.first().map(|w| w.id.clone());
                view.selected_window = session.active.clone().or_else(first);
            }
            view.sessions = sessions;
        });
        // Watch only the session on show.
        let previous = std::mem::replace(&mut self.attached, attached);
        if let Some(previous) = previous.filter(|id| Some(id) != self.attached.as_ref()) {
            let request = json!({"session_id": previous});
            let _ = self.rpc.call("session.detach", request).await;
        }
        Ok(())
    }

    /// Ask of each terminal not seen before its title, its program and what that says
    /// it is doing, and forget those that are gone.
    async fn learn(&mut self, sessions: &[Session]) -> Result<()> {
        let panes = || {
            sessions.iter().flat_map(|session| {
                let panes = session.windows.iter().flat_map(Window::every);
                panes.map(move |pane| (&session.id, &pane.id))
            })
        };
        let mut asked = Vec::new();
        for (session, pane) in panes().filter(|(_, pane)| !self.blocks.contains_key(*pane)) {
            let request = json!({"session_id": session, "block_id": pane, "args": {}});
            let title = self.rpc.enqueue(TITLE, request.clone()).await?;
            let process = self.rpc.enqueue(PROCESS, request.clone()).await?;
            let status = self.rpc.enqueue(STATUS, request).await?;
            asked.push((session, pane, title, process, status));
        }
        for (session, pane, title, process, status) in asked {
            // A server too old to be asked has nothing to say.
            let title = title.await.context(ENDED)?.unwrap_or_default();
            let process = process.await.context(ENDED)?.unwrap_or_default();
            let status = status.await.context(ENDED)?.unwrap_or_default();
            let (activity, message, app) = reported(&status["records"]);
            let block = Block {
                session: session.clone(),
                title: title["title"].as_str().unwrap_or_default().into(),
                process: wire::program(&process["foreground"]),
                shell: wire::shell(&process["child"]),
                foreground: process["foreground"]["pid"].as_u64(),
                app,
                activity,
                message,
            };
            // How a pane stood when first seen is not news. How one that the last
            // connection knew has changed since that was lost is.
            let known = self.shared.lock().view.find(pane).map(|(.., p)| p.activity);
            if let Some(said) = known.and_then(|before| block.news(before)) {
                self.tell(pane, &said);
            }
            self.blocks.insert(pane.clone(), block);
        }
        let seen: HashSet<_> = panes().map(|(_, pane)| pane).collect();
        self.blocks.retain(|id, _| seen.contains(id));
        Ok(())
    }

    /// Ask what the programs say they are doing, in each terminal that has
    /// reported since it was last asked.
    async fn statuses(&mut self) -> Result<()> {
        let mut replies = Vec::new();
        for id in std::mem::take(&mut self.stale) {
            let Some(block) = self.blocks.get(&id) else {
                continue;
            };
            let request = json!({"session_id": block.session, "block_id": id, "args": {}});
            replies.push((id, self.rpc.enqueue(STATUS, request).await?));
        }
        if replies.is_empty() {
            return Ok(());
        }
        let mut news = Vec::new();
        for (id, reply) in replies {
            let status = reply.await.context(ENDED)?.unwrap_or_default();
            let Some(block) = self.blocks.get_mut(&id) else {
                continue;
            };
            let before = block.activity;
            (block.activity, block.message, block.app) = reported(&status["records"]);
            news.extend(block.news(before).map(|said| (id, said)));
        }
        self.shared
            .change(|s| fill(&mut s.view.sessions, &self.blocks));
        for (pane, said) in news {
            self.tell(&pane, &said);
        }
        Ok(())
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

    /// Take in news of one terminal. One not seen yet is asked about when it is.
    fn note(&mut self, block: &str, news: impl FnOnce(&mut Block)) {
        if let Some(block) = self.blocks.get_mut(block) {
            news(block);
            self.shared
                .change(|s| fill(&mut s.view.sessions, &self.blocks));
        }
    }

    /// Hand the UI the text a program has copied, if its pane is on show. Text too
    /// long for the server to have sent with the news is asked for, a part at a time.
    async fn copied(&self, pane: String, text: Option<String>, id: u64) -> Result<()> {
        let Some(block) = self.blocks.get(&pane) else {
            return Ok(());
        };
        if !self.streams.contains_key(&pane) {
            return Ok(());
        }
        let text = match text {
            Some(text) => text,
            None => {
                let mut text = String::new();
                loop {
                    let args =
                        json!({"id": id, "offset": text.len(), "max_bytes": COPY_PART_BYTES});
                    let request =
                        json!({"session_id": block.session, "block_id": pane, "args": args});
                    let part = self.rpc.call(COPIED, request).await?;
                    let more = part["data"].as_str().unwrap_or_default();
                    text.push_str(more);
                    ensure!(text.len() <= MAX_COPY_BYTES, "it is too long");
                    if part["eof"] == true {
                        break text;
                    }
                    ensure!(!more.is_empty(), "the server sent only part of it");
                }
            }
        };
        self.shared.change(|s| s.view.copied = Some((pane, text)));
        Ok(())
    }

    /// Stream exactly the panes of the visible tab.
    fn reconcile(&mut self) {
        let (session, visible) = {
            let mut state = self.shared.lock();
            // Only a session this connection has joined can be streamed.
            let joined = state.view.selected_session == self.attached;
            let window = state.view.current_window().filter(|_| joined);
            let visible: HashSet<String> = window
                .into_iter()
                .flat_map(|w| w.panes.iter().map(|p| p.id.clone()))
                .collect();
            state.terminals.retain(|id, _| visible.contains(id));
            (state.view.selected_session.clone(), visible)
        };
        self.streams.retain(|id, task| {
            let keep = visible.contains(id);
            if !keep {
                task.abort();
            }
            keep
        });
        let Some(session) = session else {
            return;
        };
        for pane in visible {
            self.streams.entry(pane).or_insert_with_key(|pane| {
                tokio::spawn(stream_pane(
                    self.target.clone(),
                    self.rpc.clone(),
                    self.shared.clone(),
                    session.clone(),
                    pane.clone(),
                ))
            });
        }
    }

    async fn claim(&self, session: &str, pane: &str) -> Result<()> {
        if self.shared.owns(pane) {
            return Ok(());
        }
        let result = self
            .rpc
            .call(
                "session.own_block",
                json!({"session_id": session, "block_ids": [pane], "mode": "take"}),
            )
            .await?;
        let owner = block(&result["blocks"], pane)["owner"] == self.client_id.as_str();
        self.shared.change(|s| {
            if let Some(pane) = s.view.pane_mut(pane) {
                pane.owner = owner;
            }
        });
        ensure!(owner, "Rex did not grant terminal ownership");
        Ok(())
    }

    /// Do what the UI asked. A change to sessions or tabs completes here. Input is
    /// only queued, in wire order; the caller awaits the returned reply so that
    /// typing never waits on a round trip.
    async fn perform(&mut self, action: Action) -> Result<Option<Response>> {
        let rpc = &self.rpc;
        let mut made = None;
        // The session to show when it is done, if not the one on show already.
        let show = match action {
            Action::CreateSession(name) => {
                let mut payload = json!({"initial_windows": [{"layout": shell()}]});
                if let Some(name) = name {
                    payload["label"] = json!(name);
                }
                let created = rpc.call("session.create", payload).await?;
                let id = created["session_id"].as_str();
                Some(id.context("Missing created session id")?.to_owned())
            }
            Action::SelectSession(session) => Some(session),
            Action::SelectWindow { session, window } => {
                let payload = json!({"session_id": session, "window_id": window});
                rpc.call("session.focus_window", payload).await?;
                Some(session)
            }
            Action::CreateWindow(session) => {
                let payload = json!({"session_id": session, "layout": shell(), "focus": true});
                rpc.call("session.new_window", payload).await?;
                Some(session)
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
                payload["session_id"] = json!(session);
                if let Some(window) = window {
                    payload["window_id"] = json!(window);
                }
                rpc.call(method, payload).await?;
                None
            }
            Action::Split { pane, direction } => {
                let payload = json!({
                    "session_id": self.shown()?,
                    "anchor_id": pane,
                    "direction": match direction {
                        Direction::Right => "horizontal",
                        Direction::Down => "vertical",
                    },
                    "side": "after",
                    "layout": shell(),
                    "focus": true,
                });
                let split = rpc.call("session.new_split", payload).await?;
                made = split["block_ids"][0].as_str().map(str::to_owned);
                None
            }
            Action::Zoom { pane } => {
                let session = self.shown()?;
                // Only an owner may resize it to the room it is given.
                self.claim(&session, &pane).await?;
                let payload = json!({"session_id": session, "block_id": pane});
                rpc.call("session.zoom_block", payload).await?;
                None
            }
            Action::ClosePane { pane } => {
                let payload = json!({"session_id": self.shown()?, "block_id": pane});
                rpc.call("block.close", payload).await?;
                None
            }
            Action::Reveal { pane } => {
                // Focus ends a zoom that hides the pane, and brings back a floating
                // layer that was put away.
                let payload = json!({"session_id": self.shown()?, "block_id": pane});
                rpc.call("session.focus_block", payload).await?;
                made = Some(pane);
                None
            }
            Action::Leave => None,
            input => return self.input(input).await,
        };
        self.sessions(show.as_deref()).await?;
        // Said once the pane is there to be seen.
        if made.is_some() {
            self.shared.change(|s| s.view.made = made);
        }
        Ok(None)
    }

    fn shown(&self) -> Result<String> {
        let selected = self.shared.lock().view.selected_session.clone();
        selected.context("Select or create a session first")
    }

    /// Queue input for a terminal of the session on show.
    async fn input(&self, action: Action) -> Result<Option<Response>> {
        let (rpc, shared, session) = (&self.rpc, self.shared, self.shown()?);
        let key_request =
            |pane: &str, key: Value| json!({"session_id": session, "block_id": pane, "key": key});
        let event_request = |pane: &str, kind: &str, payload: Value| {
            let event = json!({"type": kind, "payload": payload});
            json!({"session_id": session, "block_id": pane, "event": event})
        };
        // Input that engages the terminal takes ownership and returns to live output.
        let (pane, method, payload, engage) = match action {
            Action::FocusPane { pane } => {
                self.claim(&session, &pane).await?;
                let payload = json!({"session_id": session, "block_id": pane});
                rpc.call("session.focus_block", payload).await?;
                return Ok(None);
            }
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
                let payload = json!({"session_id": session, "block_id": pane, "args": args});
                let method = "com.superlogical.terminal.resize";
                return Ok(Some(rpc.enqueue(method, payload).await?));
            }
            Action::Key {
                pane,
                key,
                code,
                modifiers,
                pressed,
                repeat,
            } => {
                let stroke = match (pressed, repeat) {
                    (false, _) => "release",
                    (true, false) => "press",
                    (true, true) => "repeat",
                };
                let payload = key_request(&pane, key_payload(&key, &code, modifiers, stroke));
                // A modifier held alone asks nothing of the terminal.
                let engage = pressed && !model::is_modifier(&code);
                (pane, "client.key", payload, engage)
            }
            Action::Text { pane, text } => {
                let payload = key_request(&pane, key_payload(&text, "unidentified", 0, "press"));
                (pane, "client.key", payload, true)
            }
            Action::Paste { pane, text } => {
                let payload = event_request(&pane, "paste", json!({"text": text}));
                (pane, "block.event", payload, true)
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
                let payload = event_request(&pane, "mouse", mouse);
                (pane, "block.event", payload, kind == "press")
            }
            Action::Wheel { pane, x, y, lines } => {
                let wheel = json!({"x": x, "y": y, "delta_x": 0, "delta_y": lines});
                let payload = event_request(&pane, "wheel", wheel);
                (pane, "block.event", payload, false)
            }
            // Done by `perform`, or by the connection loop.
            _ => return Ok(None),
        };
        let ready = shared
            .terminal(&pane)
            .filter(|terminal| terminal.latest().is_some() && terminal.status().is_none());
        let Some(terminal) = ready else {
            // A release or a motion may trail a pane that has left the screen.
            // Only input that would have done something is worth a complaint.
            ensure!(!engage, "Terminal is not ready; input was not sent");
            return Ok(None);
        };
        if engage {
            self.claim(&session, &pane).await?;
            terminal.scroll_to_bottom();
        }
        Ok(Some(rpc.enqueue(method, payload).await?))
    }
}

/// Keep one pane's terminal fed for as long as it is visible, restoring a fresh
/// snapshot whenever its stream fails.
async fn stream_pane(
    target: wire::Target,
    rpc: Rpc,
    shared: Shared,
    session: String,
    pane: String,
) {
    loop {
        let Err(error) = stream(&target, &rpc, &shared, &session, &pane).await;
        shared.change(|s| {
            let previous = s.terminals.remove(&pane).and_then(|t| t.latest());
            if let Some(pane) = s.view.pane_mut(&pane) {
                pane.frame = previous;
                pane.status = format!("Reconnecting terminal: {error:#}");
            }
        });
        tokio::time::sleep(RETRY_DELAY).await;
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
    let wake = shared.wake.clone();
    let terminal = TerminalHandle::new(nonce, Arc::new(move || wake.notify()))?;
    shared.change(|s| {
        s.terminals.insert(pane.into(), terminal.clone());
    });
    loop {
        terminal.feed(wire::next_binary(&mut socket).await?).await?;
    }
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn label(value: &Value, fallback: &str) -> String {
    let label = value.as_str().filter(|s| !s.is_empty());
    label.unwrap_or(fallback).into()
}

/// The entry for one block in a server block list, or null.
fn block<'a>(blocks: &'a Value, id: &str) -> &'a Value {
    items(blocks).find(|b| b["block_id"] == id).unwrap_or(&NULL)
}

fn parse_windows(view: &Value, client_id: &str) -> Vec<Window> {
    items(&view["windows"])
        .filter_map(|window| {
            let id: String = window["window_id"].as_str()?.into();
            let zoom = window["zoomed_block_id"].as_str();
            let (mut panes, mut hidden) = (Vec::new(), Vec::new());
            for layer in items(&window["layers"]) {
                let mut geometry = Vec::new();
                let bounds = rect(&layer["bounds"]).unwrap_or(FULL);
                layout(&layer["layout"], bounds, 0, &mut geometry);
                for (id, bounds) in geometry {
                    let block = block(&layer["blocks"], &id);
                    let creator = block["creator_name"].as_str();
                    if creator.is_some_and(|creator| creator != "com.superlogical.terminal") {
                        continue;
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
                        title: label(&block["label"], "Terminal"),
                        program: Program::default(),
                        activity: Activity::Idle,
                        message: String::new(),
                        x,
                        y,
                        width,
                        height,
                        frame: None,
                        status: "Connecting terminal…".into(),
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
            })
        })
        .collect()
}

/// What a terminal's programs say they are doing, of all that each has reported:
/// the most pressing of it, the line said of that, and the icon of one still at it.
fn reported(records: &Value) -> (Activity, String, Option<Program>) {
    let activity = |record: &Value| match record["state"].as_str() {
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
    };
    // Of several as pressing, the first: a program comes before its parts.
    let records = || items(records).map(|record| (activity(record), record));
    let top = records().min_by_key(|(state, _)| Reverse(*state));
    let (top, message) = match top {
        Some((top, record)) => (top, record["msg"].as_str().unwrap_or_default()),
        None => (Activity::Idle, ""),
    };
    // What a program that has finished left behind says nothing of what runs now.
    let live = records().filter(|(state, _)| !matches!(state, Activity::Done | Activity::Error));
    let app = Program::named(live.filter_map(|(_, record)| record["effective_app"].as_str()));
    (top, message.to_owned(), app)
}

/// Give every pane what is known of its terminal.
fn fill(sessions: &mut [Session], blocks: &HashMap<String, Block>) {
    let windows = sessions.iter_mut().flat_map(|s| &mut s.windows);
    for pane in windows.flat_map(Window::every_mut) {
        if let Some(block) = blocks.get(&pane.id) {
            // One with no title keeps its label.
            if !block.title.is_empty() {
                pane.title.clone_from(&block.title);
            }
            pane.program = block.app.or(block.process).unwrap_or_default();
            pane.activity = block.activity;
            pane.message.clone_from(&block.message);
            pane.idle = block.shell.is_some() && block.shell == block.foreground;
        }
    }
}

fn rect(value: &Value) -> Option<[f32; 4]> {
    let out: [f32; 4] = serde_json::from_value(value.clone()).ok()?;
    (out.iter().all(|x| x.is_finite()) && out[2] > 0. && out[3] > 0.).then_some(out)
}

fn layout(node: &Value, bounds: [f32; 4], depth: usize, out: &mut Vec<(String, [f32; 4])>) {
    if depth > 64 || out.len() >= 256 {
        return;
    }
    if let Some(id) = node["block_id"].as_str() {
        out.push((id.into(), bounds));
        return;
    }
    let split = &node["split"];
    if !split.is_object() {
        return;
    }
    let ratio = split["ratio"]
        .as_f64()
        .filter(|r| r.is_finite())
        .unwrap_or(0.5)
        .clamp(0., 1.) as f32;
    let [x, y, w, h] = bounds;
    let (before, after) = if split["direction"] == "vertical" {
        (
            [x, y, w, h * ratio],
            [x, y + h * ratio, w, h * (1. - ratio)],
        )
    } else {
        (
            [x, y, w * ratio, h],
            [x + w * ratio, y, w * (1. - ratio), h],
        )
    };
    layout(&split["before"], before, depth + 1, out);
    layout(&split["after"], after, depth + 1, out);
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
    let client = Client::spawn(vec![endpoint], false, Arc::new(|| {}), Herald::new().0);
    let started = std::time::Instant::now();
    loop {
        let view = client.snapshot();
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
