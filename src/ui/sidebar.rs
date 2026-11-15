//! Every server, its sessions and their tabs, down the side of the window.
use eframe::egui::{
    self, Align, FontId, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2, text::LayoutJob,
};

use super::{
    App,
    frame::titlebar,
    style::{BODY, BORDER, DIM, Icon, ROW, SMALL, TOOLBAR, bold, icon_button, search},
    toolbar::bar,
};
use crate::model::{Action, View};

pub const SIDEBAR_WIDTH: f32 = 240.0;
/// The least and the most its edge can be dragged to.
const SIDEBAR_RANGE: [f32; 2] = [180.0, 480.0];

impl App {
    /// Every server, each with its sessions beneath it, and under each session its tabs.
    /// It lies on the window's own ground.
    pub(super) fn sidebar(&mut self, root_ui: &mut egui::Ui, views: &[View]) {
        let rows = egui::Frame::new().inner_margin(Margin { top: 0, ..Margin::same(8) });
        // No wider than leaves the terminal half the window, however it was left.
        let [least, most] = SIDEBAR_RANGE;
        let most = (root_ui.available_width() / 2.0).clamp(least, most);
        let width = &mut self.remembered.kept.sidebar_width;
        *width = width.clamp(least, most);
        egui::Panel::left("sessions")
            .exact_size(*width)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new())
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // The button that puts the tabs along the top, where the toolbar
                // has it then, and the way into the palette for the pointer.
                let mut top = titlebar(ui, TOOLBAR, 8.0);
                let (tip, toggle) = ("Tabs along the top", "client.sidebar.toggle");
                self.button(&mut top, &views[self.current], Icon::Sidebar, tip, toggle);
                top.add_space(2.0);
                top.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.watch(ui, views);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let palette = "session.switch";
                        let tip = "Search sessions and tabs";
                        let field = search(ui, &self.keys.map.shortcut(palette));
                        if field.on_hover_text(tip).clicked() {
                            self.todo.push((palette, None));
                        }
                    });
                });
                let list = egui::ScrollArea::vertical().auto_shrink(false);
                list.show(ui, |ui| {
                    rows.show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        if self.activity {
                            return self.agents(ui, views);
                        }
                        for (server, view) in views.iter().enumerate() {
                            // A rule parts it from the server before. Those with
                            // nothing under their names stand together without one.
                            let bare = |view: &View| view.connected && view.sessions.is_empty();
                            match server.checked_sub(1) {
                                Some(before) if bare(&views[before]) && bare(view) => {}
                                Some(_) => rule(ui),
                                None => {}
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
    pub(super) fn edge(&mut self, root_ui: &egui::Ui) {
        let x = root_ui.max_rect().left() + self.remembered.kept.sidebar_width;
        let height = root_ui.max_rect().y_range();
        let grab = Rect::from_x_y_ranges(x - 4.0..=x + 4.0, height);
        // It takes clicks too, or a press there would be the terminal's to drag.
        let sense = Sense::click_and_drag();
        let edge = root_ui.interact(grab, egui::Id::unique("sidebar edge"), sense);
        if edge.hovered() || edge.dragged() {
            root_ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            root_ui.painter().vline(x, height, Stroke::new(1.0, DIM));
        }
        // The next drawing holds it to its least and its most.
        if let Some(pointer) = edge.interact_pointer_pos().filter(|_| edge.dragged()) {
            self.remembered.kept.sidebar_width = pointer.x - root_ui.max_rect().left();
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
                let mut status = LayoutJob::simple(view.trouble(), font, DIM, 0.0);
                status.wrap.max_rows = 3;
                ui.add(egui::Label::new(status).wrap()).on_hover_text(view.trouble());
            });
        }
        let mut sessions = Vec::new();
        for (index, session) in view.sessions.iter().enumerate() {
            let top = ui.cursor().top();
            let shown =
                server == self.current && view.session().is_some_and(|s| s.id == session.id);
            self.session(ui, server, &view.sessions, index, shown);
            // A session's one tab is the session's own row. Several are listed
            // under its name.
            if session.tabs.len() != 1 {
                // The tab on show, if it is one of this session's.
                let shown = view.selected_tab.as_deref().filter(|_| shown);
                let size = Vec2::new(ui.available_width(), ROW);
                self.tabs(ui, server, session, shown, size);
                ui.add_space(6.0);
            }
            let space = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
            sessions.push((&session.id, space));
        }
        self.reorder(ui, server, &sessions);
    }

    /// A server's name, beside a button that tries it again if it is not connected.
    /// True if its other button, for a new session, was pressed.
    pub(super) fn heading(&mut self, ui: &mut egui::Ui, server: usize, view: &View) -> bool {
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
                // One with nothing on it steps back.
                let label = match view.connected && view.sessions.is_empty() {
                    true => label.font(FontId::proportional(BODY)).color(DIM),
                    false => label.font(bold(BODY)),
                };
                ui.add(egui::Label::new(label).truncate());
            });
        });
        create
    }
}

/// A faint line across the sidebar, as wide as its rows, with the room it
/// keeps from what it parts.
fn rule(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let across = line.shrink2(egui::vec2(4.0, 0.0)).x_range();
    ui.painter().hline(across, line.center().y, Stroke::new(1.0, BORDER));
    ui.add_space(4.0);
}
