//! Every server, its sessions and their tabs, down the side of the window.
use eframe::egui::{
    self, Align, FontId, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2, text::LayoutJob,
};

use super::{
    App,
    style::{DIM, Icon, SIDEBAR, SMALL, TOOLBAR, bold, icon_button},
    toolbar::bar,
};
use crate::model::{Action, View};

pub const SIDEBAR_WIDTH: f32 = 240.0;
/// The least and the most its edge can be dragged to.
const SIDEBAR_RANGE: [f32; 2] = [180.0, 480.0];

impl App {
    /// Every server, each with its sessions beneath it, and under each session its tabs.
    pub(super) fn sidebar(&mut self, root_ui: &mut egui::Ui, views: &[View]) {
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
    pub(super) fn edge(&mut self, root_ui: &egui::Ui) {
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
                ui.add(egui::Label::new(label.font(bold(SMALL))).truncate());
            });
        });
        create
    }
}
