//! The row along the top that holds the tabs when there is no sidebar, and
//! the buttons both share.
use eframe::egui::{self, Align, Layout, Sense, UiBuilder, Vec2};

use super::{
    App,
    heard::dot,
    keys::SPLITS,
    style::{Icon, Item, TOOLBAR, icon_button, item},
};
use crate::model::{Action, View};

impl App {
    /// The buttons that split the focused pane, laid out from the right.
    pub(super) fn splits(&mut self, ui: &mut egui::Ui, view: &View) {
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
    pub(super) fn hidden(&mut self, ui: &mut egui::Ui, view: &View) {
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

    /// With the sidebar hidden, one row holds what it held: the sessions, behind a
    /// button, and the tabs of the one on show.
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, views: &[View]) {
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
}

/// A row of controls, such as the toolbar or a server's heading.
pub fn bar(ui: &mut egui::Ui, height: f32, inset: f32) -> egui::Ui {
    let size = Vec2::new(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let rect = rect.shrink2(egui::vec2(inset, 0.0));
    let layout = Layout::left_to_right(Align::Center);
    let mut bar = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    bar.spacing_mut().item_spacing.x = 2.0;
    bar
}
