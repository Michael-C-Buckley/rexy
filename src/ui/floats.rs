//! The layers that float over a tab's layout: the ground each has under its
//! panes, and how the pointer moves one and gives it another size.
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CursorIcon, Rect, ResizeDirection, Sense, Vec2};

use super::{App, frame, style::CARD_RADIUS};
use crate::model::{Action, Float, Tab, View};

/// The command that goes to the floating layers of the tab on show, bringing
/// back those put away: or from one of them puts them away, or with none
/// makes one.
pub const TOGGLE: &str = "client.float.toggle";
/// The command that makes another.
pub const NEW: &str = "client.float.new";
/// The command that floats a pane of the layout, or puts one that floats back.
pub const FLOAT: &str = "client.pane.float";

/// How far past a layer's cards its edges take the pointer, and how far along
/// an edge from its end a corner does.
const REACH: f32 = 5.0;
const CORNER: f32 = 14.0;
/// The least one is made by the pointer, each way, in points.
const LEAST: Vec2 = Vec2::new(200.0, 120.0);
/// How long at most one let go of is kept where it was left, for the server
/// to have it there: which a server far away has after two answers.
const SETTLING: Duration = Duration::from_secs(3);

/// A layer the pointer has hold of, as it was when taken: all of it, to move,
/// or the `sides` of it that go with the pointer, as `Float::held` has them.
pub struct Grip {
    float: Float,
    sides: Option<[i8; 2]>,
    /// Where it was last asked to go, which is where it is drawn.
    sent: [f32; 4],
    /// When the pointer let go of it.
    left: Option<Instant>,
}

impl App {
    /// Take hold of a layer, where the pointer has begun to drag something of it.
    pub(super) fn grip(&mut self, by: &egui::Response, float: &Float, sides: Option<[i8; 2]>) {
        if by.drag_started() {
            // One let go of a moment ago is where it was left, whether or not
            // the server has it there yet.
            let left = self.grip.as_ref().filter(|grip| grip.float.id == float.id);
            let bounds = left.map_or(float.bounds, |grip| grip.sent);
            let float = Float { bounds, ..float.clone() };
            self.grip = Some(Grip { float, sides, sent: bounds, left: None });
        }
    }

    /// What lies under the panes of a layer of `tab` whose cards fill `rect`:
    /// a shadow on what it floats over, the window's `ground` between them,
    /// and around them the edges and corners that drag to give it another
    /// size. It is asked for before its panes, which have the pointer first
    /// where they are.
    pub(super) fn float(
        &mut self,
        ui: &egui::Ui,
        tab: &Tab,
        float: &Float,
        rect: Rect,
        ground: Color32,
    ) {
        ui.painter().add(ui.visuals().popup_shadow.as_shape(rect, CARD_RADIUS));
        ui.painter().rect_filled(rect, CARD_RADIUS, ground);
        // Nothing under it has the pointer through it.
        let id = ui.scope_id().with(("float", &float.id));
        ui.interact(rect, id, Sense::click_and_drag());
        let mut taken = false;
        for (direction, hold) in frame::handles(rect.expand(REACH), REACH, REACH + CORNER) {
            let held = ui.interact(hold, id.with(direction as u8), Sense::drag());
            if held.hovered() || held.dragged() {
                ui.ctx().set_cursor_icon(frame::cursor(direction));
            }
            use ResizeDirection::*;
            let sides = match direction {
                North => [0, -1],
                South => [0, 1],
                West => [-1, 0],
                East => [1, 0],
                NorthWest => [-1, -1],
                NorthEast => [1, -1],
                SouthWest => [-1, 1],
                SouthEast => [1, 1],
            };
            self.grip(&held, float, Some(sides));
            taken |= held.drag_started();
        }
        // Taken hold of by one of those, it comes to the front with the
        // keyboard, as it does by a header.
        if taken && let Some(pane) = tab.fore(float) {
            self.focus(pane.id.clone());
        }
    }

    /// Have the layer the pointer has hold of go where the pointer has taken
    /// it, in the tab that fills `area`. It is asked of the server as the
    /// pointer moves, and drawn there without waiting to be told: so what is
    /// given is the layer, where the server has it, and where it is drawn. Let
    /// go of, it is drawn there until the server has it there too.
    pub(super) fn held<'a>(
        &mut self,
        ui: &egui::Ui,
        view: &View,
        tab: &'a Tab,
        area: Rect,
    ) -> Option<(&'a str, [Rect; 2])> {
        let grip = self.grip.as_mut()?;
        // One that has closed, or gone out of sight, is held no more.
        let Some(float) = tab.floating().find(|float| float.id == grip.float.id) else {
            self.grip = None;
            return None;
        };
        let dragged =
            ui.input(|i| i.pointer.total_drag_delta().filter(|_| i.pointer.primary_down()));
        if let Some(by) = dragged.filter(|_| grip.left.is_none()) {
            if grip.sides.is_none() {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            let (by, least) = (by / area.size(), LEAST / area.size());
            let bounds = grip.float.held(grip.sides, by.into(), least.into());
            if bounds != grip.sent
                && let Some(session) = view.selected_session.clone()
            {
                grip.sent = bounds;
                let layer = grip.float.id.clone();
                self.send(Action::Place { session, layer, bounds });
            }
        } else {
            let near = |(a, b): (&f32, &f32)| (a - b).abs() < 1e-4;
            let there = float.bounds.iter().zip(&grip.sent).all(near);
            let since = grip.left.get_or_insert_with(Instant::now).elapsed();
            if there || since > SETTLING {
                self.grip = None;
                return None;
            }
            ui.ctx().request_repaint_after(SETTLING - since);
        }
        let rect = |[x, y, width, height]: [f32; 4]| {
            Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
        };
        Some((&float.id, [rect(float.bounds), rect(self.grip.as_ref()?.sent)]))
    }
}
