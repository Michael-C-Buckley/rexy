//! A deliberately small desktop shell: the sessions and their tabs, in a sidebar or
//! along the top, and the terminal. Terminal content never becomes an egui widget tree.
mod input;
mod muted;
mod raise;
mod render;
mod style;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Align2, Color32, DragAndDrop, FontId, Key, Layout, Margin, Modifiers,
    PointerButton, Pos2, Rect, RichText, Sense, Stroke, UiBuilder, Vec2, text::LayoutJob,
};

use crate::{
    backend::Client,
    link,
    model::{self, Action, Activity, Direction, Edit, Host, Pane, Session, View, Window},
    notify::{Desktop, Herald},
    terminal::Frame,
};
use input::Keyboard;
use muted::Muted;
use raise::Raise;
use render::{Grid, PaneView, Selection, rgb};
use style::{
    ACCENT, AMBER, BODY, BORDER, DANGER, DIM, GREEN, HOVER, Icon, Item, POPOVER, RED, SIDEBAR,
    SMALL, SMALL_BUTTON, TEXT, TOOLBAR, VIEW, bold, confirm, fitted, icon_button, item,
    small_button, suggested,
};

const DEFAULT_FONT_SIZE: f32 = 15.0;
const SIDEBAR_WIDTH: f32 = 240.0;
/// The least and the most its edge can be dragged to.
const SIDEBAR_RANGE: [f32; 2] = [180.0, 480.0];
const PANE_HEADER: f32 = 28.0;
/// What a button says that has been pressed to close something busy, and asks to
/// be pressed again.
const CLOSE: &str = "Close?";
/// The ways to split a pane, and the key that with Ctrl and Shift does each.
const SPLITS: [(Icon, &str, Key, Direction); 2] = [
    (Icon::SplitRight, "Split Right", Key::D, Direction::Right),
    (Icon::SplitDown, "Split Down", Key::E, Direction::Down),
];
/// The arrow keys, and the step each takes from one pane to the next.
const ARROWS: [(Key, [f32; 2]); 4] = [
    (Key::ArrowLeft, [-1.0, 0.0]),
    (Key::ArrowRight, [1.0, 0.0]),
    (Key::ArrowUp, [0.0, -1.0]),
    (Key::ArrowDown, [0.0, 1.0]),
];
/// The lines of history one notch of a wheel moves through.
const NOTCH: f32 = 3.0;

/// A move through a pane's history. Negative is up.
enum Scroll {
    Lines(i32),
    Pages(i32),
    /// From one shell prompt to another, where the shell marks them (OSC 133).
    Prompts(i32),
    Bottom,
}

/// What a turn of the wheel over a pane is for.
#[derive(Clone, Copy, PartialEq)]
enum Wheel {
    /// The pane's history, a line at a time.
    History,
    /// A program that has the whole screen, where there is no history, and
    /// would have the wheel as arrow keys, one for a line (DEC mode 1007).
    Arrows,
    /// A program that tracks the mouse, which is told of each notch.
    Program,
    /// A program that has the whole screen and has not asked for arrow keys.
    Nothing,
}

impl Wheel {
    /// `tracking` says the program in the pane has the mouse, and Shift is not
    /// held to take it back.
    fn of(frame: &Frame, tracking: bool) -> Self {
        match (tracking, frame.alternate_screen, frame.alternate_scroll) {
            (true, ..) => Self::Program,
            (false, false, _) => Self::History,
            (false, true, true) => Self::Arrows,
            (false, true, false) => Self::Nothing,
        }
    }

    /// How far these events turned the wheel, in the steps this is taken by.
    /// Negative is up. What is left of a step stays in `rest`, in lines, for the
    /// next turn.
    fn steps(self, events: &[egui::Event], grid: Grid, rest: &mut f32) -> i32 {
        let lines = |event: &egui::Event| match event {
            egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                // A touchpad says how far it went.
                egui::MouseWheelUnit::Point => delta.y / grid.cell.y,
                // A wheel says how many notches, and under X11 so does a touchpad.
                egui::MouseWheelUnit::Line => delta.y * NOTCH,
                egui::MouseWheelUnit::Page => delta.y * grid.rect.height() / grid.cell.y,
            }),
            _ => None,
        };
        *rest += events.iter().filter_map(lines).sum::<f32>();
        let step = if self == Self::Program { NOTCH } else { 1.0 };
        let steps = (*rest / step).trunc();
        *rest -= steps * step;
        -steps as i32
    }
}

/// A server and the connection kept to it.
struct Server {
    host: Host,
    client: Client,
}

impl Server {
    /// Unless `away`, the server's first session is shown without being asked for.
    fn connect(ctx: &egui::Context, host: Host, away: bool, herald: &Herald) -> Self {
        let ctx = ctx.clone();
        let wake = Arc::new(move || ctx.request_repaint());
        let herald = herald.of(&host.label);
        Self {
            client: Client::spawn(host.endpoints.clone(), away, wake, herald),
            host,
        }
    }

    /// Home, the server this window was opened on, has no label of its own.
    fn home(ctx: &egui::Context, endpoint: &str, herald: &Herald) -> Self {
        let host = Host {
            label: String::new(),
            endpoints: vec![endpoint.to_owned()],
        };
        Self::connect(ctx, host, false, herald)
    }

    fn label(&self) -> &str {
        match self.host.label.as_str() {
            "" => "This Computer",
            label => label,
        }
    }
}

/// A session or, with `window`, one of its tabs.
#[derive(Clone, PartialEq)]
struct Target {
    server: usize,
    session: String,
    window: Option<String>,
}

impl Target {
    /// The action that shows it.
    fn select(&self) -> Action {
        let session = self.session.clone();
        match self.window.clone() {
            Some(window) => Action::SelectWindow { session, window },
            None => Action::SelectSession(session),
        }
    }

    /// The action that changes it.
    fn edit(self, edit: Edit) -> Action {
        Action::Edit {
            session: self.session,
            window: self.window,
            edit,
        }
    }
}

/// A name being typed over a session's or a tab's.
struct Rename {
    target: Target,
    text: String,
    /// The name the field opened with. Left as it was, it is not given anew: that
    /// would name a tab for good by what was running in it just then.
    was: String,
    /// The field has yet to be shown.
    fresh: bool,
    /// The field was shown this frame. A row that has gone takes its renaming with it.
    shown: bool,
}

/// What a pane's programs have had to say, and what the user has made of it.
#[derive(Default)]
struct Heard {
    said: Activity,
    /// With that said, the pane has been on show in this window, with the window in use.
    seen: bool,
    /// Its bell has rung since then.
    rang: bool,
    /// It was on show so when last the window drew.
    watched: bool,
}

/// What was asked to close while it may have had programs running, if anything,
/// and so waits to be asked again: once is easily a slip.
#[derive(Default)]
struct Closing(Option<Asked>);

struct Asked {
    /// The row of a session or a tab, or a pane, by name.
    what: egui::Id,
    /// It was asked with the pointer, and waits only while the pointer is by it.
    near: bool,
    /// The pointer was by it when last the window drew.
    kept: bool,
    until: Instant,
}

/// What pressing something that closes will do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    /// Close, there being nothing to lose.
    No,
    /// Wait to be pressed again.
    First,
    /// Close, having been pressed before.
    Again,
}

impl Closing {
    const TIME: Duration = Duration::from_secs(4);

    /// What pressing something that closes `what` will do. `spared` says that
    /// nothing would be lost by closing it.
    fn ask(&self, what: egui::Id, spared: bool) -> Ask {
        match &self.0 {
            Some(asked) if asked.what == what => Ask::Again,
            _ if spared => Ask::No,
            _ => Ask::First,
        }
    }

    /// Take a press of something that closes `what`, and say whether to close it.
    /// `near` says the press was the pointer's.
    fn press(&mut self, what: egui::Id, spared: bool, near: bool, now: Instant) -> bool {
        let first = self.ask(what, spared) == Ask::First;
        self.0 = first.then_some(Asked {
            what,
            near,
            kept: true,
            until: now + Self::TIME,
        });
        !first
    }

    /// The pointer is still by what waits.
    fn keep(&mut self) {
        if let Some(asked) = &mut self.0 {
            asked.kept = true;
        }
    }

    /// Stop waiting after a while, or once the pointer has left what it asked.
    /// Returns how long the wait still has to run: no time at all if it has just
    /// ended, and what asked has yet to be drawn as it was before.
    fn lapse(&mut self, now: Instant) -> Option<Duration> {
        let asked = self.0.as_mut()?;
        let left = asked.until.saturating_duration_since(now);
        let left_alone = !std::mem::take(&mut asked.kept) && asked.near;
        if left_alone || left.is_zero() {
            self.0 = None;
            return Some(Duration::ZERO);
        }
        Some(left)
    }
}

/// The name a pane goes by in being closed.
fn doomed(pane: &str) -> egui::Id {
    egui::Id::unique(("close", pane))
}

/// What was asked of a session or a tab through its menu.
enum Choice {
    Rename,
    /// Or the reverse, of a tab that is muted.
    Mute,
    Edit(Edit),
}

pub struct App {
    /// Home first, then the hosts it names.
    servers: Vec<Server>,
    /// The server whose session fills the window.
    current: usize,
    /// Home's address, as typed.
    endpoint: String,
    /// With the sidebar, tabs are listed in it. Without, they run along the top.
    sidebar: bool,
    sidebar_width: f32,
    /// The pane that has the keyboard, while one is on screen.
    focused: Option<String>,
    panes: HashMap<String, PaneView>,
    keyboard: Keyboard,
    font_size: f32,
    toast: Toast,
    rename: Option<Rename>,
    muted: Muted,
    /// By pane.
    heard: HashMap<String, Heard>,
    /// Where the servers' connections send what their panes have to say, and what
    /// passes it on to the desktop.
    herald: Herald,
    desktop: Desktop,
    /// What is typed in the list of sessions, and which of its rows Enter would take.
    filter: String,
    cursor: usize,
    closing: Closing,
    /// The tab on show that its list was last scrolled to, and in which list. It
    /// is scrolled to once, and the list is the user's to scroll after that.
    followed: Option<egui::Id>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, endpoint: String) -> Self {
        style::apply(&cc.egui_ctx);
        let (herald, notes) = Herald::new();
        let (ctx, raise) = (cc.egui_ctx.clone(), Raise::of(cc));
        // A picked note brings the window forward from the desktop's thread, at once:
        // a window out of sight is slow to be told anything.
        let desktop = Desktop::start(notes, move |token| {
            if let (Some(raise), Some(token)) = (raise, token) {
                raise.with(token);
            }
            ctx.request_repaint();
        });
        let muted = Muted::load();
        desktop.mute(muted.tabs().clone());
        Self {
            servers: vec![Server::home(&cc.egui_ctx, &endpoint, &herald)],
            current: 0,
            endpoint,
            sidebar: true,
            sidebar_width: SIDEBAR_WIDTH,
            focused: None,
            panes: HashMap::new(),
            keyboard: Keyboard::default(),
            font_size: DEFAULT_FONT_SIZE,
            toast: Toast::default(),
            rename: None,
            muted,
            heard: HashMap::new(),
            herald,
            desktop,
            filter: String::new(),
            cursor: 0,
            closing: Closing::default(),
            followed: None,
        }
    }

    /// The name a session's or a tab's row goes by, which stays its own whatever
    /// rows come and go around it.
    fn row(&self, target: &Target) -> egui::Id {
        let host = &self.servers[target.server].host.label;
        egui::Id::unique(("row", host, &target.session, &target.window))
    }

    /// Close something, by the action that does it, unless programs may be lost by
    /// that and this is the first asking. `near` says the pointer did the asking.
    fn close(&mut self, what: egui::Id, spared: bool, near: bool, server: usize, action: Action) {
        if self.closing.press(what, spared, near, Instant::now()) {
            self.tell(server, action);
        }
    }

    fn tell(&mut self, server: usize, action: Action) {
        if let Err(error) = self.servers[server].client.try_send(action) {
            self.toast.say(error);
        }
    }

    fn send(&mut self, action: Action) {
        self.tell(self.current, action);
    }

    /// Show a session or a tab, on whichever server it lives.
    fn open(&mut self, server: usize, action: Action) {
        if server != self.current {
            // The session left behind keeps running; this window just stops watching it.
            self.servers[self.current].client.leave();
            self.current = server;
        }
        self.send(action);
    }

    /// Something other than a terminal is taking what is typed.
    fn busy(&self, ctx: &egui::Context) -> bool {
        self.rename.is_some() || egui::Popup::is_any_open(ctx)
    }

    /// Home names the other servers. Keep a connection to each, and to no others.
    fn follow_hosts(&mut self, ctx: &egui::Context, home: &View) {
        let listed = self.servers[1..].iter().map(|server| &server.host);
        // Only a connected home knows.
        if !home.connected || listed.eq(&home.hosts) {
            return;
        }
        let current = self.servers[self.current].host.clone();
        let mut known = self.servers.split_off(1);
        for host in &home.hosts {
            let server = match known.iter().position(|server| &server.host == host) {
                Some(index) => known.swap_remove(index),
                None => Server::connect(ctx, host.clone(), true, &self.herald),
            };
            self.servers.push(server);
        }
        let current = self.servers.iter().position(|s| s.host == current);
        self.current = current.unwrap_or(0);
    }

    /// Take in what every pane has to say, and that the panes on show have been
    /// seen to say it, unless the window is not the one in use. Each pane is left
    /// saying only what is still news of it: nothing in a muted tab, a bell that
    /// rang unseen, and what its program ended with until that has been looked at.
    fn hear(&mut self, ctx: &egui::Context, views: &mut [View]) {
        let watching = ctx.input(|i| i.focused);
        // Something was said for the desktop, which is told what need not be passed on.
        let news = self.herald.news();
        let mut open = 0;
        for (server, view) in views.iter_mut().enumerate() {
            let shown = view.current_window().map(|tab| tab.id.clone());
            let shown = shown.filter(|_| watching && server == self.current);
            let tabs = view.sessions.iter_mut().flat_map(|s| &mut s.windows);
            for tab in tabs {
                let shown = shown.as_ref() == Some(&tab.id);
                let muted = self.muted.has(&tab.id);
                // Those out of sight are not seen, whatever tab is on show.
                let panes = tab.panes.iter_mut().map(|pane| (pane, shown));
                let hidden = tab.hidden.iter_mut().map(|pane| (pane, false));
                for (pane, seen) in panes.chain(hidden) {
                    open += 1;
                    let heard = match self.heard.get_mut(&pane.id) {
                        Some(heard) => heard,
                        None => self.heard.entry(pane.id.clone()).or_default(),
                    };
                    if heard.said != pane.activity {
                        heard.said = pane.activity;
                        heard.seen = false;
                    }
                    heard.rang |= view.rang.contains(&pane.id);
                    if seen {
                        heard.seen = true;
                        heard.rang = false;
                        if news || !heard.watched {
                            self.desktop.watched(&pane.id);
                        }
                    }
                    heard.watched = seen;
                    let said = match pane.activity {
                        _ if muted => Activity::Idle,
                        Activity::Done | Activity::Error if heard.seen => Activity::Idle,
                        said => said,
                    };
                    let said = match heard.rang && !muted {
                        true => said.max(Activity::Rang),
                        false => said,
                    };
                    if said != pane.activity {
                        pane.activity = said;
                        pane.message.clear();
                    }
                }
            }
        }
        // A pane has closed.
        if self.heard.len() != open {
            let open = |pane: &str| views.iter().any(|view| view.find(pane).is_some());
            self.heard.retain(|pane, _| open(pane));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, view: &View) {
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let pressed = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        // With these modifiers and no others, which `pressed` lets by.
        let exactly = |modifiers: Modifiers, key| {
            let held = ctx.input(|i| i.modifiers.matches_exact(modifiers));
            held && pressed(modifiers, key)
        };
        let paste = || ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        if pressed(chord, Key::B) {
            self.sidebar = !self.sidebar;
        }
        // A terminal's programs own Ctrl+C, X and V. In a text field they mean
        // what they usually do.
        if ctx.text_edit_focused() {
            for (key, event) in [(Key::C, egui::Event::Copy), (Key::X, egui::Event::Cut)] {
                if pressed(Modifiers::CTRL, key) {
                    ctx.input_mut(|i| i.events.push(event));
                }
            }
            if pressed(Modifiers::CTRL, Key::V) {
                paste();
            }
        }
        if self.busy(ctx) {
            return;
        }
        if pressed(chord, Key::N) && view.connected {
            self.send(Action::CreateSession(None));
        }
        if pressed(chord, Key::T)
            && view.connected
            && let Some(session) = &view.selected_session
        {
            self.send(Action::CreateWindow(session.clone()));
        }
        if (pressed(chord, Key::C) || pressed(Modifiers::NONE, Key::Copy))
            && let Some(pane) = self.focused.as_ref().and_then(|id| self.panes.get(id))
            && let Some(selection) = &pane.selection
        {
            ctx.copy_text(selection.text());
        }
        if pressed(chord, Key::V) || pressed(Modifiers::NONE, Key::Paste) {
            paste();
        }
        // The tabs of the session on show, round and round. A chord is taken with
        // Shift held too, so those that need it are asked after first.
        let back = pressed(chord, Key::Tab) || pressed(Modifiers::CTRL, Key::PageUp);
        let on = pressed(Modifiers::CTRL, Key::Tab) || pressed(Modifiers::CTRL, Key::PageDown);
        let (tabs, shown) = (view.windows(), view.selected_window.as_ref());
        if on != back
            && let Some(session) = &view.selected_session
            && let Some(at) = tabs.iter().position(|tab| Some(&tab.id) == shown)
        {
            let to = (at + if on { 1 } else { tabs.len() - 1 }) % tabs.len();
            let (session, window) = (session.clone(), tabs[to].id.clone());
            self.send(Action::SelectWindow { session, window });
        }
        if let Some(window) = view.current_window()
            && let Some(pane) = self.focused.clone()
        {
            for (key, toward) in ARROWS {
                if pressed(chord, key)
                    && let Some(next) = window.beside(&pane, toward)
                {
                    let pane = next.id.clone();
                    self.focused = Some(pane.clone());
                    self.send(Action::FocusPane { pane });
                }
            }
            for (.., key, direction) in SPLITS {
                if pressed(chord, key) {
                    let pane = pane.clone();
                    self.send(Action::Split { pane, direction });
                }
            }
            if pressed(chord, Key::Enter) && window.split() {
                self.send(Action::Zoom { pane: pane.clone() });
            }
            // The keys that move through its history, which its program never
            // sees: unless that has the whole screen, which keeps no history, and
            // so has the keys.
            let shift = Modifiers::SHIFT;
            let scrolls = [
                (shift, Key::PageUp, Scroll::Pages(-1)),
                (shift, Key::PageDown, Scroll::Pages(1)),
                // Further than any history goes.
                (shift, Key::Home, Scroll::Lines(i32::MIN)),
                (shift, Key::End, Scroll::Bottom),
                (chord, Key::Z, Scroll::Prompts(-1)),
                (chord, Key::X, Scroll::Prompts(1)),
            ];
            let frame = view.pane(&pane).and_then(|pane| pane.frame.as_ref());
            if !frame.is_some_and(|frame| frame.alternate_screen) {
                for (modifiers, key, to) in scrolls {
                    if exactly(modifiers, key) {
                        self.scroll(&pane, to);
                    }
                }
            }
            // A key held down asks once: its repeats would answer for the user.
            // They are looked for first, since `pressed` takes the key's events away.
            let repeat = |event: &egui::Event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: Key::W,
                        repeat: true,
                        ..
                    }
                )
            };
            let held = ctx.input(|i| i.events.iter().any(repeat));
            if pressed(chord, Key::W) && !held {
                let spared = view.pane(&pane).is_some_and(|pane| pane.idle);
                let action = Action::ClosePane { pane: pane.clone() };
                self.close(doomed(&pane), spared, false, self.current, action);
            }
        }
        // Ctrl with Shift or Alt and these keys is for the terminal's programs. On
        // most keyboards a plus is typed with Shift, though.
        if exactly(Modifiers::CTRL, Key::Plus)
            || exactly(chord, Key::Plus)
            || exactly(Modifiers::CTRL, Key::Equals)
        {
            self.font_size = (self.font_size + 1.0).min(32.0);
        }
        if exactly(Modifiers::CTRL, Key::Minus) {
            self.font_size = (self.font_size - 1.0).max(9.0);
        }
        if exactly(Modifiers::CTRL, Key::Num0) {
            self.font_size = DEFAULT_FONT_SIZE;
        }
    }

    /// The buttons that split the focused pane, laid out from the right.
    fn splits(&mut self, ui: &mut egui::Ui, view: &View) {
        ui.add_enabled_ui(view.connected && self.focused.is_some(), |ui| {
            for (icon, label, key, direction) in SPLITS.into_iter().rev() {
                let tip = format!("{label} (Ctrl+Shift+{})", key.name());
                if icon_button(ui, icon, &tip).clicked()
                    && let Some(pane) = self.focused.clone()
                {
                    self.send(Action::Split { pane, direction });
                }
            }
        });
    }

    /// A button that lists the panes of the tab on show that are out of sight:
    /// behind a zoomed one, or in a floating layer that was put away. Picking one
    /// brings it out. With none, there is no button.
    fn hidden(&mut self, ui: &mut egui::Ui, view: &View) {
        let hidden = view.current_window().map_or(&[][..], |tab| &tab.hidden);
        if hidden.is_empty() {
            return;
        }
        let button = icon_button(ui, Icon::Behind, "Panes out of sight");
        // It wears the most pressing thing any of them has to say.
        let said = hidden.iter().map(|pane| pane.activity).max();
        if let Some((_, color)) = dot(said.unwrap_or_default()) {
            let corner = button.rect.right_top() + egui::vec2(-3.5, 4.5);
            ui.painter().circle_filled(corner, 3.5, color);
        }
        let mut picked = None;
        egui::Popup::menu(&button).width(260.0).show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for pane in hidden {
                let row = Item {
                    icon: Some(pane.program),
                    mark: dot(pane.activity),
                    word: pane.activity.word(),
                    label: &pane.title,
                    ..Default::default()
                };
                if item(ui, Vec2::new(ui.available_width(), 32.0), row)
                    .0
                    .clicked()
                {
                    picked = Some(pane.id.clone());
                }
            }
        });
        if let Some(pane) = picked {
            self.send(Action::Reveal { pane });
        }
    }

    /// Every server, each with its sessions beneath it, and under each session its tabs.
    fn sidebar(&mut self, root_ui: &mut egui::Ui, views: &[View]) {
        let rows = egui::Frame::new().inner_margin(Margin {
            top: 0,
            ..Margin::same(6)
        });
        // No wider than leaves the terminal half the window, however it was left.
        let [least, most] = SIDEBAR_RANGE;
        let most = (root_ui.available_width() / 2.0).clamp(least, most);
        self.sidebar_width = self.sidebar_width.clamp(least, most);
        egui::Panel::left("sessions")
            .exact_size(self.sidebar_width)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // What the toolbar holds when the tabs run along the top.
                let mut top = bar(ui, TOOLBAR, 6.0);
                let tip = "Tabs along the top (Ctrl+Shift+B)";
                if icon_button(&mut top, Icon::Sidebar, tip).clicked() {
                    self.sidebar = false;
                }
                top.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.splits(ui, &views[self.current]);
                    self.hidden(ui, &views[self.current]);
                });
                let list = egui::ScrollArea::vertical().auto_shrink(false);
                list.show(ui, |ui| {
                    rows.show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (server, view) in views.iter().enumerate() {
                            if server > 0 {
                                ui.add_space(12.0);
                            }
                            self.server(ui, server, view);
                        }
                    });
                });
            });
    }

    /// The sidebar's edge, which drags to make it wider or narrower. It is asked
    /// for last of all, or the terminal beside it and the scroll bar within it
    /// would have the pointer there first.
    fn edge(&mut self, root_ui: &egui::Ui) {
        let x = root_ui.max_rect().left() + self.sidebar_width;
        let height = root_ui.max_rect().y_range();
        let grab = Rect::from_x_y_ranges(x - 4.0..=x + 4.0, height);
        // It takes clicks too, or a press there would be the terminal's to drag.
        let sense = Sense::click_and_drag();
        let edge = root_ui.interact(grab, egui::Id::unique("sidebar edge"), sense);
        if edge.hovered() || edge.dragged() {
            root_ui
                .ctx()
                .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            root_ui.painter().vline(x, height, Stroke::new(1.0, DIM));
        }
        // The next drawing holds it to its least and its most.
        if let Some(pointer) = edge.interact_pointer_pos().filter(|_| edge.dragged()) {
            self.sidebar_width = pointer.x - root_ui.max_rect().left();
        }
    }

    /// A server in the sidebar: its name, its sessions, and under each its tabs.
    fn server(&mut self, ui: &mut egui::Ui, server: usize, view: &View) {
        if self.heading(ui, server, view) {
            self.open(server, Action::CreateSession(None));
        }
        if !view.connected {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                // On as many as three lines, broken between words. In full under
                // the pointer, if that was not enough.
                let font = FontId::proportional(SMALL);
                let mut status = LayoutJob::simple(view.status.clone(), font, DIM, 0.0);
                status.wrap.max_rows = 3;
                ui.add(egui::Label::new(status).wrap())
                    .on_hover_text(&view.status);
            });
        }
        let mut sessions = Vec::new();
        for (index, session) in view.sessions.iter().enumerate() {
            let top = ui.cursor().top();
            let shown =
                server == self.current && view.session().is_some_and(|s| s.id == session.id);
            self.session(ui, server, &view.sessions, index, shown);
            // The tab on show, if it is one of this session's.
            let shown = view.selected_window.as_deref().filter(|_| shown);
            let size = Vec2::new(ui.available_width(), 32.0);
            self.tabs(ui, server, session, shown, size);
            let space = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
            sessions.push((&session.id, space));
        }
        self.reorder(ui, server, &sessions);
    }

    /// A server's name, beside a button that tries it again if it is not connected.
    /// True if its other button, for a new session, was pressed.
    fn heading(&mut self, ui: &mut egui::Ui, server: usize, view: &View) -> bool {
        let mut create = false;
        let mut heading = bar(ui, 32.0, 0.0);
        heading.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if view.connected {
                create = icon_button(ui, Icon::Plus, "New session").clicked();
            } else if icon_button(ui, Icon::Retry, "Try again").clicked() {
                self.tell(server, Action::Reconnect);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(12.0);
                let label = RichText::new(self.servers[server].label());
                ui.add(egui::Label::new(label.font(bold(SMALL))).truncate());
            });
        });
        create
    }

    /// A session's tabs, down the sidebar or along the top. `shown` is the one on show.
    fn tabs(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        session: &Session,
        shown: Option<&str>,
        size: Vec2,
    ) {
        let windows = session.windows.iter().enumerate();
        let tabs: Vec<_> = windows
            .map(|(index, window)| {
                (
                    &window.id,
                    self.tab(ui, server, session, shown, index, size),
                )
            })
            .collect();
        self.reorder(ui, server, &tabs);
    }

    /// While one of these rows is dragged, mark where among them it would land, and
    /// move it there when it is let go. The rows are a server's sessions, or the tabs
    /// of one of them, each with the space it takes.
    fn reorder(&mut self, ui: &egui::Ui, server: usize, rows: &[(&String, Rect)]) {
        let Some(drag) = DragAndDrop::payload::<Target>(ui.ctx()) else {
            return;
        };
        let id = drag.window.as_ref().unwrap_or(&drag.session);
        let from = rows.iter().position(|(row, _)| *row == id);
        let from = from.filter(|_| drag.server == server);
        let (Some(from), Some(pointer)) = (from, ui.input(|i| i.pointer.latest_pos())) else {
            return;
        };
        let across = ui.layout().is_horizontal();
        let along = |pos: Pos2| if across { pos.x } else { pos.y };
        // It lands ahead of the first row whose middle the pointer has yet to pass.
        let passed = |(_, space): &&(&String, Rect)| along(pointer) >= along(space.center());
        let to = rows.iter().take_while(passed).count();
        // Where it is already, there is nowhere to go.
        if to == from || to == from + 1 {
            return;
        }
        let (edge, row) = match rows.get(to) {
            Some((_, space)) => (along(space.min) - 1.0, space),
            None => (along(rows[to - 1].1.max) + 1.0, &rows[to - 1].1),
        };
        let line = Stroke::new(2.0, ACCENT);
        if across {
            ui.painter().vline(edge, row.y_range(), line);
        } else {
            ui.painter().hline(row.x_range(), edge, line);
        }
        if ui.input(|i| i.pointer.any_released()) {
            let before = rows.get(to).map(|(row, _)| (*row).clone());
            self.edit(Target::clone(&drag), Edit::MoveBefore(before));
        }
    }

    /// With the sidebar hidden, one row holds what it held: the sessions, behind a
    /// button, and the tabs of the one on show.
    fn toolbar(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let view = &views[self.current];
        let mut bar = bar(ui, TOOLBAR, 6.0);
        if icon_button(&mut bar, Icon::Sidebar, "Tabs in a sidebar (Ctrl+Shift+B)").clicked() {
            self.sidebar = true;
        }
        self.switcher(&mut bar, views);
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            self.splits(ui, view);
            self.hidden(ui, view);
            ui.add_enabled_ui(view.connected && view.selected_session.is_some(), |ui| {
                if icon_button(ui, Icon::Plus, "New tab (Ctrl+Shift+T)").clicked()
                    && let Some(session) = &view.selected_session
                {
                    self.send(Action::CreateWindow(session.clone()));
                }
            });
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(4.0);
                let Some(session) = view.session() else {
                    return;
                };
                // The wheel has one way to turn them, whichever way it is turned.
                ui.style_mut().always_scroll_the_only_direction = true;
                egui::ScrollArea::horizontal()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let count = session.windows.len().max(1) as f32;
                        let width = (ui.available_width() - 4.0 * (count - 1.0)) / count;
                        let size = Vec2::new(width.clamp(120.0, 220.0), 30.0);
                        let shown = view.selected_window.as_deref();
                        self.tabs(ui, self.current, session, shown, size);
                    });
            });
        });
    }

    /// The session on show, as a button that lists them all: to find one by name,
    /// or to make one with a name of the user's choosing.
    fn switcher(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let current = self.current;
        let name = views[current].session().map_or("Sessions", |s| &s.name);
        // Every session, and whether it is the one on show.
        let sessions = || {
            let servers = views.iter().enumerate();
            servers.flat_map(|(server, view)| {
                let shown = view.selected_session.as_ref();
                let shown = shown.filter(|_| server == current);
                let sessions = view.sessions.iter();
                sessions.map(move |session| (Some(&session.id) == shown, session))
            })
        };
        // The tabs of the session on show speak for themselves, right beside this.
        let elsewhere = sessions().filter(|(shown, _)| !shown);
        let elsewhere = elsewhere.map(|(_, session)| session.activity()).max();
        let font = FontId::proportional(BODY);
        let text = ui.painter().layout_no_wrap(name.to_owned(), font, TEXT);
        let size = Vec2::new((text.size().x + 54.0).min(240.0), 30.0);
        let button = Item {
            menu: true,
            mark: dot(elsewhere.unwrap_or_default()),
            label: name,
            ..Default::default()
        };
        let (button, _) = item(ui, size, button);
        let id = egui::Popup::default_response_id(&button);
        let opened = !egui::Popup::is_id_open(ui.ctx(), id);
        if opened {
            // It opens with nothing typed, at the session on show.
            self.filter.clear();
            self.cursor = sessions().position(|(shown, _)| shown).unwrap_or(0);
        }
        let mut picked = None;
        egui::Popup::menu(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(280.0)
            .show(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // The arrows walk the list while what is typed goes to the field.
                let step = ui.input_mut(|i| {
                    let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                    isize::from(down) - isize::from(i.consume_key(Modifiers::NONE, Key::ArrowUp))
                });
                let field = egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter or create…")
                    .margin(egui::vec2(10.0, 8.0))
                    .desired_width(f32::INFINITY);
                let field = ui.add(field);
                if opened {
                    field.request_focus();
                }
                if field.changed() {
                    self.cursor = 0;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let name = self.filter.trim().to_owned();
                let wanted = name.to_lowercase();
                let found = |session: &&Session| session.name.to_lowercase().contains(&wanted);
                let count = sessions().filter(|(_, session)| found(session)).count();
                // After the sessions comes the row that makes one, by the name typed.
                self.cursor = self.cursor.saturating_add_signed(step).min(count);
                let create = || Action::CreateSession((!name.is_empty()).then(|| name.clone()));
                let cursor = self.cursor;
                let mut index = 0;
                // A row of the list. True if it was taken, by a click or with Enter.
                let mut row = |ui: &mut egui::Ui, label: &str, mark| {
                    let at = index == cursor;
                    index += 1;
                    let row = Item {
                        selected: at,
                        mark,
                        label,
                        ..Default::default()
                    };
                    let (response, _) = item(ui, Vec2::new(ui.available_width(), 32.0), row);
                    if at && step != 0 {
                        response.scroll_to_me(None);
                    }
                    response.clicked() || (at && enter)
                };
                let list = egui::ScrollArea::vertical().max_height(420.0);
                list.show(ui, |ui| {
                    for (server, view) in views.iter().enumerate() {
                        // A session made on a server takes the name typed, too.
                        if self.heading(ui, server, view) {
                            picked = Some((server, create()));
                        }
                        for session in view.sessions.iter().filter(found) {
                            if row(ui, &session.name, dot(session.activity())) {
                                picked = Some((server, Action::SelectSession(session.id.clone())));
                            }
                        }
                    }
                    ui.add_space(6.0);
                    let label = match name.as_str() {
                        "" => "New Session".to_owned(),
                        name => format!("New Session “{name}”"),
                    };
                    if row(ui, &label, None) {
                        picked = Some((current, create()));
                    }
                });
            });
        if let Some((server, action)) = picked {
            egui::Popup::close_id(ui.ctx(), id);
            self.open(server, action);
        }
    }

    /// A session's name, heading its tabs in the sidebar.
    fn session(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        sessions: &[Session],
        index: usize,
        shown: bool,
    ) {
        let session = &sessions[index];
        let target = Target {
            server,
            session: session.id.clone(),
            window: None,
        };
        let size = Vec2::new(ui.available_width(), 28.0);
        let id = self.row(&target);
        if self.renaming(ui, &target, id, size).is_some() {
            return;
        }
        let spared = session.windows.iter().all(Window::idle);
        let ask = self.closing.ask(id, spared);
        let row = Item {
            id: Some(id),
            caption: true,
            buttons: &[(Icon::Plus, "New tab"), (Icon::Close, "Close session")],
            confirm: (ask == Ask::Again).then_some(CLOSE),
            movable: true,
            label: &session.name,
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        self.picked(&response, &target, shown);
        self.wait(ui, response.rect, &response, ask);
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = ["Move Up", "Move Down", "Close Session"];
        let at = |index: usize| sessions.get(index).map(|s| s.id.clone());
        match (button, menu(&response, labels, None, index, at, ask)) {
            (Some(0), _) => self.open(server, Action::CreateWindow(session.id.clone())),
            // This ends every program in the session.
            (Some(_), _) | (None, Some(Choice::Edit(Edit::Close))) => {
                self.close(id, spared, true, server, target.edit(Edit::Close));
            }
            (None, Some(choice)) => self.choose(target, &session.name, choice),
            (None, None) => {}
        }
    }

    /// Take a click on a session's or a tab's row, if there was one, and show the
    /// row unless it is on show. The second click of two asks for a new name, and
    /// nothing tells a first click from a lone one, so the first has shown the
    /// row by then.
    fn picked(&mut self, response: &egui::Response, target: &Target, shown: bool) {
        if response.clicked() && !shown {
            self.open(target.server, target.select());
        }
    }

    /// Go on waiting to be asked again to close something, while the pointer is
    /// on it, which is `within`, or in the menu `of` it.
    fn wait(&mut self, ui: &egui::Ui, within: Rect, of: &egui::Response, ask: Ask) {
        if ask == Ask::Again && (ui.rect_contains_pointer(within) || of.context_menu_opened()) {
            self.closing.keep();
        }
    }

    /// A tab, in the sidebar or along the top. Returns the space it took.
    fn tab(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        session: &Session,
        shown: Option<&str>,
        index: usize,
        size: Vec2,
    ) -> Rect {
        let across = ui.layout().is_horizontal();
        let window = &session.windows[index];
        let target = Target {
            server,
            session: session.id.clone(),
            window: Some(window.id.clone()),
        };
        let id = self.row(&target);
        if let Some(field) = self.renaming(ui, &target, id, size) {
            return field;
        }
        let muted = self.muted.has(&window.id);
        let (said, message) = window.says();
        let selected = shown == Some(&window.id);
        let spared = window.idle();
        let ask = self.closing.ask(id, spared);
        // What it says in full, for a row with no room for it, and with the line
        // its program adds.
        let tip = model::line(said.word().unwrap_or_default(), message);
        let row = Item {
            id: Some(id),
            icon: Some(window.program()),
            selected,
            align: if across { Align::Center } else { Align::LEFT },
            mark: if muted {
                Some((Icon::Muted, DIM))
            } else {
                dot(said)
            },
            word: said.word(),
            buttons: &[(Icon::Close, "Close tab")],
            confirm: (ask == Ask::Again).then_some(CLOSE),
            tip: &tip,
            movable: true,
            label: window.name(),
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        // The list turns to the tab on show when another comes to be it, by
        // whatever means, and stays where the user turns it otherwise.
        let list = id.with(self.sidebar);
        if selected && self.followed != Some(list) {
            self.followed = Some(list);
            response.scroll_to_me(None);
        }
        self.picked(&response, &target, selected);
        self.wait(ui, response.rect, &response, ask);
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = if across {
            ["Move Left", "Move Right", "Close Tab"]
        } else {
            ["Move Up", "Move Down", "Close Tab"]
        };
        let at = |index: usize| session.windows.get(index).map(|w| w.id.clone());
        match (button, menu(&response, labels, Some(muted), index, at, ask)) {
            (Some(_), _) | (None, Some(Choice::Edit(Edit::Close))) => {
                self.close(id, spared, true, server, target.edit(Edit::Close));
            }
            // The field opens on the name it goes by, given or not.
            (None, Some(choice)) => self.choose(target, window.name(), choice),
            (None, None) => {}
        }
        response.rect
    }

    /// Do what a session's or a tab's menu was asked. `name` is the one it goes by.
    fn choose(&mut self, target: Target, name: &str, choice: Choice) {
        match choice {
            Choice::Rename => {
                self.rename = Some(Rename {
                    target,
                    text: name.to_owned(),
                    was: name.to_owned(),
                    fresh: true,
                    shown: true,
                });
            }
            Choice::Mute => {
                let tab = target.window.as_deref().unwrap_or_default();
                if let Err(error) = self.muted.toggle(tab) {
                    let said = format!("Muted tabs could not be saved: {error}");
                    self.toast.say(said);
                }
                self.desktop.mute(self.muted.tabs().clone());
            }
            Choice::Edit(edit) => self.edit(target, edit),
        }
    }

    fn edit(&mut self, target: Target, edit: Edit) {
        self.tell(target.server, target.edit(edit));
    }

    /// Show the field that stands in for a row while it is renamed, and return the
    /// space it took. None if this row is not the one.
    fn renaming(
        &mut self,
        ui: &mut egui::Ui,
        target: &Target,
        row: egui::Id,
        size: Vec2,
    ) -> Option<Rect> {
        let rename = self.rename.as_mut().filter(|r| &r.target == target)?;
        rename.shown = true;
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        // By the row's own name, so that the field keeps the keyboard when rows
        // before it come and go.
        let field = egui::TextEdit::singleline(&mut rename.text)
            .id(row.with("rename"))
            .hint_text("Name")
            .margin(egui::vec2(11.0, 0.0))
            .align(Align2::LEFT_CENTER)
            .min_size(rect.size());
        let mut output = field.show(&mut ui.new_child(UiBuilder::new().max_rect(rect)));
        if std::mem::take(&mut rename.fresh) {
            // Typing replaces the old name.
            let all = egui::text::CCursorRange::select_all_str(&rename.text);
            output.state.cursor.set_char_range(Some(all));
            output.state.store(ui.ctx(), output.response.id);
            output.response.request_focus();
        } else if output.response.lost_focus() {
            // Escape leaves the name as it was. Anything else, such as Enter or a
            // click elsewhere, takes the new one, if it is new.
            let name = self
                .rename
                .take()
                .filter(|rename| rename.text != rename.was);
            let name = name.map(|rename| rename.text);
            if let Some(name) = name.filter(|_| !ui.input(|i| i.key_pressed(Key::Escape))) {
                self.edit(target.clone(), Edit::Rename(name));
            }
        }
        Some(rect)
    }

    /// What to show when there is no terminal to show.
    fn empty(&mut self, ui: &mut egui::Ui, view: &View) {
        let action = |label| {
            suggested(label)
                .corner_radius(19)
                .min_size(Vec2::new(140.0, 38.0))
        };
        ui.add_space((ui.available_height() * 0.28).max(20.0));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            if view.connected {
                // A session outlives its last tab.
                let (heading, label, new) = match &view.selected_session {
                    Some(session) => ("No Tabs", "New Tab", Action::CreateWindow(session.clone())),
                    None => ("No Terminals", "New Session", Action::CreateSession(None)),
                };
                ui.heading(heading);
                let about = "Programs you start keep running on the server.";
                ui.label(RichText::new(about).color(DIM));
                ui.add_space(12.0);
                if ui.add(action(label)).clicked() {
                    self.send(new);
                }
                return;
            }
            ui.heading("Not Connected");
            ui.add(egui::Label::new(RichText::new(&view.status).color(DIM)).wrap());
            // Home's address is the one typed here. Another server's comes from home.
            if self.current == 0 {
                ui.add_space(12.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.endpoint)
                        .desired_width(380.0)
                        .margin(egui::vec2(10.0, 8.0))
                        .font(egui::TextStyle::Monospace)
                        .hint_text("unix:/path/to/rex/server.sock"),
                );
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.add(action("Connect")).clicked() || enter {
                    self.servers[0] = Server::home(ui.ctx(), self.endpoint.trim(), &self.herald);
                    self.toast = Toast::default();
                }
            } else if ui.add(action("Try Again")).clicked() {
                self.send(Action::Reconnect);
            }
        });
    }

    /// A lost connection says so above its last screen, in plain words and with
    /// a way to try again. What the connection itself had to say comes under them.
    fn lost(&mut self, ui: &mut egui::Ui, view: &View) {
        let size = Vec2::new(ui.available_width(), 48.0);
        let (banner, response) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(banner, 0, HOVER);
        let mut text = banner.shrink2(egui::vec2(12.0, 0.0));
        let layout = Layout::right_to_left(Align::Center);
        let mut row = ui.new_child(UiBuilder::new().max_rect(text).layout(layout));
        let again = row.button("Reconnect");
        if again.clicked() {
            self.send(Action::Reconnect);
        }
        text.max.x = again.rect.left() - 12.0;
        // Room for a mark ahead of the words.
        text.min.x += 14.0;
        let server = self.servers[self.current].label();
        let said = format!("Disconnected from {server}. This is its last screen.");
        let said = fitted(ui, &said, FontId::proportional(BODY), TEXT, text.width());
        let font = FontId::proportional(SMALL);
        let why = fitted(ui, &view.status, font, DIM, text.width());
        let line = said.size().y;
        let top = banner.center().y - (line + why.size().y) / 2.0;
        let mark = egui::pos2(text.left() - 10.0, top + line / 2.0);
        ui.painter().circle_filled(mark, 3.5, RED);
        ui.painter()
            .galley(egui::pos2(text.left(), top), said, TEXT);
        let under = egui::pos2(text.left(), top + line);
        ui.painter().galley(under, why, DIM);
        // In full, where there was not the room.
        response.on_hover_text(&view.status);
    }

    /// A pane of the tab on show. One that is not `live` shows the last screen of
    /// a connection that was lost.
    fn terminal_pane(
        &mut self,
        ui: &mut egui::Ui,
        window: &Window,
        pane: &Pane,
        rect: Rect,
        live: bool,
    ) {
        let active = self.focused.as_deref() == Some(&pane.id);
        // The focused pane holds the keyboard whenever nothing else is taking it.
        let keyboard = active && !self.busy(ui.ctx());
        let mut actions = Vec::new();
        let (what, spared) = (doomed(&pane.id), pane.idle);
        let ask = self.closing.ask(what, spared);
        let over = ui.rect_contains_pointer(rect);
        // It was asked to close just now, with the pointer.
        let mut close = false;
        // Its header was clicked.
        let mut picked = false;
        // Its view is to move through its history.
        let mut scroll = None;
        let mut terminal_rect = rect;
        // One pane of several has a header, and so has one that fills the tab in
        // place of the rest.
        if window.split() {
            terminal_rect.min.y += PANE_HEADER;
            // A click on it takes the keyboard to the pane, as one in its terminal
            // does. Its buttons lie over it.
            let bar = rect.with_max_y(terminal_rect.top());
            let id = ui.scope_id().with(("header", &pane.id));
            picked = ui.interact(bar, id, Sense::click()).clicked();
            // The pane with the keyboard is marked along its top.
            if active {
                let line = Stroke::new(2.0, ACCENT);
                ui.painter().hline(bar.x_range(), bar.top() + 1.0, line);
            }
            let mut header = rect.shrink2(egui::vec2(4.0, 0.0));
            // The name sits a little low, toward the terminal it names.
            header.min.y += 2.0;
            header.max.y = terminal_rect.top();
            let layout = Layout::right_to_left(Align::Center);
            let mut buttons = ui.new_child(UiBuilder::new().max_rect(header).layout(layout));
            buttons.spacing_mut().item_spacing.x = 0.0;
            // Its buttons show under the pointer. The way back from a pane that
            // fills the tab shows always, where it would be among them.
            if !over || ask == Ask::Again {
                buttons.add_space(SMALL_BUTTON);
            } else if small_button(&mut buttons, Icon::Close, "Close Pane (Ctrl+Shift+W)").clicked()
            {
                close = true;
            }
            let (zoom, tip) = if window.zoomed {
                (Icon::Restore, "Restore Panes (Ctrl+Shift+Enter)")
            } else {
                (Icon::Zoom, "Zoom Pane (Ctrl+Shift+Enter)")
            };
            if (over || window.zoomed) && small_button(&mut buttons, zoom, tip).clicked() {
                let pane = pane.id.clone();
                actions.push(Action::Zoom { pane });
            }
            header.max.x -= 2.0 * SMALL_BUTTON + 4.0;
            // What it has to say sits ahead of them, as on its tab.
            if let (Some(word), Some((_, color))) = (pane.activity.word(), dot(pane.activity)) {
                let (edge, font) = (header.right_center(), FontId::proportional(SMALL));
                let word = ui
                    .painter()
                    .text(edge, Align2::RIGHT_CENTER, word, font, color);
                header.max.x = word.left() - 8.0;
            }
            // A title too long for the room left ends in an ellipsis.
            let color = if active { TEXT } else { DIM };
            let font = FontId::proportional(SMALL);
            let title = fitted(ui, &pane.title, font, color, header.width() - 8.0);
            let corner = header.left_center() + egui::vec2(8.0, -title.size().y / 2.0);
            ui.painter().galley(corner, title, color);
        }
        let state = self.panes.entry(pane.id.clone()).or_default();
        // A selection keeps showing the frame it was made on, including on the
        // frame of the click that clears it.
        let selected = state.selection.as_ref().map(|s| s.frame.clone());
        let frame = selected.or_else(|| pane.frame.clone());
        if let Some(frame) = &frame {
            ui.painter().rect_filled(terminal_rect, 0, rgb(frame.bg));
        }
        let grid = Grid::new(
            ui.ctx(),
            terminal_rect.shrink2(egui::vec2(12.0, 8.0)),
            self.font_size,
        );
        let id = ui.scope_id().with(("terminal", &pane.id));
        let response = ui.interact(terminal_rect, id, Sense::click_and_drag());
        if keyboard {
            if !response.has_focus() {
                response.request_focus();
            }
            // Including the keys that would otherwise move it elsewhere.
            let keys = egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(id, keys));
        }
        // With Ctrl held, a link under the pointer is there to be followed, even in
        // a program that takes the mouse for itself.
        let link = frame.as_ref().and_then(|frame| {
            let pointer = response
                .hover_pos()
                .filter(|_| ui.input(|i| i.modifiers.ctrl))?;
            let (x, y) = grid.cell_at(pointer, frame);
            link::at(frame, x.into(), y.into())
        });
        if response.hovered() {
            let cursor = match link {
                Some(_) => egui::CursorIcon::PointingHand,
                None => egui::CursorIcon::Text,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        // Where it leads is said in full: its text may claim anything.
        let response = match &link {
            Some(link) => response.on_hover_text_at_pointer(&link.target),
            None => response,
        };
        if response.clicked()
            && let Some(link) = &link
            && let Err(error) = link::open(&link.target)
        {
            self.toast
                .say(format!("The link could not be opened: {error}"));
        }
        let size = grid.size(ui.ctx().pixels_per_point());
        if state.last_size != Some(size) {
            let (pane, client) = (pane.id.clone(), &self.servers[self.current].client);
            // A size that could not be sent is sent when it can be.
            if client.try_send(Action::Resize { pane, size }).is_ok() {
                state.last_size = Some(size);
            }
        }
        // Shift overrides a program's mouse capture.
        let tracking =
            frame.as_ref().is_some_and(|f| f.mouse_tracking) && !ui.input(|i| i.modifiers.shift);
        // A middle click pastes the desktop's last selection, made in any program,
        // unless a program that tracks the mouse has the click.
        let paste = |event: &egui::Event| match event {
            egui::Event::MiddleClickPaste { text, .. } => Some(text.clone()),
            _ => None,
        };
        let paste = ui.input(|i| i.events.iter().find_map(paste));
        let paste = paste.filter(|_| response.hovered() && !tracking);
        let pressed = response.clicked() || response.drag_started() || paste.is_some();
        if pressed || picked {
            self.focused = Some(pane.id.clone());
            // A pane that is this window's, and where the server has the keyboard
            // already, needs no telling.
            if !active || !pane.owner || window.focus.as_ref() != Some(&pane.id) {
                actions.push(Action::FocusPane {
                    pane: pane.id.clone(),
                });
            }
        }
        if pressed {
            state.selection = None;
        }
        if let Some(text) = paste {
            let pane = pane.id.clone();
            actions.push(Action::Paste { pane, text });
        }
        if let Some(frame) = &frame {
            let events = ui.input(|i| i.events.clone());
            let pointer = Pointer {
                ui,
                response: &response,
                events: &events,
                grid,
                frame,
                pane: &pane.id,
                keyboard: &self.keyboard,
                tracking: tracking && link.is_none(),
            };
            pointer.forward(state, &mut actions);
            if !tracking {
                pointer.select(state);
            }
            scroll = pointer.wheel(Wheel::of(frame, tracking), state, &mut actions);
            // A screen that is no longer kept up shows a cursor nobody is at.
            state.paint(ui.painter(), grid, frame, active && live);
            for &(row, start, end) in link.iter().flat_map(|link| &link.spans) {
                let span = grid.span(row, start, end).intersect(grid.rect);
                let line = Stroke::new(1.0, rgb(frame.fg));
                ui.painter()
                    .hline(span.x_range(), span.bottom() - 1.0, line);
            }
            // Scrolled back, it says so at its foot, where a click returns it to
            // the last of its output. How far back is known only as of this
            // frame, so that is not said.
            if frame.scroll_offset > 0 {
                let font = FontId::proportional(SMALL);
                let said = ui
                    .painter()
                    .layout_no_wrap("Scrolled back".into(), font, TEXT);
                let size = said.size() + egui::vec2(20.0, 10.0);
                let pill = Rect::from_min_size(grid.rect.right_bottom() - size, size);
                let id = ui.scope_id().with(("scrolled", &pane.id));
                let back = ui
                    .interact(pill, id, Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("To the bottom (Shift+End)");
                if back.clicked() {
                    scroll = Some(Scroll::Bottom);
                }
                ui.painter().rect_filled(pill, size.y / 2.0, POPOVER);
                ui.painter()
                    .galley(pill.min + egui::vec2(10.0, 5.0), said, TEXT);
            }
            if active && response.has_focus() {
                compose(ui, grid, frame, &self.keyboard.preedit);
            }
            // And goes dim, halfway to its background.
            if !live {
                let veil = rgb(frame.bg).gamma_multiply(0.5);
                ui.painter().rect_filled(terminal_rect, 0, veil);
            }
        } else {
            ui.painter().text(
                grid.rect.center(),
                Align2::CENTER_CENTER,
                &pane.status,
                FontId::proportional(BODY),
                DIM,
            );
        }
        // One line between neighbours, none around the outside.
        let line = Stroke::new(1.0, BORDER);
        if pane.x > 0.0 {
            ui.painter().vline(rect.left(), rect.y_range(), line);
        }
        if pane.y > 0.0 {
            ui.painter().hline(rect.x_range(), rect.top(), line);
        }
        if !tracking {
            asking(&response).show(|ui| {
                let copy = egui::Button::new("Copy selection");
                if ui.add_enabled(state.selection.is_some(), copy).clicked()
                    && let Some(selection) = &state.selection
                {
                    ui.ctx().copy_text(selection.text());
                    ui.close();
                }
                if ui.button("Paste").clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    self.focused = Some(pane.id.clone());
                    ui.close();
                }
                ui.separator();
                let pane = || pane.id.clone();
                let split = |(_, label, _, direction)| {
                    let pane = pane();
                    (label, true, Action::Split { pane, direction })
                };
                let zoom = if window.zoomed {
                    "Restore Panes"
                } else {
                    "Zoom Pane"
                };
                let zoom = (zoom, window.split(), Action::Zoom { pane: pane() });
                for (label, enabled, action) in SPLITS.into_iter().map(split).chain([zoom]) {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        actions.push(action);
                        ui.close();
                    }
                }
                close |= closer(ui, "Close Pane", ask);
            });
        }
        self.wait(ui, rect, &response, ask);
        // Asked to close, it asks back from where its close button is, or would
        // be: it may have been asked with a key, and may have no header.
        if ask == Ask::Again {
            let edge = rect.right_top() + egui::vec2(-4.0, 1.0 + PANE_HEADER / 2.0);
            close |= confirm(ui, edge, what, CLOSE).clicked();
        }
        for action in actions {
            self.send(action);
        }
        if let Some(to) = scroll {
            self.scroll(&pane.id, to);
        }
        if close {
            let pane = pane.id.clone();
            self.close(what, spared, true, self.current, Action::ClosePane { pane });
        }
    }

    /// Move a pane's view through its history. No server is told.
    fn scroll(&mut self, pane: &str, to: Scroll) {
        // A selection holds the pane to the frame it was made on.
        if let Some(state) = self.panes.get_mut(pane) {
            state.selection = None;
        }
        let terminal = self.servers[self.current].client.terminal(pane);
        let moved = terminal.and_then(|terminal| {
            let moved = match to {
                Scroll::Lines(lines) => terminal.scroll(lines),
                Scroll::Pages(pages) => terminal.scroll_pages(pages),
                Scroll::Prompts(prompts) => terminal.scroll_prompts(prompts),
                Scroll::Bottom => {
                    terminal.scroll_to_bottom();
                    Ok(())
                }
            };
            moved.map_err(|error| error.to_string())
        });
        if let Err(error) = moved {
            self.toast.say(error);
        }
    }

    /// Send this frame's keyboard events to the focused pane.
    fn terminal_keys(&mut self, ctx: &egui::Context, pane: &str) {
        // Escape leaves a selection instead of reaching the program.
        if let Some(state) = self.panes.get_mut(pane)
            && state.selection.is_some()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            state.selection = None;
        }
        let actions = ctx.input(|i| self.keyboard.translate(pane, &i.events));
        if actions.iter().any(input::clears_selection)
            && let Some(state) = self.panes.get_mut(pane)
        {
            state.selection = None;
        }
        for action in actions {
            self.send(action);
        }
    }
}

impl eframe::App for App {
    /// This runs whether or not the window can be seen, as it cannot when the
    /// user picks one of its notes from the desktop: show the pane the note was about.
    /// On Wayland the window has been brought forward by now, if it is to be.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Some(pane) = self.desktop.picked() {
            let place = |server: &Server| server.client.place(&pane);
            let mut servers = self.servers.iter().enumerate();
            let found = servers.find_map(|(index, server)| Some((index, place(server)?)));
            if let Some((server, (session, window))) = found {
                self.open(server, Action::SelectWindow { session, window });
                self.tell(server, Action::Reveal { pane });
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        self.keyboard.settle(&mut input.events);
        self.keyboard.watch(&input.events);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        let home = self.servers[0].client.snapshot();
        self.follow_hosts(&ctx, &home);
        let others = self.servers[1..].iter().map(|s| s.client.snapshot());
        let mut views: Vec<View> = std::iter::once(home).chain(others).collect();
        for error in views.iter_mut().filter_map(|view| view.error.take()) {
            self.toast.say(error);
        }
        self.hear(&ctx, &mut views);
        let made = views[self.current].made.take();
        let copied = views[self.current].copied.take();
        let view = &views[self.current];
        let window = view.current_window();
        let panes = window.map_or(&[][..], |w| &w.panes);
        // A pane this window has just made or brought out takes the keyboard. One
        // made from elsewhere does not: that would take it from under the user's hands.
        if made.is_some() {
            self.focused = made;
        }
        // Focus stays with a pane while it is on screen, and otherwise goes where the
        // server has it.
        if !panes.iter().any(|p| Some(&p.id) == self.focused.as_ref()) {
            self.focused = window.and_then(Window::front).map(|p| p.id.clone());
        }
        // Whether this frame's keys were pressed for something other than a terminal.
        let busy = self.busy(&ctx);
        // A program copies to the clipboard only from the pane the user is at, in
        // the window they are at.
        if let Some((pane, text)) = copied
            && Some(&pane) == self.focused.as_ref()
            && !busy
            && ctx.input(|i| i.focused)
        {
            ctx.copy_text(text);
        }
        self.shortcuts(&ctx, view);
        if self.sidebar {
            self.sidebar(root_ui, &views);
        }
        let terminal = panes.iter().find_map(|p| p.frame.as_ref());
        let ground = terminal.map_or(VIEW, |frame| style::ground(rgb(frame.bg)));
        // The panel's work, and the space it has for terminals.
        let shown = egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(ground))
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if !self.sidebar {
                    self.toolbar(ui, &views);
                }
                let Some(window) = window.filter(|window| !window.panes.is_empty()) else {
                    let area = ui.available_rect_before_wrap();
                    self.empty(ui, view);
                    return area;
                };
                // Healthy needs no words.
                if !view.connected {
                    self.lost(ui, view);
                }
                let area = ui.available_rect_before_wrap();
                for pane in panes {
                    let min = area.min + area.size() * egui::vec2(pane.x, pane.y);
                    let size = area.size() * egui::vec2(pane.width, pane.height);
                    let rect = Rect::from_min_size(min, size);
                    self.terminal_pane(ui, window, pane, rect, view.connected);
                }
                area
            });
        if self.sidebar {
            self.edge(root_ui);
        }
        self.toast.show(&ctx, shown.inner);
        // One more drawing when the wait to be asked again to close is over, and
        // none until then.
        if let Some(left) = self.closing.lapse(Instant::now()) {
            ctx.request_repaint_after(left);
        }
        if view.connected
            && !busy
            && let Some(pane) = self.focused.clone()
        {
            self.terminal_keys(&ctx, &pane);
        }
        self.panes.retain(|id, _| view.pane(id).is_some());
        // A connection that returns is told every size again.
        if !view.connected {
            for pane in self.panes.values_mut() {
                pane.last_size = None;
            }
        }
        if let Some(rename) = &mut self.rename
            && !std::mem::take(&mut rename.shown)
        {
            self.rename = None;
        }
        // A tab that has closed needs no remembering. Which those are is known only
        // with every server heard from.
        if views.iter().all(|view| view.connected) {
            let tabs = || views.iter().flat_map(View::tabs);
            let open = |tab: &str| tabs().any(|(_, open)| open.id == tab);
            let _ = self.muted.keep(open);
        }
    }
}

/// Trouble is said once, briefly, over the terminals.
#[derive(Default)]
struct Toast(Option<(String, Instant)>);

impl Toast {
    const TIME: Duration = Duration::from_secs(5);

    fn say(&mut self, message: String) {
        self.0 = Some((message, Instant::now() + Self::TIME));
    }

    /// Show it at the head of the space the terminals have: at the foot is the
    /// line being typed, more often than not. A click sends it away.
    fn show(&mut self, ctx: &egui::Context, within: Rect) {
        let Some((message, until)) = &self.0 else {
            return;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.0 = None;
            return;
        }
        ctx.request_repaint_after(left);
        let frame = egui::Frame::new()
            .fill(POPOVER)
            .corner_radius(12)
            .inner_margin(Margin::symmetric(16, 10))
            .shadow(ctx.global_style().visuals.popup_shadow);
        let area = egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Foreground)
            .constrain_to(within)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 12.0));
        let shown = area.show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_max_width((within.width() - 64.0).clamp(120.0, 520.0));
                // Its words are not for selecting: that would take the click.
                ui.add(egui::Label::new(message).selectable(false));
            });
        });
        if shown.response.clicked() {
            self.0 = None;
            ctx.request_repaint();
        }
    }
}

/// The mark of a tab whose programs have something to say, in the colour of it.
fn dot(activity: Activity) -> Option<(Icon, Color32)> {
    let color = match activity {
        Activity::Idle => return None,
        Activity::Working => ACCENT,
        Activity::Done => GREEN,
        Activity::Error => RED,
        Activity::Rang | Activity::Blocked(_) => AMBER,
    };
    Some((Icon::Dot, color))
}

/// What a session's or a tab's menu was asked, if anything. A double click asks for
/// a new name, as the menu's first entry does. A tab says whether it is `muted`.
/// The row is the one at `index` among its like, whose IDs `id` gives; `labels`
/// name moving it back, moving it on, and closing it, and `ask` says what
/// choosing to close it will do.
fn menu(
    response: &egui::Response,
    labels: [&str; 3],
    muted: Option<bool>,
    index: usize,
    id: impl Fn(usize) -> Option<String>,
    ask: Ask,
) -> Option<Choice> {
    let mut picked = response.double_clicked().then_some(Choice::Rename);
    asking(response).show(|ui| {
        let mute = |muted| if muted { "Unmute Tab" } else { "Mute Tab" };
        let entries = [
            Some(("Rename", true, Choice::Rename)),
            muted.map(|muted| (mute(muted), true, Choice::Mute)),
            Some((
                labels[0],
                index > 0,
                Choice::Edit(Edit::MoveBefore(id(index.saturating_sub(1)))),
            )),
            Some((
                labels[1],
                id(index + 1).is_some(),
                // Before the one after the next, or last of all.
                Choice::Edit(Edit::MoveBefore(id(index + 2))),
            )),
        ];
        for (label, enabled, choice) in entries.into_iter().flatten() {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                picked = Some(choice);
                ui.close();
            }
        }
        if closer(ui, labels[2], ask) {
            picked = Some(Choice::Edit(Edit::Close));
        }
    });
    picked
}

/// The menu of whatever was clicked with the other button. A click in it leaves
/// it open: its entries close it, but for one that asks to be chosen twice.
fn asking(response: &egui::Response) -> egui::Popup<'_> {
    egui::Popup::context_menu(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// The entry of a menu that closes something, set apart at its foot. True when
/// chosen. Chosen once, it stays under the pointer and shows that it waits to be
/// chosen again.
fn closer(ui: &mut egui::Ui, label: &str, ask: Ask) -> bool {
    ui.separator();
    let entry = egui::Button::new(label);
    let entry = match ask {
        Ask::Again => entry.fill(DANGER),
        _ => entry,
    };
    let chosen = ui.add(entry).clicked();
    if chosen && ask != Ask::First {
        ui.close();
    }
    chosen
}

/// A row of controls, such as the toolbar or a server's heading.
fn bar(ui: &mut egui::Ui, height: f32, inset: f32) -> egui::Ui {
    let size = Vec2::new(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let rect = rect.shrink2(egui::vec2(inset, 0.0));
    let layout = Layout::left_to_right(Align::Center);
    let mut bar = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    bar.spacing_mut().item_spacing.x = 2.0;
    bar
}

/// One pane's pointer input for one frame.
struct Pointer<'a> {
    ui: &'a egui::Ui,
    response: &'a egui::Response,
    events: &'a [egui::Event],
    grid: Grid,
    frame: &'a Arc<Frame>,
    pane: &'a str,
    keyboard: &'a Keyboard,
    /// The program in the pane has captured the mouse.
    tracking: bool,
}

impl Pointer<'_> {
    /// Forward buttons and motion to a program tracking the mouse. A button it saw
    /// pressed is followed through to its release even if tracking ends meanwhile.
    fn forward(&self, state: &mut PaneView, actions: &mut Vec<Action>) {
        let mut motion = None;
        for event in self.events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    // Hovered, not merely inside: whatever is drawn over the pane keeps its clicks.
                    let start = *pressed && self.tracking && self.response.hovered();
                    let end = !pressed && state.mouse_button == Some(*button);
                    if start || end {
                        let (x, y) = self.grid.cell_at(*pos, self.frame);
                        actions.push(Action::Mouse {
                            pane: self.pane.into(),
                            kind: if *pressed { "press" } else { "release" },
                            x,
                            y,
                            button: Some(mouse_button(*button)),
                            modifiers: self.keyboard.bits(*modifiers),
                        });
                        state.mouse_button = pressed.then_some(*button);
                        if *pressed {
                            state.selection = None;
                        }
                    }
                }
                egui::Event::PointerMoved(pos)
                    if (self.tracking && self.response.hovered())
                        || state.mouse_button.is_some() =>
                {
                    motion = Some(self.grid.cell_at(*pos, self.frame));
                }
                _ => {}
            }
        }
        if let Some((x, y)) = motion
            && state.mouse_cell != Some((x, y))
        {
            state.mouse_cell = Some((x, y));
            actions.push(Action::Mouse {
                pane: self.pane.into(),
                kind: "motion",
                x,
                y,
                button: state.mouse_button.map(mouse_button),
                modifiers: self.keyboard.bits(self.ui.input(|i| i.modifiers)),
            });
        }
    }

    /// Drag to select cells; double-click to select a word.
    fn select(&self, state: &mut PaneView) {
        let (response, grid, frame) = (self.response, self.grid, self.frame);
        let selection = |range| {
            let frame = frame.clone();
            Some(Selection { range, frame })
        };
        if state.mouse_button.is_none() {
            let origin = self.ui.input(|i| i.pointer.press_origin());
            if response.drag_started()
                && let Some(pos) = origin.or_else(|| response.interact_pointer_pos())
            {
                let index = grid.index_at(pos, frame);
                state.selection = selection((index, index));
            }
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
                && let Some(selection) = &mut state.selection
            {
                selection.range.1 = grid.index_at(pos, frame);
            }
        }
        if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            state.selection = selection(render::word_at(frame, grid.index_at(pos, frame)));
        }
        // Made, it is the desktop's to paste with a middle click, in any program.
        if (response.drag_stopped_by(PointerButton::Primary) || response.double_clicked())
            && let Some(selection) = &state.selection
        {
            let made = egui::OutputCommand::TextSelectionSettled(selection.text());
            self.ui.ctx().send_cmd(made);
        }
    }

    /// Take the wheel's turn over this pane for what it is: keys or steps for a
    /// program, or a move through history, which is returned.
    fn wheel(
        &self,
        wheel: Wheel,
        state: &mut PaneView,
        actions: &mut Vec<Action>,
    ) -> Option<Scroll> {
        let pos = self.response.hover_pos()?;
        let steps = wheel.steps(self.events, self.grid, &mut state.wheel);
        let turn: Vec<Action> = match wheel {
            _ if steps == 0 => return None,
            Wheel::Nothing => return None,
            Wheel::History => return Some(Scroll::Lines(steps)),
            // A press and its release, which the server writes as the program
            // has asked to be sent its arrow keys.
            Wheel::Arrows => [true, false]
                .map(|pressed| Action::Key {
                    pane: self.pane.into(),
                    key: String::new(),
                    code: if steps < 0 { "arrow_up" } else { "arrow_down" }.into(),
                    modifiers: 0,
                    pressed,
                    repeat: false,
                })
                .into(),
            // The server takes each for one step, however far it says it went.
            Wheel::Program => {
                let (x, y) = self.grid.cell_at(pos, self.frame);
                let (pane, lines) = (self.pane.into(), steps.signum());
                vec![Action::Wheel { pane, x, y, lines }]
            }
        };
        state.selection = None;
        actions.extend(std::iter::repeat_n(turn, steps.unsigned_abs() as usize).flatten());
        None
    }
}

/// Place the IME at the terminal cursor and show its composition there.
fn compose(ui: &egui::Ui, grid: Grid, frame: &Frame, preedit: &str) {
    let (x, y) = frame.cursor.unwrap_or_default();
    let cursor = grid.span(y.into(), x.into(), usize::from(x) + 1);
    ui.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            rect: grid.rect,
            cursor_rect: cursor,
            should_interrupt_composition: false,
        })
    });
    if !preedit.is_empty() {
        let galley = ui.painter().layout_no_wrap(
            preedit.to_owned(),
            FontId::monospace(grid.font_size),
            TEXT,
        );
        let background = Rect::from_min_size(cursor.min, galley.size()).expand(3.0);
        ui.painter().rect_filled(background, 4, POPOVER);
        ui.painter().galley(cursor.min, galley, TEXT);
    }
}

fn mouse_button(button: egui::PointerButton) -> &'static str {
    match button {
        egui::PointerButton::Secondary => "right",
        egui::PointerButton::Middle => "middle",
        _ => "left",
    }
}

#[cfg(test)]
#[path = "../tests/unit/ui.rs"]
mod tests;
