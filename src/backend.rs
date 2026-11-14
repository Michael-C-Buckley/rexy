//! A thin Rex client: one I/O runtime, bounded actions, separate terminal CPU workers.
use crate::{
    model::{self, Action, Direction, Pane, Session, View, Window},
    terminal::{Notify, TerminalHandle},
    wire::{self, Control, Response, Rpc},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use std::{
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
const FULL: [f32; 4] = [0., 0., 1., 1.];
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
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    wake: Arc<Wake>,
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
    fn selected(&self) -> Option<String> {
        self.lock().view.selected_session.clone()
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
}

impl Client {
    pub fn spawn(endpoint: String, wake: Notify) -> Self {
        let shared = Shared {
            state: Arc::default(),
            wake: Arc::new(Wake {
                pending: AtomicBool::new(false),
                callback: wake,
            }),
        };
        let (actions, receiver) = mpsc::channel(256);
        let worker = shared.clone();
        let started = std::thread::Builder::new()
            .name("rex-io".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run(endpoint, receiver, worker)),
                    Err(error) => {
                        worker.status(format!("Cannot start network runtime: {error}"), false)
                    }
                }
            });
        if let Err(error) = started {
            shared.status(format!("Cannot start network worker: {error}"), false);
        }
        Self { actions, shared }
    }

    pub fn snapshot(&self) -> View {
        // Reset before reading: an update arriving during the copy schedules another paint.
        self.shared.wake.pending.store(false, Ordering::Release);
        let state = self.shared.lock();
        let mut view = state.view.clone();
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
        if !state.view.connected && !matches!(action, Action::Reconnect) {
            return Err("Disconnected; input was not sent".into());
        }
        let selected = match &action {
            Action::SelectWindow(id) => Some(id.clone()),
            _ => None,
        };
        self.actions
            .try_send(action)
            .map_err(|e| format!("Input not sent: {e}"))?;
        // Show the tab at once; the server's view follows.
        if let Some(id) = selected
            && state.view.windows.iter().any(|w| w.id == id)
        {
            state.view.selected_window = Some(id);
            drop(state);
            self.shared.wake.notify();
        }
        Ok(())
    }

    /// Move the local scrollback viewport; negative is up. Nothing is sent to the server.
    pub fn scroll(&self, pane: &str, lines: i32) -> Result<(), String> {
        let terminal = self.shared.terminal(pane).ok_or("Terminal is not ready")?;
        terminal.scroll(lines).map_err(|e| e.to_string())
    }
}

async fn run(endpoint: String, mut actions: mpsc::Receiver<Action>, shared: Shared) {
    let mut token = None;
    loop {
        shared.status("Connecting…", false);
        let result = run_connected(&endpoint, &mut token, &mut actions, &shared).await;
        if actions.is_closed() {
            return;
        }
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
            s.view.status = match result {
                Ok(()) => "Reconnecting…".into(),
                Err(error) => format!("Disconnected: {error:#} · retrying"),
            };
        });
        while actions.try_recv().is_ok() {} // Commands belong to the failed connection.
        tokio::select! {
            _ = tokio::time::sleep(RETRY_DELAY) => {}
            action = actions.recv() => { if action.is_none() { return; } }
        }
    }
}

async fn run_connected(
    endpoint: &str,
    token: &mut Option<String>,
    actions: &mut mpsc::Receiver<Action>,
    shared: &Shared,
) -> Result<()> {
    let mut control = Control::connect(endpoint, token.as_deref()).await?;
    *token = control.reconnect_token.take();
    let mut connection = Connection {
        endpoint,
        shared,
        rpc: control.rpc.clone(),
        client_id: control.client_id.clone(),
        streams: HashMap::new(),
    };
    connection.refresh().await?;
    shared.status("Connected", true);
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
            changed = control.changes.recv() => {
                ensure!(changed.is_some(), "Rex control connection ended");
                dirty = true;
            }
            _ = refresh.tick(), if dirty => {
                dirty = false;
                connection.refresh().await?;
                connection.reconcile();
            }
            action = actions.recv(), if replies.len() < 64 => {
                let Some(action) = action else { return Ok(()) };
                if matches!(action, Action::Reconnect) {
                    return Ok(());
                }
                match connection.perform(action).await {
                    Ok(Some(reply)) => replies.push(reply),
                    Ok(None) => {}
                    Err(error) => shared.status(format!("Action failed: {error:#}"), true),
                }
                connection.reconcile();
            }
            reply = replies.next(), if !replies.is_empty() => match reply {
                Some(Ok(Err(error))) => shared.status(format!("Input failed: {error}"), true),
                Some(Err(_)) => bail!("Control connection ended; input was not retried"),
                Some(Ok(Ok(_))) | None => {}
            }
        }
    }
}

/// The state of one control connection. Dropping it stops every terminal stream.
struct Connection<'a> {
    endpoint: &'a str,
    shared: &'a Shared,
    rpc: Rpc,
    client_id: String,
    /// One stream per pane of the visible tab.
    streams: HashMap<String, JoinHandle<()>>,
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        for task in self.streams.values() {
            task.abort();
        }
    }
}

impl Connection<'_> {
    async fn refresh(&self) -> Result<()> {
        self.list_sessions().await?;
        if let Some(session) = self.shared.selected() {
            // Attach is idempotent and keeps newly selected sessions joined after deletion/reconnect.
            self.attach(&session).await?;
        }
        Ok(())
    }

    async fn list_sessions(&self) -> Result<()> {
        let result = self
            .rpc
            .call("session.attach_all", json!({"event_scope": "session"}))
            .await?;
        let sessions: Vec<_> = items(&result["sessions"])
            .filter_map(|s| {
                Some(Session {
                    id: s["session_id"].as_str()?.into(),
                    name: label(&s["label"], "Untitled"),
                })
            })
            .collect();
        self.shared.change(|s| {
            let selected = s.view.selected_session.as_ref();
            if !sessions.iter().any(|x| Some(&x.id) == selected) {
                s.view.selected_session = sessions.first().map(|x| x.id.clone());
                s.view.selected_window = None;
                s.view.windows.clear();
            }
            s.view.sessions = sessions;
        });
        Ok(())
    }

    /// Join a session and adopt the server's view of it.
    async fn attach(&self, session: &str) -> Result<()> {
        let reply = self
            .rpc
            .call("session.attach", json!({"session_id": session}))
            .await?;
        let view = &reply["view"];
        let windows = parse_windows(view, &self.client_id);
        let selected = view["active_window_id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| windows.first().map(|w| w.id.clone()));
        self.shared.change(|s| {
            s.view.selected_session = Some(session.into());
            s.view.selected_window = selected;
            s.view.windows = windows;
        });
        Ok(())
    }

    /// Stream exactly the panes of the visible tab.
    fn reconcile(&mut self) {
        let (session, visible) = {
            let mut state = self.shared.lock();
            let window = state.view.current_window();
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
                    self.endpoint.to_owned(),
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

    /// Structural actions complete here. Input is only queued in wire order; the
    /// caller awaits the returned reply so that typing never waits on a round trip.
    async fn perform(&mut self, action: Action) -> Result<Option<Response>> {
        let (rpc, shared) = (&self.rpc, self.shared);
        let session = match &action {
            Action::CreateSession { name } => {
                let payload = json!({
                    "label": name,
                    "initial_windows": [{"window_label": "Shell", "layout": shell()}],
                });
                let created = rpc.call("session.create", payload).await?;
                let id = created["session_id"]
                    .as_str()
                    .context("Missing created session id")?;
                self.list_sessions().await?;
                self.attach(id).await?;
                return Ok(None);
            }
            Action::SelectSession(id) => {
                self.attach(id).await?;
                return Ok(None);
            }
            _ => shared
                .selected()
                .context("Select or create a session first")?,
        };
        let key_request =
            |pane: &str, key: Value| json!({"session_id": session, "block_id": pane, "key": key});
        let event_request = |pane: &str, kind: &str, payload: Value| {
            let event = json!({"type": kind, "payload": payload});
            json!({"session_id": session, "block_id": pane, "event": event})
        };
        // Input that engages the terminal takes ownership and returns to live output.
        let (pane, method, payload, engage) = match action {
            Action::SelectWindow(id) => {
                let payload = json!({"session_id": session, "window_id": id});
                rpc.call("session.focus_window", payload).await?;
                self.attach(&session).await?;
                return Ok(None);
            }
            Action::CreateWindow => {
                let payload = json!({
                    "session_id": session,
                    "window_label": "Shell",
                    "layout": shell(),
                    "focus": true,
                });
                rpc.call("session.new_window", payload).await?;
                self.attach(&session).await?;
                return Ok(None);
            }
            Action::Split { pane, direction } => {
                let payload = json!({
                    "session_id": session,
                    "anchor_id": pane,
                    "direction": match direction {
                        Direction::Right => "horizontal",
                        Direction::Down => "vertical",
                    },
                    "side": "after",
                    "layout": shell(),
                    "focus": true,
                });
                rpc.call("session.new_split", payload).await?;
                self.attach(&session).await?;
                return Ok(None);
            }
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
                // Only an owner may resize. The UI reports the size again
                // whenever ownership returns.
                if !shared.owns(&pane) {
                    return Ok(None);
                }
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
            } => {
                let payload = key_request(&pane, key_payload(&key, &code, modifiers, pressed));
                (pane, "client.key", payload, pressed)
            }
            Action::Text { pane, text } => {
                let payload = key_request(&pane, key_payload(&text, "unidentified", 0, true));
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
            // Handled above, or by the connection loop.
            Action::CreateSession { .. } | Action::SelectSession(_) | Action::Reconnect => {
                return Ok(None);
            }
        };
        let terminal = shared
            .terminal(&pane)
            .filter(|terminal| terminal.latest().is_some() && terminal.status().is_none())
            .context("Terminal is not ready; input was not sent")?;
        if engage {
            self.claim(&session, &pane).await?;
            terminal.scroll_to_bottom();
        }
        Ok(Some(rpc.enqueue(method, payload).await?))
    }
}

/// Keep one pane's terminal fed for as long as it is visible, restoring a fresh
/// snapshot whenever its stream fails.
async fn stream_pane(endpoint: String, rpc: Rpc, shared: Shared, session: String, pane: String) {
    loop {
        let Err(error) = stream(&endpoint, &rpc, &shared, &session, &pane).await;
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
    endpoint: &str,
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
    let mut socket = wire::socket(endpoint, "/api/data", wire::DATA_PROTOCOL).await?;
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
        .enumerate()
        .filter_map(|(index, window)| {
            let id = window["window_id"].as_str()?.into();
            let zoom = window["zoomed_block_id"].as_str();
            let mut panes = Vec::new();
            for layer in items(&window["layers"]).filter(|l| l["state"] != "minimized") {
                let mut geometry = Vec::new();
                let bounds = rect(&layer["bounds"]).unwrap_or(FULL);
                layout(&layer["layout"], bounds, 0, &mut geometry);
                for (id, bounds) in geometry {
                    if zoom.is_some_and(|zoom| zoom != id) {
                        continue;
                    }
                    let block = block(&layer["blocks"], &id);
                    let creator = block["creator_name"].as_str();
                    if creator.is_some_and(|creator| creator != "com.superlogical.terminal") {
                        continue;
                    }
                    let [x, y, width, height] = if zoom.is_some() { FULL } else { bounds };
                    panes.push(Pane {
                        id,
                        title: label(&block["label"], "Terminal"),
                        x,
                        y,
                        width,
                        height,
                        frame: None,
                        status: "Connecting terminal…".into(),
                        owner: block["owner"] == client_id,
                    });
                }
            }
            Some(Window {
                id,
                name: label(&window["label"], &format!("Tab {}", index + 1)),
                panes,
            })
        })
        .collect()
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

fn key_payload(key: &str, code: &str, bits: u8, pressed: bool) -> Value {
    // Releases and Control/Super chords do not type anything.
    let text = if pressed && bits & (model::CTRL | model::SUPER) == 0 {
        key
    } else {
        ""
    };
    let shifted = bits & model::SHIFT != 0 && !text.is_empty() && text != " ";
    let mut chars = key.chars();
    let codepoint = match (chars.next(), chars.next()) {
        (Some(only), None) => only.to_ascii_lowercase() as u32,
        _ => 0,
    };
    json!({
        "key": code,
        "mods": modifier_names(bits),
        "consumed_mods": if shifted { "shift" } else { "" },
        "text": text,
        "unshifted_codepoint": codepoint,
        "composing": false,
        "action": if pressed { "press" } else { "release" },
    })
}

fn shell() -> Value {
    json!({"block": {"flavor": "com.superlogical.terminal.shell"}})
}

/// Read-only connectivity probe used by --probe: no sessions are created and no input is sent.
pub fn probe(endpoint: String, timeout: Duration) -> Result<String> {
    let client = Client::spawn(endpoint, Arc::new(|| {}));
    let started = std::time::Instant::now();
    loop {
        let view = client.snapshot();
        let frames = view.panes().filter(|p| p.frame.is_some()).count();
        if view.connected && (view.windows.is_empty() || frames > 0) {
            return Ok(format!(
                "{}; {} sessions, {} tabs, {} decoded terminal frames",
                view.status,
                view.sessions.len(),
                view.windows.len(),
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
