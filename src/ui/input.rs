//! Translate egui events once, preserving physical key identity without typing twice.
use std::collections::{HashMap, HashSet};

use eframe::egui::{Event, ImeEvent, Key, Modifiers};

use crate::model::{self, Action};

#[derive(Default)]
pub struct Keyboard {
    held: HashMap<Key, Held>,
    pub preedit: String,
    modifiers: Modifiers,
    shifted_control: bool,
    paste_release: bool,
}

#[derive(Clone)]
struct Held {
    pane: String,
    key: String,
    code: String,
    modifiers: u8,
}

impl Held {
    fn into_key(self, pressed: bool) -> Action {
        Action::Key {
            pane: self.pane,
            key: self.key,
            code: self.code,
            modifiers: self.modifiers,
            pressed,
        }
    }
}

impl Keyboard {
    pub fn clear(&mut self) {
        self.held.clear();
        self.preedit.clear();
        self.shifted_control = false;
        self.paste_release = false;
    }

    fn set_modifiers(&mut self, modifiers: Modifiers) {
        if !modifiers.ctrl {
            self.shifted_control = false;
        } else if modifiers.shift {
            self.shifted_control = true;
        }
        self.modifiers = modifiers;
    }

    /// Remove only clipboard copies owned by the shell, using modifiers at the
    /// event's position. A complete chord can press and release within one frame.
    pub fn take_copy(&self, events: &mut Vec<Event>) -> bool {
        let mut modifiers = self.modifiers;
        let mut copy = false;
        events.retain(|event| {
            match event {
                Event::ModifiersChanged(current)
                | Event::Key {
                    modifiers: current, ..
                } => {
                    modifiers = *current;
                }
                Event::Copy if modifiers.shift || !modifiers.ctrl => {
                    copy = true;
                    return false;
                }
                _ => {}
            }
            true
        });
        copy
    }

    fn press(&mut self, identity: Key, held: Held, actions: &mut Vec<Action>) {
        actions.push(held.clone().into_key(true));
        self.held.insert(identity, held);
    }

    /// Press a key whose event egui-winit turned into a clipboard event.
    fn clipboard_key(&mut self, pane: &str, key: Key, actions: &mut Vec<Action>) {
        let name = key.name().to_lowercase();
        let held = Held {
            pane: pane.into(),
            code: format!("key_{name}"),
            key: name,
            modifiers: modifier_bits(self.modifiers),
        };
        self.press(key, held, actions);
    }

    pub fn translate(&mut self, pane: &str, events: &[Event]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut used_text = HashSet::new();
        for (index, event) in events.iter().enumerate() {
            match event {
                Event::ModifiersChanged(modifiers) => {
                    self.set_modifiers(*modifiers);
                }
                Event::Copy | Event::Cut => {
                    // egui-winit consumes these presses before producing Event::Key.
                    // Shift+Copy is handled by the shell; plain Ctrl+C/X belongs to the PTY.
                    let copy = matches!(event, Event::Copy);
                    if self.modifiers.ctrl && !(copy && self.modifiers.shift) {
                        self.clipboard_key(pane, if copy { Key::C } else { Key::X }, &mut actions);
                    }
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed: false,
                    modifiers,
                    ..
                } => {
                    self.set_modifiers(*modifiers);
                    let identity = physical_key.unwrap_or(*key);
                    if identity == Key::V
                        && !std::mem::take(&mut self.paste_release)
                        && !self.held.contains_key(&identity)
                        && modifiers.ctrl
                        && !modifiers.shift
                        && !modifiers.alt
                        && !modifiers.mac_cmd
                        && !self.shifted_control
                    {
                        // With an empty clipboard, egui-winit drops the Ctrl+V
                        // press entirely. Recover it on release. A chord which
                        // included Shift is ambiguous, so never replay it.
                        self.clipboard_key(pane, Key::V, &mut actions);
                    }
                    if let Some(held) = self.held.remove(&identity) {
                        let modifiers = modifier_bits(*modifiers);
                        actions.push(Held { modifiers, ..held }.into_key(false));
                    }
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    self.set_modifiers(*modifiers);
                    let identity = physical_key.unwrap_or(*key);
                    let Some(code) = key_code(identity) else {
                        continue;
                    };
                    // The text this press typed follows it, before any other key event.
                    let text = events
                        .iter()
                        .enumerate()
                        .skip(index + 1)
                        .take_while(|(_, e)| !matches!(e, Event::Key { .. }))
                        .find_map(|(i, e)| match e {
                            Event::Text(text) => Some((i, text)),
                            _ => None,
                        });
                    let chord = modifiers.ctrl || modifiers.mac_cmd;
                    let key = match text {
                        Some((i, text)) => {
                            used_text.insert(i);
                            if chord {
                                logical_text(*key).unwrap_or_default()
                            } else {
                                text.clone()
                            }
                        }
                        // During compose/dead-key input the final text arrives separately.
                        None if is_printable(*key) && !chord && !modifiers.alt => continue,
                        None => logical_text(*key).unwrap_or_default(),
                    };
                    let held = Held {
                        pane: pane.into(),
                        key,
                        code,
                        modifiers: modifier_bits(*modifiers),
                    };
                    self.press(identity, held, &mut actions);
                }
                Event::Text(text) if !used_text.contains(&index) && !text.is_empty() => {
                    actions.push(Action::Text {
                        pane: pane.into(),
                        text: text.clone(),
                    });
                }
                Event::Paste(text) => {
                    if self.modifiers.ctrl && !self.modifiers.shift {
                        self.clipboard_key(pane, Key::V, &mut actions);
                    } else {
                        self.paste_release = self.modifiers.ctrl;
                        actions.push(Action::Paste {
                            pane: pane.into(),
                            text: text.clone(),
                        });
                    }
                }
                Event::Ime(ImeEvent::Preedit { text, .. }) => {
                    self.preedit.clone_from(text);
                }
                Event::Ime(ImeEvent::Commit(text)) => {
                    self.preedit.clear();
                    if !text.is_empty() {
                        actions.push(Action::Text {
                            pane: pane.into(),
                            text: text.clone(),
                        });
                    }
                }
                Event::WindowFocused(false) => {
                    actions.extend(self.held.drain().map(|(_, held)| held.into_key(false)));
                    self.preedit.clear();
                }
                _ => {}
            }
        }
        actions
    }
}

/// Copy shortcuts still produce physical modifier presses/releases. They must
/// leave selection in place so the next Escape resumes output locally.
pub fn clears_selection(action: &Action) -> bool {
    match action {
        Action::Text { text, .. } | Action::Paste { text, .. } => !text.is_empty(),
        Action::Key {
            pressed: true,
            code,
            ..
        } => !matches!(
            code.as_str(),
            "shift_left"
                | "shift_right"
                | "control_left"
                | "control_right"
                | "alt_left"
                | "alt_right"
                | "super_left"
                | "super_right"
        ),
        _ => false,
    }
}

pub fn modifier_bits(mods: Modifiers) -> u8 {
    (u8::from(mods.shift) * model::SHIFT)
        | (u8::from(mods.ctrl) * model::CTRL)
        | (u8::from(mods.alt) * model::ALT)
        | (u8::from(mods.mac_cmd) * model::SUPER)
}

fn between(key: Key, first: Key, last: Key) -> bool {
    (first as u8..=last as u8).contains(&(key as u8))
}

/// Keys whose press is expected to arrive with text of its own.
fn is_printable(key: Key) -> bool {
    !matches!(key, Key::Enter | Key::Tab) && logical_text(key).is_some()
}

fn logical_text(key: Key) -> Option<String> {
    let text = match key {
        Key::Enter => "\n",
        Key::Tab => "\t",
        Key::Space => " ",
        Key::Colon => ":",
        Key::Comma => ",",
        Key::Backslash => "\\",
        Key::Slash => "/",
        Key::Pipe => "|",
        Key::Questionmark => "?",
        Key::Exclamationmark => "!",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::OpenCurlyBracket => "{",
        Key::CloseCurlyBracket => "}",
        Key::Backtick => "`",
        Key::Minus => "-",
        Key::Period => ".",
        Key::Plus => "+",
        Key::Equals => "=",
        Key::Semicolon => ";",
        Key::Quote => "'",
        _ if between(key, Key::Num0, Key::Z) => return Some(key.name().to_lowercase()),
        _ => return None,
    };
    Some(text.into())
}

fn key_code(key: Key) -> Option<String> {
    let code = match key {
        Key::ArrowDown => "arrow_down",
        Key::ArrowLeft => "arrow_left",
        Key::ArrowRight => "arrow_right",
        Key::ArrowUp => "arrow_up",
        Key::Escape => "escape",
        Key::Tab => "tab",
        Key::Backspace => "backspace",
        Key::Enter => "enter",
        Key::Space => "space",
        Key::Insert => "insert",
        Key::Delete => "delete",
        Key::Home => "home",
        Key::End => "end",
        Key::PageUp => "page_up",
        Key::PageDown => "page_down",
        Key::Colon | Key::Semicolon => "semicolon",
        Key::Comma => "comma",
        Key::Backslash | Key::Pipe => "backslash",
        Key::Slash | Key::Questionmark => "slash",
        Key::OpenBracket | Key::OpenCurlyBracket => "bracket_left",
        Key::CloseBracket | Key::CloseCurlyBracket => "bracket_right",
        Key::Backtick => "backquote",
        Key::Minus => "minus",
        Key::Period => "period",
        Key::Plus | Key::Equals => "equal",
        Key::Quote => "quote",
        Key::Exclamationmark => "digit_1",
        Key::ShiftLeft => "shift_left",
        Key::ShiftRight => "shift_right",
        Key::ControlLeft => "control_left",
        Key::ControlRight => "control_right",
        Key::AltLeft => "alt_left",
        Key::AltRight => "alt_right",
        Key::SuperLeft => "super_left",
        Key::SuperRight => "super_right",
        Key::IntlBackslash => "intl_backslash",
        _ if between(key, Key::Num0, Key::Num9) => return Some(format!("digit_{}", key.name())),
        _ if between(key, Key::A, Key::Z) => {
            return Some(format!("key_{}", key.name().to_lowercase()));
        }
        _ if between(key, Key::F1, Key::F35) => return Some(key.name().to_lowercase()),
        _ => return None,
    };
    Some(code.into())
}

#[cfg(test)]
#[path = "../../tests/unit/ui_input.rs"]
mod tests;
