//! Translate egui events once, preserving physical key identity without typing twice.
//! Every key reaches the terminal as itself: the window is told not to turn Ctrl+C,
//! X and V into clipboard events.
use std::collections::{HashMap, HashSet};

use eframe::egui::{Event, ImeEvent, Key, Modifiers};

use crate::model::{self, Action};

#[derive(Default)]
pub struct Keyboard {
    held: HashMap<Key, Held>,
    pub preedit: String,
}

struct Held {
    pane: String,
    key: String,
    code: String,
    modifiers: u8,
}

impl Keyboard {
    pub fn translate(&mut self, pane: &str, events: &[Event]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut used_text = HashSet::new();
        let key_action = |held: &Held, modifiers, pressed| Action::Key {
            pane: held.pane.clone(),
            key: held.key.clone(),
            code: held.code.clone(),
            modifiers,
            pressed,
        };
        for (index, event) in events.iter().enumerate() {
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed: false,
                    modifiers,
                    ..
                } => {
                    if let Some(held) = self.held.remove(&physical_key.unwrap_or(*key)) {
                        actions.push(key_action(&held, modifier_bits(*modifiers), false));
                    }
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
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
                    actions.push(key_action(&held, held.modifiers, true));
                    self.held.insert(identity, held);
                }
                Event::Text(text) if !used_text.contains(&index) && !text.is_empty() => {
                    actions.push(Action::Text {
                        pane: pane.into(),
                        text: text.clone(),
                    });
                }
                Event::Paste(text) => actions.push(Action::Paste {
                    pane: pane.into(),
                    text: text.clone(),
                }),
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
                    let released = self.held.drain();
                    actions
                        .extend(released.map(|(_, held)| key_action(&held, held.modifiers, false)));
                    self.preedit.clear();
                }
                _ => {}
            }
        }
        actions
    }
}

/// A copy chord still presses and releases its modifiers. Those must leave the
/// selection in place, so that the copy has something to copy.
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
