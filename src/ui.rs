//! A deliberately small desktop shell. Terminal content never becomes an egui widget tree.
mod input;
mod render;

use std::{collections::HashMap, sync::Arc};

use eframe::egui::{self, Color32, FontId, Key, Margin, Modifiers, RichText, Stroke, Vec2};

use crate::{
    backend::Client,
    model::{Action, Direction, Pane, Size, View},
    terminal::Frame,
};
use input::{Keyboard, modifier_bits};
use render::{Grid, PaneView, Selection, rgb};

const BACKGROUND: Color32 = Color32::from_rgb(17, 21, 19);
const SIDEBAR: Color32 = Color32::from_rgb(22, 27, 24);
const SURFACE: Color32 = Color32::from_rgb(31, 38, 33);
const BORDER: Color32 = Color32::from_rgb(45, 54, 47);
const TEXT: Color32 = Color32::from_rgb(220, 229, 221);
const MUTED: Color32 = Color32::from_rgb(132, 148, 137);
const ACCENT: Color32 = Color32::from_rgb(164, 198, 174);

const DEFAULT_FONT_SIZE: f32 = 15.0;
const PANE_HEADER: f32 = 26.0;
const SPLITS: [(&str, Direction); 2] = [
    ("Split right", Direction::Right),
    ("Split down", Direction::Down),
];

pub struct App {
    client: Client,
    endpoint: String,
    sidebar: bool,
    /// The name being typed while the new-session dialog is open.
    new_session: Option<String>,
    focus_name: bool,
    focused: Option<String>,
    /// Give the focused pane keyboard focus on the next frame.
    focus_terminal: bool,
    /// Whether the focused pane held keyboard focus this frame.
    terminal_keyboard: bool,
    panes: HashMap<String, PaneView>,
    keyboard: Keyboard,
    font_size: f32,
    notice: Option<String>,
    previous_session: Option<String>,
    previous_window: Option<String>,
}

fn connect(ctx: &egui::Context, endpoint: &str) -> Client {
    let ctx = ctx.clone();
    Client::spawn(endpoint.to_owned(), Arc::new(move || ctx.request_repaint()))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, endpoint: String) -> Self {
        theme(&cc.egui_ctx);
        Self {
            client: connect(&cc.egui_ctx, &endpoint),
            endpoint,
            sidebar: true,
            new_session: None,
            focus_name: false,
            focused: None,
            focus_terminal: true,
            terminal_keyboard: false,
            panes: HashMap::new(),
            keyboard: Keyboard::default(),
            font_size: DEFAULT_FONT_SIZE,
            notice: None,
            previous_session: None,
            previous_window: None,
        }
    }

    fn send(&mut self, action: Action) {
        if let Err(error) = self.client.try_send(action) {
            self.notice = Some(error);
        }
    }

    fn open_session_dialog(&mut self) {
        self.new_session = Some(String::new());
        self.focus_name = true;
    }

    fn reset_focus(&mut self) {
        self.focused = None;
        self.focus_terminal = true;
        self.keyboard.clear();
    }

    fn copy_selection(&self, ctx: &egui::Context) {
        let pane = self.focused.as_ref().and_then(|id| self.panes.get(id));
        if let Some(selection) = pane.and_then(|pane| pane.selection.as_ref()) {
            ctx.copy_text(selection.text());
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context, view: &View) {
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let pressed = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        if pressed(chord, Key::B) {
            self.sidebar = !self.sidebar;
        }
        if pressed(chord, Key::N) {
            self.open_session_dialog();
        }
        // egui-winit reports the copy and paste chords as clipboard events, never
        // as key presses. A copy is taken here; a paste reaches the focused pane.
        if self.new_session.is_none() && ctx.input_mut(|i| self.keyboard.take_copy(&mut i.events)) {
            self.copy_selection(ctx);
        }
        if self.new_session.is_some() || egui::Popup::is_any_open(ctx) {
            return;
        }
        if pressed(chord, Key::T) && view.connected {
            self.send(Action::CreateWindow);
            self.focus_terminal = true;
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

    fn status_bar(&mut self, root_ui: &mut egui::Ui, view: &View) {
        egui::Panel::bottom("status")
            .exact_size(29.0)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(Margin::symmetric(14, 4)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        3.0,
                        if view.connected { ACCENT } else { MUTED },
                    );
                    let text = self.notice.as_deref().unwrap_or(&view.status);
                    ui.add(
                        egui::Label::new(RichText::new(text).size(11.0).color(MUTED)).truncate(),
                    )
                    .on_hover_text(text);
                    if self.notice.is_some() && ui.small_button("Dismiss").clicked() {
                        self.notice = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Ctrl Shift B  sidebar")
                                .size(10.5)
                                .color(MUTED),
                        );
                    });
                });
            });
    }

    fn sidebar(&mut self, root_ui: &mut egui::Ui, view: &View) {
        egui::Panel::left("sessions")
            .exact_size(204.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(Margin::symmetric(14, 18)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("rex").size(23.0).strong().color(TEXT));
                    ui.label(RichText::new("lite").size(13.0).color(ACCENT));
                });
                ui.add_space(2.0);
                ui.label(
                    RichText::new(endpoint_label(&self.endpoint))
                        .size(11.0)
                        .color(MUTED),
                )
                .on_hover_text(&self.endpoint);
                ui.add_space(28.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SESSIONS").size(10.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(view.connected, egui::Button::new("+").frame(false))
                            .on_hover_text("New session · Ctrl Shift N")
                            .clicked()
                        {
                            self.open_session_dialog();
                        }
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for session in &view.sessions {
                            let selected = view.selected_session.as_deref() == Some(&session.id);
                            let response = ui.add_sized(
                                [ui.available_width(), 36.0],
                                item(&session.name, 13.0, selected).corner_radius(5),
                            );
                            if selected {
                                let r = response.rect;
                                ui.painter().line_segment(
                                    [
                                        r.left_top() + egui::vec2(0.0, 10.0),
                                        r.left_bottom() - egui::vec2(0.0, 10.0),
                                    ],
                                    Stroke::new(2.0, ACCENT),
                                );
                            }
                            if response.on_hover_text(&session.name).clicked() {
                                self.send(Action::SelectSession(session.id.clone()));
                                self.reset_focus();
                            }
                            ui.add_space(3.0);
                        }
                        if view.connected && view.sessions.is_empty() {
                            ui.label(RichText::new("No sessions yet").size(12.0).color(MUTED));
                        }
                    });
            });
    }

    fn tabs(&mut self, ui: &mut egui::Ui, view: &View) {
        ui.horizontal(|ui| {
            if !self.sidebar && ui.button("☰").on_hover_text("Show sessions").clicked() {
                self.sidebar = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(view.connected && self.focused.is_some(), |ui| {
                    ui.menu_button("Split", |ui| {
                        for (label, direction) in SPLITS {
                            if ui.button(label).clicked() {
                                if let Some(pane) = self.focused.clone() {
                                    self.send(Action::Split { pane, direction });
                                    self.focus_terminal = true;
                                }
                                ui.close();
                            }
                        }
                    });
                });
                if ui
                    .add_enabled(
                        view.connected && view.selected_session.is_some(),
                        egui::Button::new("+"),
                    )
                    .on_hover_text("New tab · Ctrl Shift T")
                    .clicked()
                {
                    self.send(Action::CreateWindow);
                    self.focus_terminal = true;
                }
                let width = ui.available_width();
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("tabs-scroll")
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for window in &view.windows {
                                        let selected =
                                            view.selected_window.as_deref() == Some(&window.id);
                                        if ui
                                            .add(
                                                item(&window.name, 12.0, selected)
                                                    .min_size(Vec2::new(72.0, 30.0)),
                                            )
                                            .on_hover_text(&window.name)
                                            .clicked()
                                        {
                                            self.send(Action::SelectWindow(window.id.clone()));
                                            self.reset_focus();
                                        }
                                    }
                                });
                            });
                    },
                );
            });
        });
    }

    fn empty(&mut self, ui: &mut egui::Ui, view: &View) {
        let height = ui.available_height();
        ui.add_space((height * 0.35).max(20.0));
        ui.vertical_centered(|ui| {
            if view.connected {
                ui.label(
                    RichText::new("A little room to work.")
                        .size(25.0)
                        .color(TEXT),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new("Start a session. Your programs stay on the server.")
                        .color(MUTED),
                );
                ui.add_space(24.0);
                if ui
                    .add(egui::Button::new("New session").min_size(Vec2::new(135.0, 38.0)))
                    .clicked()
                {
                    self.open_session_dialog();
                }
            } else {
                ui.label(
                    RichText::new("Your terminal, minus the noise.")
                        .size(25.0)
                        .color(TEXT),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Connect to a running Rex server.").color(MUTED));
                ui.add_space(22.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.endpoint)
                        .desired_width(390.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("unix:/path/to/rex/server.sock"),
                );
                ui.add_space(12.0);
                if ui
                    .add(egui::Button::new("Connect").min_size(Vec2::new(115.0, 36.0)))
                    .clicked()
                {
                    self.client = connect(ui.ctx(), self.endpoint.trim());
                    self.reset_focus();
                    self.notice = None;
                }
                ui.add_space(15.0);
                ui.add(
                    egui::Label::new(RichText::new(&view.status).size(12.0).color(MUTED)).wrap(),
                );
            }
        });
    }

    fn terminal_pane(&mut self, ui: &mut egui::Ui, pane: &Pane, rect: egui::Rect, split: bool) {
        let active = self.focused.as_deref() == Some(&pane.id);
        let mut terminal_rect = rect;
        if split {
            let header = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), PANE_HEADER));
            ui.painter().rect_filled(header, 0, SIDEBAR);
            ui.painter().text(
                header.left_center() + egui::vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &pane.title,
                FontId::proportional(11.0),
                if active { ACCENT } else { MUTED },
            );
            terminal_rect.min.y += PANE_HEADER;
        }
        let state = self.panes.entry(pane.id.clone()).or_default();
        // A selection keeps showing the frame it was made on, including on the
        // frame of the click that clears it.
        let selected = state.selection.as_ref().map(|s| s.frame.clone());
        let frame = selected.or_else(|| pane.frame.clone());
        let background = frame.as_ref().map_or(BACKGROUND, |f| rgb(f.bg));
        ui.painter().rect_filled(terminal_rect, 0, background);
        let grid = Grid::new(
            ui.ctx(),
            terminal_rect.shrink2(egui::vec2(12.0, 9.0)),
            self.font_size,
        );
        let id = ui.id().with(("terminal", &pane.id));
        let response = ui.interact(terminal_rect, id, egui::Sense::click_and_drag());
        if active && self.focus_terminal {
            response.request_focus();
            self.focus_terminal = false;
        }
        if active {
            self.terminal_keyboard |= response.has_focus();
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let mut actions = Vec::new();
        let scale = ui.ctx().pixels_per_point();
        let size = Size {
            cols: (grid.rect.width() / grid.cell.x).clamp(2.0, 1000.0) as u16,
            rows: (grid.rect.height() / grid.cell.y).clamp(1.0, 1000.0) as u16,
            cell_width: (grid.cell.x * scale).ceil() as u16,
            cell_height: (grid.cell.y * scale).ceil() as u16,
        };
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
            response.request_focus();
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
                        self.notice = Some(error);
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
                egui::Align2::CENTER_CENTER,
                &pane.status,
                FontId::proportional(12.0),
                MUTED,
            );
        }
        if split {
            let color = if active {
                BORDER.gamma_multiply(1.5)
            } else {
                BORDER
            };
            ui.painter()
                .rect_stroke(rect, 0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
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
                    self.focus_terminal = true;
                    ui.close();
                }
                ui.separator();
                for (label, direction) in SPLITS {
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
        let mut open = true;
        let mut create = false;
        egui::Window::new("New session")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Give this workspace a name.").color(MUTED));
                ui.add_space(12.0);
                let field = ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text("e.g. backend")
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus_name) {
                    field.request_focus();
                }
                ui.add_space(15.0);
                let enabled = !name.trim().is_empty();
                create = ui
                    .add_enabled(enabled, egui::Button::new("Create session"))
                    .clicked()
                    || (enabled && ctx.input(|i| i.key_pressed(Key::Enter)));
            });
        if create {
            let name = name.trim().to_owned();
            self.send(Action::CreateSession { name });
        }
        if create || !open || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.new_session = None;
            self.focus_terminal = true;
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
        self.terminal_keyboard = false;
        let view = self.client.snapshot();
        if self.previous_session != view.selected_session
            || self.previous_window != view.selected_window
        {
            self.previous_session.clone_from(&view.selected_session);
            self.previous_window.clone_from(&view.selected_window);
            self.reset_focus();
        }
        self.global_shortcuts(&ctx, &view);
        self.status_bar(root_ui, &view);
        if self.sidebar {
            self.sidebar(root_ui, &view);
        }
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(0))
            .show(root_ui, |ui| {
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        self.tabs(ui, &view);
                    });
                let Some(window) = view.current_window().filter(|w| !w.panes.is_empty()) else {
                    self.empty(ui, &view);
                    return;
                };
                if !window
                    .panes
                    .iter()
                    .any(|p| self.focused.as_ref() == Some(&p.id))
                {
                    self.focused = Some(window.panes[0].id.clone());
                    self.focus_terminal = true;
                }
                let area = ui.available_rect_before_wrap();
                for pane in &window.panes {
                    let min = area.min + area.size() * egui::vec2(pane.x, pane.y);
                    let size = area.size() * egui::vec2(pane.width, pane.height);
                    self.terminal_pane(
                        ui,
                        pane,
                        egui::Rect::from_min_size(min, size),
                        window.panes.len() > 1,
                    );
                }
            });
        self.session_dialog(&ctx);
        // The pane has explicit focus; dialogs and popups always keep their own input.
        if view.connected
            && self.new_session.is_none()
            && !egui::Popup::is_any_open(&ctx)
            && self.terminal_keyboard
            && let Some(pane) = self.focused.clone()
        {
            self.terminal_keys(&ctx, &pane);
        }
        self.panes.retain(|id, _| view.pane(id).is_some());
    }
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
                    let start = *pressed && self.tracking && self.response.rect.contains(*pos);
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
        let background = egui::Rect::from_min_size(cursor.min, galley.size()).expand(3.0);
        ui.painter().rect_filled(background, 2, SURFACE);
        ui.painter().galley(cursor.min, galley, TEXT);
    }
}

/// A session or tab button, quiet unless it is the selected one.
fn item(label: &str, size: f32, selected: bool) -> egui::Button<'_> {
    let (color, fill) = if selected {
        (TEXT, SURFACE)
    } else {
        (MUTED, Color32::TRANSPARENT)
    };
    egui::Button::new(RichText::new(label).size(size).color(color))
        .fill(fill)
        .stroke(Stroke::NONE)
}

fn endpoint_label(endpoint: &str) -> &str {
    if endpoint.starts_with("unix:") || endpoint.starts_with('/') {
        "LOCAL WORKSPACE"
    } else {
        endpoint
    }
}

fn theme(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = SIDEBAR;
    style.visuals.extreme_bg_color = BACKGROUND;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(53, 73, 59);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.window_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(43, 54, 46);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(51, 68, 56);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(11.0, 7.0);
    style.animation_time = 0.0;
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    // A terminal owns Ctrl+/- itself. Avoid a second, implicit zoom of the whole UI.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

fn mouse_button(button: egui::PointerButton) -> &'static str {
    match button {
        egui::PointerButton::Secondary => "right",
        egui::PointerButton::Middle => "middle",
        _ => "left",
    }
}
