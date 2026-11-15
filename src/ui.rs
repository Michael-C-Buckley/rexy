//! A deliberately small desktop shell: the sessions and their tabs, in a sidebar or
//! along the top, and the terminal. Terminal content never becomes an egui widget tree.
mod agents;
mod borders;
mod boxes;
mod close;
pub mod commands;
mod find;
mod frame;
mod heard;
mod input;
mod keymap;
mod keys;
mod motion;
mod palette;
mod pane;
mod pointer;
mod raise;
mod remembered;
mod render;
mod rows;
mod sidebar;
mod style;
mod toolbar;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Align2, FontId, Key, Margin, Rect, RichText, Sense, Vec2};

use crate::{
    backend::Client,
    model::{Action, Host, View, Window},
    notify::{Desktop, Herald},
};
use close::Closing;
use find::Find;
use heard::Heard;
use input::Keyboard;
use keymap::Keys;
use raise::Raise;
pub use remembered::{LEAST_WINDOW, Remembered, state_home};
use render::{PaneView, rgb};
use rows::Rename;
pub use style::icon;
use style::{BODY, DIM, GUTTER, TEXT, fitted, suggested};

const DEFAULT_FONT_SIZE: f32 = 15.0;
/// The least and the most the keys take it to.
const FONT_SIZES: [f32; 2] = [9.0, 32.0];

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
            client: Client::spawn(
                host.endpoints.clone(),
                wake,
                herald,
                Some(commands::request()),
            ),
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

    /// Home goes by its address, unless that is a socket on this machine.
    fn label(&self) -> &str {
        let address = self.host.endpoints.first().map_or("", String::as_str);
        match self.host.label.as_str() {
            "" if address.starts_with('/') || address.starts_with("unix:") => "This Computer",
            "" => address
                .split_once("://")
                .map_or(address, |(_, machine)| machine),
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
    /// The pane that has the keyboard, while one is on screen.
    focused: Option<String>,
    panes: HashMap<String, PaneView>,
    keyboard: Keyboard,
    /// Where the keyboard stands with the keymap of the server on show.
    keys: Keys,
    /// What the buttons and menus have asked for since the window last drew, each
    /// by its name in the table of commands and the pane it was asked of, if one.
    todo: Vec<(&'static str, Option<String>)>,
    toast: Toast,
    rename: Option<Rename>,
    /// The field that finds text in a pane, while it is open.
    find: Option<Find>,
    remembered: Remembered,
    /// The session the window's title names.
    title: String,
    /// By pane.
    heard: HashMap<String, Heard>,
    /// Where the panes of the tab on show are on their way to.
    motion: motion::Motion,
    /// The sidebar lists what is at work, in place of the sessions.
    agents: bool,
    /// Where the servers' connections send what their panes have to say, and what
    /// passes it on to the desktop.
    herald: Herald,
    desktop: Desktop,
    /// What is typed in the palette, and which of its rows Enter would take, once
    /// it has shown any.
    filter: String,
    cursor: Option<usize>,
    closing: Closing,
    /// The tab on show that its list was last scrolled to, and in which list. It
    /// is scrolled to once, and the list is the user's to scroll after that.
    followed: Option<egui::Id>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, endpoint: String, remembered: Remembered) -> Self {
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
        desktop.mute(remembered.kept.muted.clone());
        Self {
            servers: vec![Server::home(&cc.egui_ctx, &endpoint, &herald)],
            current: 0,
            endpoint,
            focused: None,
            panes: HashMap::new(),
            keyboard: Keyboard::default(),
            keys: Keys::default(),
            todo: Vec::new(),
            toast: Toast::default(),
            rename: None,
            find: None,
            remembered,
            title: String::new(),
            heard: HashMap::new(),
            motion: motion::Motion::default(),
            agents: false,
            herald,
            desktop,
            filter: String::new(),
            cursor: None,
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

    /// Show a pane of a server's, whatever tab it is in and whatever hides it
    /// there. False if the server has no such pane.
    fn reveal(&mut self, server: usize, pane: &str) -> bool {
        let Some((session, window)) = self.servers[server].client.place(pane) else {
            return false;
        };
        self.open(server, Action::SelectWindow { session, window });
        let pane = pane.to_owned();
        self.tell(server, Action::Reveal { pane });
        true
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
        self.rename.is_some() || self.find.is_some() || egui::Popup::is_any_open(ctx)
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

    /// A lost connection says so over its last screen, in plain words and with a
    /// way to try again. What the connection itself had to say comes after them,
    /// and in full under the pointer.
    fn lost(&mut self, ctx: &egui::Context, view: &View, within: Rect) {
        let frame = egui::Frame::popup(&ctx.global_style()).inner_margin(Margin {
            left: 14,
            right: 6,
            top: 6,
            bottom: 6,
        });
        let area = egui::Area::new(egui::Id::unique("lost"))
            .order(egui::Order::Middle)
            .constrain_to(within)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 12.0));
        let server = self.servers[self.current].label().to_owned();
        let mut again = false;
        area.show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (mark, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                    let failed = style::tints(ui.ctx()).failed;
                    ui.painter().circle_filled(mark.center(), 3.5, failed);
                    let room = (within.width() - 220.0).clamp(80.0, 560.0);
                    let said = format!("Lost the connection to {server}.");
                    let said = fitted(ui, &said, FontId::proportional(BODY), TEXT, room);
                    let room = room - said.size().x;
                    let said = ui.add(egui::Label::new(said).selectable(false));
                    // Why, without saying over again that it is lost.
                    let why = view.status.strip_prefix("Disconnected: ");
                    let why = why.unwrap_or(&view.status);
                    if room > 60.0 && !why.is_empty() {
                        let font = FontId::proportional(BODY);
                        let why = fitted(ui, why, font, DIM, room);
                        ui.add(egui::Label::new(why).selectable(false));
                    }
                    said.on_hover_text(&view.status);
                    let button = egui::Button::new("Reconnect").min_size(Vec2::new(0.0, 26.0));
                    again = ui.add(button).clicked();
                });
            });
        });
        if again {
            self.send(Action::Reconnect);
        }
    }
}

impl eframe::App for App {
    /// Nothing, where the window draws nothing: past its round corners.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// This runs whether or not the window can be seen, as it cannot when the
    /// user picks one of its notes from the desktop: show the pane the note was about.
    /// On Wayland the window has been brought forward by now, if it is to be.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Some(pane) = self.desktop.picked() {
            let _ = (0..self.servers.len()).any(|server| self.reveal(server, &pane));
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
        // The server on show shows a session if it has one: where the window last
        // was on it, or failing that its first. No other is asked to.
        if let Some(first) = views[self.current].unshown() {
            let back = self.returning(&views[self.current]);
            self.send(back.unwrap_or(first));
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
        self.window_focus(&ctx);
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
        // The window's ground goes by the terminals on it. So do what is laid
        // over it, menus, notes and the palette, and the colours that mark the
        // pane in use and what programs have to say.
        let shown = panes.iter().find_map(|p| p.frame.as_ref());
        let shown = shown.map(|frame| (rgb(frame.bg), frame.ansi.map(rgb)));
        frame::ground(root_ui, style::theme(&ctx, shown));
        // With the sidebar, tabs are listed in it. Without, they run along the top.
        let sidebar = self.remembered.kept.sidebar;
        if sidebar {
            self.sidebar(root_ui, &views);
        }
        // The panel's work, and the space it has for terminals.
        let shown = egui::CentralPanel::no_frame().show(root_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match sidebar {
                true => self.header(ui, view),
                false => self.toolbar(ui, &views),
            }
            let Some(window) = window.filter(|window| !window.panes.is_empty()) else {
                let area = ui.available_rect_before_wrap();
                self.empty(ui, view);
                return area;
            };
            // The terminals are cards on the window's ground, a gutter apart
            // and a gutter from its edges: but for the row above them, and the
            // sidebar, which keep their own distance. Each card gives up half
            // a gutter all round, so the space they share reaches half a
            // gutter past where the outermost of them end.
            let mut within = ui.available_rect_before_wrap();
            within.max -= Vec2::splat(GUTTER);
            if !sidebar {
                within.min.x += GUTTER;
            }
            let area = within.expand(GUTTER / 2.0);
            // Healthy needs no words.
            if !view.connected {
                self.lost(ui.ctx(), view, within);
            }
            // Each has its part of the tab, and is drawn on its way there.
            let belong = panes.iter().map(|pane| {
                let size = egui::vec2(pane.width, pane.height);
                let part = Rect::from_min_size(egui::pos2(pane.x, pane.y), size);
                (&*pane.id, part)
            });
            let belong: Vec<_> = belong.collect();
            // What the pointer has hold of, such as a border, goes where it does.
            let held = ui.input(|i| i.pointer.any_down());
            let shown = self.motion.place(ui.ctx(), &window.id, &belong, held);
            let card = |part: Rect| {
                let [min, max] = [part.min, part.max].map(|at| area.lerp_inside(at.to_vec2()));
                Rect::from_min_max(min, max).shrink(GUTTER / 2.0)
            };
            for ((pane, (_, rest)), shown) in panes.iter().zip(&belong).zip(shown) {
                self.terminal_pane(ui, view, window, pane, card(shown), card(*rest));
            }
            if view.connected {
                self.borders(ui, window, area);
            }
            within
        });
        if sidebar {
            self.edge(root_ui);
        }
        frame::edges(root_ui);
        self.answer(&ctx, &views);
        self.palette(&ctx, &views);
        self.hints(&ctx, shown.inner);
        // Under what a lost connection says there, where it says it.
        let lost = !view.connected && window.is_some_and(|tab| !tab.panes.is_empty());
        self.toast
            .show(&ctx, shown.inner, if lost { 60.0 } else { 12.0 });
        // One more drawing when the wait to be asked again to close is over, and
        // none until then.
        if let Some(left) = self.closing.lapse(Instant::now()) {
            ctx.request_repaint_after(left);
        }
        let typed = self.focused.as_deref().and_then(|id| view.pane(id));
        if view.connected
            && !busy
            && let Some(pane) = typed
        {
            self.terminal_keys(&ctx, pane);
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
        if let Some(find) = &mut self.find
            && !std::mem::take(&mut find.shown)
        {
            self.find_close();
        }
        // A tab that has closed needs no remembering. Which those are is known only
        // with every server heard from.
        if views.iter().all(|view| view.connected) {
            let tabs = || views.iter().flat_map(View::tabs);
            let open = |tab: &str| tabs().any(|(_, open)| open.id == tab);
            self.remembered.kept.muted.retain(|tab| open(tab));
        }
        self.remember(&ctx, &views);
        // The window is named for the session it shows.
        let name = view.session().map_or("", |session| &session.name);
        if name != self.title {
            self.title = name.to_owned();
            let title = match name {
                "" => "Rex Lite".into(),
                name => format!("{name} — Rex Lite"),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
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

    /// Show it `down` from the head of the space the terminals have: at the foot
    /// is the line being typed, more often than not. A click sends it away.
    fn show(&mut self, ctx: &egui::Context, within: Rect, down: f32) {
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
            .fill(ctx.global_style().visuals.window_fill)
            .corner_radius(12)
            .inner_margin(Margin::symmetric(16, 10))
            .shadow(ctx.global_style().visuals.popup_shadow);
        let area = egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Foreground)
            .constrain_to(within)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, down));
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
