//! The keys that are the window's own, and those that go to the terminal.
use eframe::egui::{self, Key, Modifiers};

use super::{App, DEFAULT_FONT_SIZE, close::doomed, input, pane::Scroll, style::Icon};
use crate::model::{Action, Direction, View};

/// The ways to split a pane, and the key that with Ctrl and Shift does each.
pub const SPLITS: [(Icon, &str, Key, Direction); 2] = [
    (Icon::SplitRight, "Split Right", Key::D, Direction::Right),
    (Icon::SplitDown, "Split Down", Key::E, Direction::Down),
];
/// The arrow keys, and the step each takes from one pane to the next.
const ARROWS: [(Key, [f32; 2]); 4] = [
    (Key::ArrowLeft, [-1.0, 0.0]),
    (Key::ArrowRight, [1.0, 0.0]),
    (Key::ArrowUp, [0.0, -1.0]),
    (Key::ArrowDown, [0.0, 1.0]),
];

impl App {
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context, view: &View) {
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let pressed = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        // With these modifiers and no others, which `pressed` lets by.
        let exactly = |modifiers: Modifiers, key| {
            let held = ctx.input(|i| i.modifiers.matches_exact(modifiers));
            held && pressed(modifiers, key)
        };
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
        if self.busy(ctx) {
            return;
        }
        if pressed(chord, Key::N) && view.connected {
            self.send(Action::CreateSession(None));
        }
        if pressed(chord, Key::T)
            && view.connected
            && let Some(session) = &view.selected_session
        {
            self.send(Action::CreateWindow(session.clone()));
        }
        if (pressed(chord, Key::C) || pressed(Modifiers::NONE, Key::Copy))
            && let Some(pane) = &self.focused
        {
            self.copy(ctx, pane);
        }
        if pressed(chord, Key::V) || pressed(Modifiers::NONE, Key::Paste) {
            paste();
        }
        // The tabs of the session on show, round and round. A chord is taken with
        // Shift held too, so those that need it are asked after first.
        let back = pressed(chord, Key::Tab) || pressed(Modifiers::CTRL, Key::PageUp);
        let on = pressed(Modifiers::CTRL, Key::Tab) || pressed(Modifiers::CTRL, Key::PageDown);
        let (tabs, shown) = (view.windows(), view.selected_window.as_ref());
        if on != back
            && let Some(session) = &view.selected_session
            && let Some(at) = tabs.iter().position(|tab| Some(&tab.id) == shown)
        {
            let to = (at + if on { 1 } else { tabs.len() - 1 }) % tabs.len();
            let (session, window) = (session.clone(), tabs[to].id.clone());
            self.send(Action::SelectWindow { session, window });
        }
        if let Some(window) = view.current_window()
            && let Some(pane) = self.focused.clone()
        {
            for (key, toward) in ARROWS {
                if pressed(chord, key)
                    && let Some(next) = window.beside(&pane, toward)
                {
                    let pane = next.id.clone();
                    self.focused = Some(pane.clone());
                    self.send(Action::FocusPane { pane });
                }
            }
            for (.., key, direction) in SPLITS {
                if pressed(chord, key) {
                    let pane = pane.clone();
                    self.send(Action::Split { pane, direction });
                }
            }
            if pressed(chord, Key::Enter) && window.split() {
                self.send(Action::Zoom { pane: pane.clone() });
            }
            // The keys that move through its history, which its program never
            // sees: unless that has the whole screen, which keeps no history, and
            // so has the keys.
            let shift = Modifiers::SHIFT;
            let scrolls = [
                (shift, Key::PageUp, Scroll::Pages(-1)),
                (shift, Key::PageDown, Scroll::Pages(1)),
                // Further than any history goes.
                (shift, Key::Home, Scroll::Lines(i32::MIN)),
                (shift, Key::End, Scroll::Bottom),
                (chord, Key::Z, Scroll::Prompts(-1)),
                (chord, Key::X, Scroll::Prompts(1)),
            ];
            let frame = view.pane(&pane).and_then(|pane| pane.frame.as_ref());
            if !frame.is_some_and(|frame| frame.alternate_screen) {
                for (modifiers, key, to) in scrolls {
                    if exactly(modifiers, key) {
                        self.scroll(&pane, to);
                    }
                }
            }
            // A key held down asks once: its repeats would answer for the user.
            // They are looked for first, since `pressed` takes the key's events away.
            let repeat = |event: &egui::Event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: Key::W,
                        repeat: true,
                        ..
                    }
                )
            };
            let held = ctx.input(|i| i.events.iter().any(repeat));
            if pressed(chord, Key::W) && !held {
                let spared = view.pane(&pane).is_some_and(|pane| pane.idle);
                let action = Action::ClosePane { pane: pane.clone() };
                self.close(doomed(&pane), spared, false, self.current, action);
            }
        }
        // Ctrl with Shift or Alt and these keys is for the terminal's programs. On
        // most keyboards a plus is typed with Shift, though.
        if exactly(Modifiers::CTRL, Key::Plus)
            || exactly(chord, Key::Plus)
            || exactly(Modifiers::CTRL, Key::Equals)
        {
            self.font_size = (self.font_size + 1.0).min(32.0);
        }
        if exactly(Modifiers::CTRL, Key::Minus) {
            self.font_size = (self.font_size - 1.0).max(9.0);
        }
        if exactly(Modifiers::CTRL, Key::Num0) {
            self.font_size = DEFAULT_FONT_SIZE;
        }
    }

    /// Send this frame's keyboard events to the focused pane.
    pub(super) fn terminal_keys(&mut self, ctx: &egui::Context, pane: &str) {
        // Escape leaves a selection instead of reaching the program.
        if self.selected(pane) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.deselect(pane);
        }
        let actions = ctx.input(|i| self.keyboard.translate(pane, &i.events));
        if actions.iter().any(input::clears_selection) {
            self.deselect(pane);
        }
        for action in actions {
            self.send(action);
        }
    }
}
