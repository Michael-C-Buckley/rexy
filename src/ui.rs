//! A deliberately small desktop shell: the sessions and their tabs, in a sidebar or
//! along the top, and the terminal. Terminal content never becomes an egui widget tree.
mod input;
mod muted;
mod render;
mod style;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Align2, Color32, DragAndDrop, FontId, Key, Layout, Margin, Modifiers,
    PointerButton, Pos2, Rect, RichText, Sense, Stroke, UiBuilder, Vec2,
};

use crate::{
    backend::Client,
    model::{Action, Activity, Direction, Edit, Host, Pane, Session, View, Window},
    terminal::Frame,
};
use input::{Keyboard, modifier_bits};
use muted::Muted;
use render::{Grid, PaneView, Selection, rgb};
use style::{
    ACCENT, AMBER, BODY, BORDER, DIM, HOVER, Icon, Item, POPOVER, SIDEBAR, SMALL, TEXT, TOOLBAR,
    VIEW, bold, icon_button, item, suggested,
};

const DEFAULT_FONT_SIZE: f32 = 15.0;
const SIDEBAR_WIDTH: f32 = 240.0;
const PANE_HEADER: f32 = 28.0;
const SPLITS: [(Icon, &str, Direction); 2] = [
    (Icon::SplitRight, "Split Right", Direction::Right),
    (Icon::SplitDown, "Split Down", Direction::Down),
];

/// A server and the connection kept to it.
struct Server {
    host: Host,
    client: Client,
}

impl Server {
    /// Unless `away`, the server's first session is shown without being asked for.
    fn connect(ctx: &egui::Context, host: Host, away: bool) -> Self {
        let ctx = ctx.clone();
        let wake = Arc::new(move || ctx.request_repaint());
        Self {
            client: Client::spawn(host.endpoints.clone(), away, wake),
            host,
        }
    }

    /// Home, the server this window was opened on, has no label of its own.
    fn home(ctx: &egui::Context, endpoint: &str) -> Self {
        let host = Host {
            label: String::new(),
            endpoints: vec![endpoint.to_owned()],
        };
        Self::connect(ctx, host, false)
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

/// A name being typed over a session's or a tab's.
struct Rename {
    target: Target,
    text: String,
    /// The field has yet to be shown.
    fresh: bool,
    /// The field was shown this frame. A row that has gone takes its renaming with it.
    shown: bool,
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
    /// The pane that has the keyboard, while one is on screen.
    focused: Option<String>,
    panes: HashMap<String, PaneView>,
    keyboard: Keyboard,
    font_size: f32,
    toast: Toast,
    rename: Option<Rename>,
    muted: Muted,
    /// What is typed in the list of sessions, and which of its rows Enter would take.
    filter: String,
    cursor: usize,
    /// A session whose close button was pressed, and until when a second press ends it.
    closing: Option<(String, Instant)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, endpoint: String) -> Self {
        style::apply(&cc.egui_ctx);
        Self {
            servers: vec![Server::home(&cc.egui_ctx, &endpoint)],
            current: 0,
            endpoint,
            sidebar: true,
            focused: None,
            panes: HashMap::new(),
            keyboard: Keyboard::default(),
            font_size: DEFAULT_FONT_SIZE,
            toast: Toast::default(),
            rename: None,
            muted: Muted::load(),
            filter: String::new(),
            cursor: 0,
            closing: None,
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
                None => Server::connect(ctx, host.clone(), true),
            };
            self.servers.push(server);
        }
        let current = self.servers.iter().position(|s| s.host == current);
        self.current = current.unwrap_or(0);
    }

    /// The most pressing thing a session's tabs have to say, muted ones aside.
    fn activity(&self, session: &Session) -> Activity {
        let heard = session.windows.iter().filter(|w| !self.muted.has(&w.id));
        heard.map(Window::activity).max().unwrap_or_default()
    }

    fn shortcuts(&mut self, ctx: &egui::Context, view: &View) {
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let pressed = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
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
        if pressed(Modifiers::CTRL, Key::Plus) || pressed(Modifiers::CTRL, Key::Equals) {
            self.font_size = (self.font_size + 1.0).min(32.0);
        }
        if pressed(Modifiers::CTRL, Key::Minus) {
            self.font_size = (self.font_size - 1.0).max(9.0);
        }
        if pressed(Modifiers::CTRL, Key::Num0) {
            self.font_size = DEFAULT_FONT_SIZE;
        }
    }

    /// The buttons that split the focused pane, laid out from the right.
    fn splits(&mut self, ui: &mut egui::Ui, view: &View) {
        ui.add_enabled_ui(view.connected && self.focused.is_some(), |ui| {
            for (icon, label, direction) in SPLITS.into_iter().rev() {
                if icon_button(ui, icon, label).clicked()
                    && let Some(pane) = self.focused.clone()
                {
                    self.send(Action::Split { pane, direction });
                }
            }
        });
    }

    /// Every server, each with its sessions beneath it, and under each session its tabs.
    fn sidebar(&mut self, root_ui: &mut egui::Ui, views: &[View]) {
        let rows = egui::Frame::new().inner_margin(Margin {
            top: 0,
            ..Margin::same(6)
        });
        egui::Panel::left("sessions")
            .exact_size(SIDEBAR_WIDTH)
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

    /// A server in the sidebar: its name, its sessions, and under each its tabs.
    fn server(&mut self, ui: &mut egui::Ui, server: usize, view: &View) {
        if self.heading(ui, server, view) {
            self.open(server, Action::CreateSession(None));
        }
        if !view.connected {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let status = RichText::new(&view.status).size(SMALL).color(DIM);
                ui.add(egui::Label::new(status).truncate())
                    .on_hover_text(&view.status);
            });
        }
        let mut sessions = Vec::new();
        for (index, session) in view.sessions.iter().enumerate() {
            let top = ui.cursor().top();
            self.session(ui, server, &view.sessions, index);
            // The tab on show, if it is one of this session's.
            let shown =
                server == self.current && view.session().is_some_and(|s| s.id == session.id);
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
        let elsewhere = elsewhere.map(|(_, session)| self.activity(session)).max();
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
                            if row(ui, &session.name, dot(self.activity(session))) {
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
    fn session(&mut self, ui: &mut egui::Ui, server: usize, sessions: &[Session], index: usize) {
        let session = &sessions[index];
        let target = Target {
            server,
            session: session.id.clone(),
            window: None,
        };
        let size = Vec2::new(ui.available_width(), 28.0);
        if self.renaming(ui, &target, size).is_some() {
            return;
        }
        let row = Item {
            caption: true,
            buttons: &[(Icon::Plus, "New tab"), (Icon::Close, "Close session")],
            movable: true,
            label: &session.name,
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        if response.clicked() {
            self.open(server, Action::SelectSession(session.id.clone()));
        }
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = ["Move Up", "Move Down", "Close Session"];
        let id = |index: usize| sessions.get(index).map(|s| s.id.clone());
        match (button, menu(&response, labels, None, index, id)) {
            (Some(0), _) => self.open(server, Action::CreateWindow(session.id.clone())),
            // One press is easily a slip, and this ends every program in the session.
            (Some(_), _) => {
                let now = Instant::now();
                let armed = self.closing.take();
                if armed.is_some_and(|(id, until)| id == session.id && now < until) {
                    self.edit(target, Edit::Close);
                } else {
                    let name = &session.name;
                    self.toast.say(format!(
                        "Press again to close {name} and all that runs in it"
                    ));
                    self.closing = Some((session.id.clone(), now + Toast::TIME));
                }
            }
            (None, Some(choice)) => self.choose(target, &session.name, choice),
            (None, None) => {}
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
        if let Some(field) = self.renaming(ui, &target, size) {
            return field;
        }
        let muted = self.muted.has(&window.id);
        let row = Item {
            icon: Some(window.program()),
            selected: shown == Some(&window.id),
            align: if across { Align::Center } else { Align::LEFT },
            mark: if muted {
                Some((Icon::Muted, DIM))
            } else {
                dot(window.activity())
            },
            buttons: &[(Icon::Close, "Close tab")],
            movable: true,
            label: window.name(),
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        if response.clicked() {
            let (session, window) = (session.id.clone(), window.id.clone());
            self.open(server, Action::SelectWindow { session, window });
        }
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = if across {
            ["Move Left", "Move Right", "Close Tab"]
        } else {
            ["Move Up", "Move Down", "Close Tab"]
        };
        let id = |index: usize| session.windows.get(index).map(|w| w.id.clone());
        match (button, menu(&response, labels, Some(muted), index, id)) {
            (Some(_), _) => self.edit(target, Edit::Close),
            // One that goes by what runs in it has no name to start from.
            (None, Some(choice)) => {
                self.choose(target, window.label.as_deref().unwrap_or_default(), choice);
            }
            (None, None) => {}
        }
        response.rect
    }

    /// Do what a session's or a tab's menu was asked. `name` is the one it was given.
    fn choose(&mut self, target: Target, name: &str, choice: Choice) {
        match choice {
            Choice::Rename => {
                self.rename = Some(Rename {
                    target,
                    text: name.to_owned(),
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
            }
            Choice::Edit(edit) => self.edit(target, edit),
        }
    }

    fn edit(&mut self, target: Target, edit: Edit) {
        let Target {
            server,
            session,
            window,
        } = target;
        let action = Action::Edit {
            session,
            window,
            edit,
        };
        self.tell(server, action);
    }

    /// Show the field that stands in for a row while it is renamed, and return the
    /// space it took. None if this row is not the one.
    fn renaming(&mut self, ui: &mut egui::Ui, target: &Target, size: Vec2) -> Option<Rect> {
        let rename = self.rename.as_mut().filter(|r| &r.target == target)?;
        rename.shown = true;
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let field = egui::TextEdit::singleline(&mut rename.text)
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
            // click elsewhere, takes the new one.
            let name = self.rename.take().map(|rename| rename.text);
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
                ui.add(
                    egui::TextEdit::singleline(&mut self.endpoint)
                        .desired_width(380.0)
                        .margin(egui::vec2(10.0, 8.0))
                        .font(egui::TextStyle::Monospace)
                        .hint_text("unix:/path/to/rex/server.sock"),
                );
                if ui.add(action("Connect")).clicked() {
                    self.servers[0] = Server::home(ui.ctx(), self.endpoint.trim());
                    self.toast = Toast::default();
                }
            } else if ui.add(action("Try Again")).clicked() {
                self.send(Action::Reconnect);
            }
        });
    }

    fn terminal_pane(&mut self, ui: &mut egui::Ui, pane: &Pane, rect: Rect, split: bool) {
        let active = self.focused.as_deref() == Some(&pane.id);
        // The focused pane holds the keyboard whenever nothing else is taking it.
        let keyboard = active && !self.busy(ui.ctx());
        let mut terminal_rect = rect;
        if split {
            ui.painter().with_clip_rect(rect).text(
                rect.left_top() + egui::vec2(12.0, PANE_HEADER / 2.0 + 2.0),
                Align2::LEFT_CENTER,
                &pane.title,
                FontId::proportional(SMALL),
                if active { TEXT } else { DIM },
            );
            terminal_rect.min.y += PANE_HEADER;
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
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let mut actions = Vec::new();
        let size = grid.size(ui.ctx().pixels_per_point());
        // Only owned panes are resized, so report the size again when ownership returns.
        if !pane.owner {
            state.last_size = None;
        } else if state.last_size != Some(size) {
            state.last_size = Some(size);
            actions.push(Action::Resize {
                pane: pane.id.clone(),
                size,
            });
        }
        if response.clicked() || response.drag_started() {
            self.focused = Some(pane.id.clone());
            actions.push(Action::FocusPane {
                pane: pane.id.clone(),
            });
            state.selection = None;
        }
        // Shift overrides a program's mouse capture.
        let tracking =
            frame.as_ref().is_some_and(|f| f.mouse_tracking) && !ui.input(|i| i.modifiers.shift);
        if let Some(frame) = &frame {
            let events = ui.input(|i| i.events.clone());
            let pointer = Pointer {
                ui,
                response: &response,
                events: &events,
                grid,
                frame,
                pane: &pane.id,
                tracking,
            };
            pointer.forward(state, &mut actions);
            if !tracking {
                pointer.select(state);
            }
            match pointer.wheel(state) {
                0 => {}
                lines if !tracking => {
                    let client = &self.servers[self.current].client;
                    if let Err(error) = client.scroll(&pane.id, lines) {
                        self.toast.say(error);
                    }
                }
                lines => {
                    if let Some(pos) = response.hover_pos() {
                        let (x, y) = grid.cell_at(pos, frame);
                        actions.push(Action::Wheel {
                            pane: pane.id.clone(),
                            x,
                            y,
                            lines,
                        });
                    }
                }
            }
            state.paint(ui.painter(), grid, frame, active);
            if active && response.has_focus() {
                compose(ui, grid, frame, &self.keyboard.preedit);
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
            response.context_menu(|ui| {
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
                for (_, label, direction) in SPLITS {
                    if ui.button(label).clicked() {
                        actions.push(Action::Split {
                            pane: pane.id.clone(),
                            direction,
                        });
                        ui.close();
                    }
                }
            });
        }
        for action in actions {
            self.send(action);
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
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        let home = self.servers[0].client.snapshot();
        self.follow_hosts(&ctx, &home);
        let others = self.servers[1..].iter().map(|s| s.client.snapshot());
        let mut views: Vec<View> = std::iter::once(home).chain(others).collect();
        for error in views.iter_mut().filter_map(|view| view.error.take()) {
            self.toast.say(error);
        }
        let view = &views[self.current];
        // Whether this frame's keys were pressed for something other than a terminal.
        let busy = self.busy(&ctx);
        self.shortcuts(&ctx, view);
        if self.sidebar {
            self.sidebar(root_ui, &views);
        }
        let panes = view.current_window().map_or(&[][..], |w| &w.panes);
        // Focus stays with a pane while it is on screen, and otherwise goes to the first.
        if !panes.iter().any(|p| Some(&p.id) == self.focused.as_ref()) {
            self.focused = panes.first().map(|p| p.id.clone());
        }
        let terminal = panes.iter().find_map(|p| p.frame.as_ref());
        let ground = terminal.map_or(VIEW, |frame| style::ground(rgb(frame.bg)));
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(ground))
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if !self.sidebar {
                    self.toolbar(ui, &views);
                }
                if panes.is_empty() {
                    return self.empty(ui, view);
                }
                // Healthy needs no words. A lost connection says so above its last screen.
                if !view.connected {
                    let size = Vec2::new(ui.available_width(), 36.0);
                    let (banner, _) = ui.allocate_exact_size(size, Sense::hover());
                    ui.painter().rect_filled(banner, 0, HOVER);
                    ui.painter().with_clip_rect(banner).text(
                        banner.center(),
                        Align2::CENTER_CENTER,
                        &view.status,
                        FontId::proportional(BODY),
                        TEXT,
                    );
                }
                let area = ui.available_rect_before_wrap();
                for pane in panes {
                    let min = area.min + area.size() * egui::vec2(pane.x, pane.y);
                    let size = area.size() * egui::vec2(pane.width, pane.height);
                    let rect = Rect::from_min_size(min, size);
                    self.terminal_pane(ui, pane, rect, panes.len() > 1);
                }
            });
        self.toast.show(&ctx);
        if view.connected
            && !busy
            && let Some(pane) = self.focused.clone()
        {
            self.terminal_keys(&ctx, &pane);
        }
        self.panes.retain(|id, _| view.pane(id).is_some());
        if let Some(rename) = &mut self.rename
            && !std::mem::take(&mut rename.shown)
        {
            self.rename = None;
        }
        // A tab that has closed needs no remembering. Which those are is known only
        // with every server heard from.
        if views.iter().all(|view| view.connected) {
            let sessions = || views.iter().flat_map(|view| &view.sessions);
            let open = |tab: &str| sessions().flat_map(|s| &s.windows).any(|w| w.id == tab);
            let _ = self.muted.keep(open);
        }
    }
}

/// Trouble is said once, briefly, at the foot of the window.
#[derive(Default)]
struct Toast(Option<(String, Instant)>);

impl Toast {
    const TIME: Duration = Duration::from_secs(5);

    fn say(&mut self, message: String) {
        self.0 = Some((message, Instant::now() + Self::TIME));
    }

    fn show(&mut self, ctx: &egui::Context) {
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
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
            .interactable(false)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_max_width(520.0);
                    ui.label(message);
                });
            });
    }
}

/// The mark of a tab whose programs have something to say.
fn dot(activity: Activity) -> Option<(Icon, Color32)> {
    match activity {
        Activity::Idle => None,
        Activity::Working => Some((Icon::Dot, ACCENT)),
        Activity::Blocked => Some((Icon::Dot, AMBER)),
    }
}

/// What a session's or a tab's menu was asked, if anything. A double click asks for
/// a new name, as the menu's first entry does. A tab says whether it is `muted`.
/// The row is the one at `index` among its like, whose IDs `id` gives; `labels`
/// name moving it back, moving it on, and closing it.
fn menu(
    response: &egui::Response,
    labels: [&str; 3],
    muted: Option<bool>,
    index: usize,
    id: impl Fn(usize) -> Option<String>,
) -> Option<Choice> {
    let mut picked = response.double_clicked().then_some(Choice::Rename);
    response.context_menu(|ui| {
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
            Some((labels[2], true, Choice::Edit(Edit::Close))),
        ];
        for (label, enabled, choice) in entries.into_iter().flatten() {
            if matches!(choice, Choice::Edit(Edit::Close)) {
                ui.separator();
            }
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                picked = Some(choice);
                ui.close();
            }
        }
    });
    picked
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
                            modifiers: modifier_bits(*modifiers),
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
                kind: "move",
                x,
                y,
                button: state.mouse_button.map(mouse_button),
                modifiers: modifier_bits(self.ui.input(|i| i.modifiers)),
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
    }

    /// Whole lines the wheel turned over this pane; negative is up.
    fn wheel(&self, state: &mut PaneView) -> i32 {
        if !self.response.hovered() {
            return 0;
        }
        let cell = self.grid.cell.y;
        let points: f32 = self
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Line => delta.y * cell,
                    egui::MouseWheelUnit::Page => delta.y * self.grid.rect.height(),
                }),
                _ => None,
            })
            .sum();
        state.wheel += points / cell;
        let lines = state.wheel.trunc();
        if lines != 0.0 {
            state.wheel -= lines;
            state.selection = None;
        }
        -lines as i32
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
