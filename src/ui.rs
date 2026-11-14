//! A deliberately small desktop shell: the sessions and their tabs, in a sidebar or
//! along the top, and the terminal. Terminal content never becomes an egui widget tree.
mod close;
mod heard;
mod input;
mod keys;
mod muted;
mod pane;
mod pointer;
mod raise;
mod render;
mod rows;
mod sidebar;
mod style;
mod switcher;
mod toolbar;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Align2, FontId, Key, Layout, Margin, Rect, RichText, Sense, UiBuilder, Vec2,
};

use crate::{
    backend::Client,
    model::{Action, Host, View, Window},
    notify::{Desktop, Herald},
};
use close::Closing;
use heard::Heard;
use input::Keyboard;
use muted::Muted;
use raise::Raise;
use render::{PaneView, rgb};
use rows::Rename;
use sidebar::SIDEBAR_WIDTH;
use style::{BODY, DIM, HOVER, POPOVER, RED, SMALL, TEXT, VIEW, fitted, suggested};

const DEFAULT_FONT_SIZE: f32 = 15.0;

/// A server and the connection kept to it.
struct Server {
    host: Host,
    client: Client,
}

impl Server {
    fn connect(ctx: &egui::Context, host: Host, herald: &Herald) -> Self {
        let ctx = ctx.clone();
        let wake = Arc::new(move || ctx.request_repaint());
        let herald = herald.of(&host.label);
        Self {
            client: Client::spawn(host.endpoints.clone(), wake, herald),
            host,
        }
    }

    /// Home, the server this window was opened on, has no label of its own.
    fn home(ctx: &egui::Context, endpoint: &str, herald: &Herald) -> Self {
        let host = Host {
            label: String::new(),
            endpoints: vec![endpoint.to_owned()],
        };
        Self::connect(ctx, host, herald)
    }

    fn label(&self) -> &str {
        match self.host.label.as_str() {
            "" => "This Computer",
            label => label,
        }
    }
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
                None => Server::connect(ctx, host.clone(), &self.herald),
            };
            self.servers.push(server);
        }
        let current = self.servers.iter().position(|s| s.host == current);
        self.current = current.unwrap_or(0);
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
        // The server on show shows a session if it has one. No other is asked to.
        if let Some(first) = views[self.current].unshown() {
            self.send(first);
        }
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
