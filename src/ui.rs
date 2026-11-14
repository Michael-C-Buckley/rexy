//! A deliberately small desktop shell: a sidebar of sessions, one row of tabs, and
//! the terminal. Terminal content never becomes an egui widget tree.
mod input;
mod render;
mod style;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Align2, FontId, Key, Layout, Margin, Modifiers, Rect, RichText, Sense, Stroke,
    UiBuilder, Vec2,
};

use crate::{
    backend::Client,
    model::{Action, Direction, Pane, View},
    terminal::Frame,
};
use input::{Keyboard, modifier_bits};
use render::{Grid, PaneView, Selection, rgb};
use style::{
    BODY, BORDER, DIM, HOVER, Icon, POPOVER, SIDEBAR, SMALL, TEXT, TOOLBAR, VIEW, bold,
    icon_button, item, suggested,
};

const DEFAULT_FONT_SIZE: f32 = 15.0;
const SIDEBAR_WIDTH: f32 = 240.0;
const PANE_HEADER: f32 = 28.0;
const SPLITS: [(Icon, &str, Direction); 2] = [
    (Icon::SplitRight, "Split Right", Direction::Right),
    (Icon::SplitDown, "Split Down", Direction::Down),
];

pub struct App {
    client: Client,
    endpoint: String,
    sidebar: bool,
    /// The name being typed while the new-session dialog is open.
    new_session: Option<String>,
    /// The pane that has the keyboard, while one is on screen.
    focused: Option<String>,
    panes: HashMap<String, PaneView>,
    keyboard: Keyboard,
    font_size: f32,
    toast: Toast,
}

fn connect(ctx: &egui::Context, endpoint: &str) -> Client {
    let ctx = ctx.clone();
    Client::spawn(endpoint.to_owned(), Arc::new(move || ctx.request_repaint()))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, endpoint: String) -> Self {
        style::apply(&cc.egui_ctx);
        Self {
            client: connect(&cc.egui_ctx, &endpoint),
            endpoint,
            sidebar: true,
            new_session: None,
            focused: None,
            panes: HashMap::new(),
            keyboard: Keyboard::default(),
            font_size: DEFAULT_FONT_SIZE,
            toast: Toast::default(),
        }
    }

    fn send(&mut self, action: Action) {
        if let Err(error) = self.client.try_send(action) {
            self.toast.say(error);
        }
    }

    /// A dialog or a menu is up, and keeps the keyboard to itself.
    fn modal(&self, ctx: &egui::Context) -> bool {
        self.new_session.is_some() || egui::Popup::is_any_open(ctx)
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
        if self.modal(ctx) {
            return;
        }
        if pressed(chord, Key::N) {
            self.new_session = Some(String::new());
        }
        if pressed(chord, Key::T) && view.connected {
            self.send(Action::CreateWindow);
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

    fn sidebar(&mut self, root_ui: &mut egui::Ui, view: &View) {
        egui::Panel::left("sessions")
            .exact_size(SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut bar = bar(ui);
                bar.add_space(8.0);
                bar.label(RichText::new("Sessions").font(bold(BODY)));
                bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(view.connected, |ui| {
                        if icon_button(ui, Icon::Plus, "New session (Ctrl+Shift+N)").clicked() {
                            self.new_session = Some(String::new());
                        }
                    });
                });
                let rows = egui::Frame::new().inner_margin(Margin::symmetric(6, 0));
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        rows.show(ui, |ui| {
                            for session in &view.sessions {
                                let selected =
                                    view.selected_session.as_deref() == Some(&session.id);
                                let size = Vec2::new(ui.available_width(), 34.0);
                                let row = item(ui, size, &session.name, selected, Align::LEFT);
                                if row.clicked() {
                                    self.send(Action::SelectSession(session.id.clone()));
                                }
                            }
                        });
                    });
            });
    }

    /// One row: the sidebar toggle, the session's tabs, and what can be added to it.
    fn toolbar(&mut self, ui: &mut egui::Ui, view: &View) {
        let mut bar = bar(ui);
        if icon_button(&mut bar, Icon::Sidebar, "Toggle sidebar (Ctrl+Shift+B)").clicked() {
            self.sidebar = !self.sidebar;
        }
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(view.connected && self.focused.is_some(), |ui| {
                for (icon, label, direction) in SPLITS.into_iter().rev() {
                    if icon_button(ui, icon, label).clicked()
                        && let Some(pane) = self.focused.clone()
                    {
                        self.send(Action::Split { pane, direction });
                    }
                }
            });
            ui.add_enabled_ui(view.connected && view.selected_session.is_some(), |ui| {
                if icon_button(ui, Icon::Plus, "New tab (Ctrl+Shift+T)").clicked() {
                    self.send(Action::CreateWindow);
                }
            });
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(4.0);
                egui::ScrollArea::horizontal()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let count = view.windows.len().max(1) as f32;
                        let width = (ui.available_width() - 4.0 * (count - 1.0)) / count;
                        let size = Vec2::new(width.clamp(96.0, 200.0), 30.0);
                        for window in &view.windows {
                            let selected = view.selected_window.as_deref() == Some(&window.id);
                            if item(ui, size, &window.name, selected, Align::Center).clicked() {
                                self.send(Action::SelectWindow(window.id.clone()));
                            }
                        }
                    });
            });
        });
    }

    /// What to show when there is no terminal to show.
    fn empty(&mut self, ui: &mut egui::Ui, view: &View) {
        let (title, detail, action) = if view.connected {
            (
                "No Terminals",
                "Programs you start keep running on the server.",
                "New Session",
            )
        } else {
            ("Not Connected", view.status.as_str(), "Connect")
        };
        ui.add_space((ui.available_height() * 0.28).max(20.0));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.heading(title);
            ui.add(egui::Label::new(RichText::new(detail).color(DIM)).wrap());
            ui.add_space(12.0);
            if !view.connected {
                ui.add(
                    egui::TextEdit::singleline(&mut self.endpoint)
                        .desired_width(380.0)
                        .margin(egui::vec2(10.0, 8.0))
                        .font(egui::TextStyle::Monospace)
                        .hint_text("unix:/path/to/rex/server.sock"),
                );
            }
            let button = suggested(action)
                .corner_radius(19)
                .min_size(Vec2::new(140.0, 38.0));
            if ui.add(button).clicked() {
                if view.connected {
                    self.new_session = Some(String::new());
                } else {
                    self.client = connect(ui.ctx(), self.endpoint.trim());
                    self.toast = Toast::default();
                }
            }
        });
    }

    fn terminal_pane(&mut self, ui: &mut egui::Ui, pane: &Pane, rect: Rect, split: bool) {
        let active = self.focused.as_deref() == Some(&pane.id);
        // The focused pane holds the keyboard whenever a dialog or a menu does not.
        let keyboard = active && !self.modal(ui.ctx());
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
                    if let Err(error) = self.client.scroll(&pane.id, lines) {
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

    fn session_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = &mut self.new_session else {
            return;
        };
        let (mut create, mut cancel) = (false, false);
        let frame = egui::Frame::new()
            .fill(POPOVER)
            .corner_radius(14)
            .inner_margin(24);
        let modal = egui::Modal::new(egui::Id::unique("new-session"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("New Session").font(bold(17.0)));
                    ui.label(RichText::new("Its programs keep running on the server.").color(DIM));
                });
                let field = ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text("Name")
                        .margin(egui::vec2(10.0, 8.0))
                        .desired_width(f32::INFINITY),
                );
                if !field.has_focus() {
                    field.request_focus();
                }
                let named = !name.trim().is_empty();
                ui.columns(2, |columns| {
                    let size = [columns[0].available_width(), 36.0];
                    cancel = columns[0]
                        .add_sized(size, egui::Button::new("Cancel"))
                        .clicked();
                    columns[1].add_enabled_ui(named, |ui| {
                        create = ui.add_sized(size, suggested("Create")).clicked();
                    });
                });
                create |= named && ui.input(|i| i.key_pressed(Key::Enter));
            });
        if create {
            let name = name.trim().to_owned();
            self.send(Action::CreateSession { name });
        }
        if create || cancel || modal.should_close() {
            self.new_session = None;
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
        let mut view = self.client.snapshot();
        if let Some(error) = view.error.take() {
            self.toast.say(error);
        }
        let view = &view;
        // Whether this frame's keys were pressed with a dialog or a menu up.
        let modal = self.modal(&ctx);
        self.shortcuts(&ctx, view);
        if self.sidebar {
            self.sidebar(root_ui, view);
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
                self.toolbar(ui, view);
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
        self.session_dialog(&ctx);
        self.toast.show(&ctx);
        if view.connected
            && !modal
            && let Some(pane) = self.focused.clone()
        {
            self.terminal_keys(&ctx, &pane);
        }
        self.panes.retain(|id, _| view.pane(id).is_some());
    }
}

/// Trouble is said once, briefly, at the foot of the window.
#[derive(Default)]
struct Toast(Option<(String, Instant)>);

impl Toast {
    fn say(&mut self, message: String) {
        self.0 = Some((message, Instant::now() + Duration::from_secs(5)));
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

/// A row of controls at the top of the sidebar or the content.
fn bar(ui: &mut egui::Ui) -> egui::Ui {
    let size = Vec2::new(ui.available_width(), TOOLBAR);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let rect = rect.shrink2(egui::vec2(6.0, 0.0));
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
                    // Hovered, not merely inside: a dialog or a menu over the pane keeps its clicks.
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
