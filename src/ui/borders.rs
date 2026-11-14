//! The borders between the panes of a split tab, which drag to resize them.
use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke};

use super::{App, style::DIM};
use crate::model::{Action, Border, Window};

/// The least of the room two sides share that a drag leaves either of them.
const LEAST: f32 = 0.05;

/// A border of the tab that fills `area`, and what the pointer asks of it, if
/// anything: the share of its room that the side before it is to have. A drag
/// puts the border where the pointer is, and a double click in the middle.
fn drag(ui: &egui::Ui, border: &Border, area: Rect) -> Option<f32> {
    let [x, y, width, height] = border.room;
    let room = Rect::from_min_size(
        area.min + area.size() * egui::vec2(x, y),
        area.size() * egui::vec2(width, height),
    );
    // Along the way it moves, and across it.
    let axis = usize::from(!border.upright);
    let at = area.min[axis] + area.size()[axis] * border.at;
    let line = |at: f32| match border.upright {
        true => Rect::from_x_y_ranges(at..=at, room.y_range()),
        false => Rect::from_x_y_ranges(room.x_range(), at..=at),
    };
    let id = ui
        .scope_id()
        .with(("border", &border.before, &border.after));
    let grab = ui.interact(line(at).expand(4.0), id, Sense::click_and_drag());
    let pointer = grab.interact_pointer_pos().filter(|_| grab.dragged());
    let ratio = pointer.map(|pointer| (pointer - room.min)[axis] / room.size()[axis]);
    let ratio = ratio.map(|ratio| ratio.clamp(LEAST, 1.0 - LEAST));
    if grab.hovered() || grab.dragged() {
        ui.ctx().set_cursor_icon(match border.upright {
            true => CursorIcon::ResizeHorizontal,
            false => CursorIcon::ResizeVertical,
        });
        // Drawn where the pointer has it, which the server makes so a moment later.
        let line = line(ratio.map_or(at, |ratio| room.min[axis] + room.size()[axis] * ratio));
        let stroke = Stroke::new(1.0, DIM);
        ui.painter().line_segment([line.min, line.max], stroke);
    }
    if grab.double_clicked() {
        return Some(0.5);
    }
    // Asked only when the pointer has moved.
    ratio.filter(|_| grab.drag_delta()[axis] != 0.0)
}

impl App {
    /// The borders of the tab on show, which fills `area`. They are asked for
    /// after the panes, or the terminals either side would have the pointer first.
    pub(super) fn borders(&mut self, ui: &egui::Ui, window: &Window, area: Rect) {
        for border in &window.borders {
            if let Some(ratio) = drag(ui, border, area) {
                self.send(Action::Divide {
                    before: border.before.clone(),
                    after: border.after.clone(),
                    upright: border.upright,
                    ratio,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_borders.rs"]
mod tests;
