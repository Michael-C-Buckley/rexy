//! The borders between the panes of a split tab, which drag to resize them.
use eframe::egui::{self, CursorIcon, Rect, Sense};

use super::{
    App,
    style::{eased, tints},
};
use crate::model::{Action, Border, Float, Tab};

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
    let id = ui.scope_id().with(("border", &border.before, &border.after));
    let grab = ui.interact(line(at).expand(4.0), id, Sense::click_and_drag());
    let pointer = grab.interact_pointer_pos().filter(|_| grab.dragged());
    let ratio = pointer.map(|pointer| (pointer - room.min)[axis] / room.size()[axis]);
    let ratio = ratio.map(|ratio| ratio.clamp(Border::LEAST, Border::MOST));
    let held = grab.hovered() || grab.dragged();
    if held {
        ui.ctx().set_cursor_icon(match border.upright {
            true => CursorIcon::ResizeHorizontal,
            false => CursorIcon::ResizeVertical,
        });
    }
    // A short bar in the gutter says the border is there to be dragged. It
    // comes and goes over a moment, and is drawn where the pointer has the
    // border, which the server makes so a moment later.
    let lit = eased(ui, id.with("lit"), held);
    if lit > 0.0 {
        let line = line(ratio.map_or(at, |ratio| room.min[axis] + room.size()[axis] * ratio));
        let along = egui::vec2(10.0, 10.0).min(line.size() / 4.0);
        let bar =
            line.shrink2(along * egui::vec2(f32::from(!border.upright), f32::from(border.upright)));
        let color = tints(ui.ctx()).accent.gamma_multiply(lit);
        ui.painter().rect_filled(bar.expand(1.5), 1.5, color);
    }
    if grab.double_clicked() {
        return Some(0.5);
    }
    // Asked only when the pointer has moved.
    ratio.filter(|_| grab.drag_delta()[axis] != 0.0)
}

impl App {
    /// The borders of the tab on show, which fills `area`: those of its
    /// layout, or of a `layer` that floats over it. They are asked for after
    /// the layer's panes, or the terminals either side would have the pointer first.
    pub(super) fn borders(&mut self, ui: &egui::Ui, tab: &Tab, area: Rect, layer: Option<&Float>) {
        let layer = layer.map(|layer| &layer.id);
        for border in tab.borders.iter().filter(|border| border.float.as_ref() == layer) {
            if let Some(ratio) = drag(ui, border, area) {
                self.send(Action::Divide(vec![border.moved(ratio)]));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_borders.rs"]
mod tests;
