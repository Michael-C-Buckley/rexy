//! What the pointer does over a terminal: a program's mouse, a selection, the wheel.
use std::sync::Arc;

use eframe::egui::{self, PointerButton};

use super::{
    input::Keyboard,
    pane::Scroll,
    render::{self, Grid, PaneView, Selection},
};
use crate::{model::Action, terminal::Frame};

/// The lines of history one notch of a wheel moves through.
const NOTCH: f32 = 3.0;

/// What a turn of the wheel over a pane is for.
#[derive(Clone, Copy, PartialEq)]
pub enum Wheel {
    /// The pane's history, a line at a time.
    History,
    /// A program that has the whole screen, where there is no history, and
    /// would have the wheel as arrow keys, one for a line (DEC mode 1007).
    Arrows,
    /// A program that tracks the mouse, which is told of each notch.
    Program,
    /// A program that has the whole screen and has not asked for arrow keys.
    Nothing,
}

impl Wheel {
    /// `tracking` says the program in the pane has the mouse, and Shift is not
    /// held to take it back.
    pub fn of(frame: &Frame, tracking: bool) -> Self {
        match (tracking, frame.alternate_screen, frame.alternate_scroll) {
            (true, ..) => Self::Program,
            (false, false, _) => Self::History,
            (false, true, true) => Self::Arrows,
            (false, true, false) => Self::Nothing,
        }
    }

    /// How far these events turned the wheel, in the steps this is taken by.
    /// Negative is up. What is left of a step stays in `rest`, in lines, for the
    /// next turn.
    fn steps(self, events: &[egui::Event], grid: Grid, rest: &mut f32) -> i32 {
        let lines = |event: &egui::Event| match event {
            egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                // A touchpad says how far it went.
                egui::MouseWheelUnit::Point => delta.y / grid.cell.y,
                // A wheel says how many notches, and under X11 so does a touchpad.
                egui::MouseWheelUnit::Line => delta.y * NOTCH,
                egui::MouseWheelUnit::Page => delta.y * grid.rect.height() / grid.cell.y,
            }),
            _ => None,
        };
        *rest += events.iter().filter_map(lines).sum::<f32>();
        let step = if self == Self::Program { NOTCH } else { 1.0 };
        let steps = (*rest / step).trunc();
        *rest -= steps * step;
        -steps as i32
    }
}

/// One pane's pointer input for one frame.
pub struct Pointer<'a> {
    pub ui: &'a egui::Ui,
    pub response: &'a egui::Response,
    pub events: &'a [egui::Event],
    pub grid: Grid,
    pub frame: &'a Arc<Frame>,
    pub pane: &'a str,
    pub keyboard: &'a Keyboard,
    /// The program in the pane has captured the mouse.
    pub tracking: bool,
}

impl Pointer<'_> {
    /// Forward buttons and motion to a program tracking the mouse. A button it saw
    /// pressed is followed through to its release even if tracking ends meanwhile.
    pub fn forward(&self, state: &mut PaneView, actions: &mut Vec<Action>) {
        let mut motion = None;
        for event in self.events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    // Hovered, not merely inside: whatever is drawn over the pane keeps its clicks.
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
                            modifiers: self.keyboard.bits(*modifiers),
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
                kind: "motion",
                x,
                y,
                button: state.mouse_button.map(mouse_button),
                modifiers: self.keyboard.bits(self.ui.input(|i| i.modifiers)),
            });
        }
    }

    /// Drag to select cells; double-click to select a word.
    pub fn select(&self, state: &mut PaneView) {
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
        // Made, it is the desktop's to paste with a middle click, in any program.
        if (response.drag_stopped_by(PointerButton::Primary) || response.double_clicked())
            && let Some(selection) = &state.selection
        {
            let made = egui::OutputCommand::TextSelectionSettled(selection.text());
            self.ui.ctx().send_cmd(made);
        }
    }

    /// Take the wheel's turn over this pane for what it is: keys or steps for a
    /// program, or a move through history, which is returned.
    pub fn wheel(
        &self,
        wheel: Wheel,
        state: &mut PaneView,
        actions: &mut Vec<Action>,
    ) -> Option<Scroll> {
        let pos = self.response.hover_pos()?;
        let steps = wheel.steps(self.events, self.grid, &mut state.wheel);
        let turn: Vec<Action> = match wheel {
            _ if steps == 0 => return None,
            Wheel::Nothing => return None,
            Wheel::History => return Some(Scroll::Lines(steps)),
            // A press and its release, which the server writes as the program
            // has asked to be sent its arrow keys.
            Wheel::Arrows => [true, false]
                .map(|pressed| Action::Key {
                    pane: self.pane.into(),
                    key: String::new(),
                    code: if steps < 0 { "arrow_up" } else { "arrow_down" }.into(),
                    modifiers: 0,
                    pressed,
                    repeat: false,
                })
                .into(),
            // The server takes each for one step, however far it says it went.
            Wheel::Program => {
                let (x, y) = self.grid.cell_at(pos, self.frame);
                let (pane, lines) = (self.pane.into(), steps.signum());
                vec![Action::Wheel { pane, x, y, lines }]
            }
        };
        state.selection = None;
        actions.extend(std::iter::repeat_n(turn, steps.unsigned_abs() as usize).flatten());
        None
    }
}

fn mouse_button(button: egui::PointerButton) -> &'static str {
    match button {
        egui::PointerButton::Secondary => "right",
        egui::PointerButton::Middle => "middle",
        _ => "left",
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_pointer.rs"]
mod tests;
