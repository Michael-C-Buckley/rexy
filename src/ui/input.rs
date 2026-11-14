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
    /// An input method was last heard to be composing something.
    composing: bool,
    /// The Super keys that are down, left and right.
    supers: [bool; 2],
}

struct Held {
    pane: String,
    key: String,
    code: String,
    modifiers: u8,
}

impl Keyboard {
    /// Drop the news that nothing is being composed when nothing was. On Wayland
    /// it comes in answer to the input method being told where the cursor is, and
    /// that is told it after every frame that had news: each would bring on the
    /// next, and the window would draw without end.
    pub fn settle(&mut self, events: &mut Vec<Event>) {
        events.retain(|event| {
            let Event::Ime(ImeEvent::Preedit { text, .. }) = event else {
                return true;
            };
            let composing = !text.is_empty();
            std::mem::replace(&mut self.composing, composing) || composing
        });
    }

    /// Note which Super keys are down. egui counts Super among the modifiers only
    /// on a Mac, so it is told from its own presses and releases, whatever they
    /// were meant for.
    pub fn watch(&mut self, events: &[Event]) {
        for event in events {
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    ..
                } => match physical_key.unwrap_or(*key) {
                    Key::SuperLeft => self.supers[0] = *pressed,
                    Key::SuperRight => self.supers[1] = *pressed,
                    _ => {}
                },
                // Its release goes to whatever window has the keyboard by then.
                Event::WindowFocused(false) => self.supers = [false; 2],
                _ => {}
            }
        }
    }

    /// The modifiers held, as bits, Super among them.
    pub fn bits(&self, modifiers: Modifiers) -> u8 {
        let held = self.supers.contains(&true) || modifiers.mac_cmd;
        modifier_bits(modifiers) | (u8::from(held) * model::SUPER)
    }

    pub fn translate(&mut self, pane: &str, events: &[Event]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut used_text = HashSet::new();
        let key_action = |held: &Held, modifiers, pressed, repeat| Action::Key {
            pane: held.pane.clone(),
            key: held.key.clone(),
            code: held.code.clone(),
            modifiers,
            pressed,
            repeat,
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
                        actions.push(key_action(&held, self.bits(*modifiers), false, false));
                    }
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    repeat,
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
                    let bits = self.bits(*modifiers);
                    let chord = bits & (model::CTRL | model::SUPER) != 0;
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
                        modifiers: bits,
                    };
                    actions.push(key_action(&held, bits, true, *repeat));
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
                    let release =
                        |(_, held): (_, Held)| key_action(&held, held.modifiers, false, false);
                    actions.extend(released.map(release));
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
        } => !model::is_modifier(code),
        _ => false,
    }
}

/// All but Super, which `Keyboard::bits` adds.
fn modifier_bits(mods: Modifiers) -> u8 {
    (u8::from(mods.shift) * model::SHIFT)
        | (u8::from(mods.ctrl) * model::CTRL)
        | (u8::from(mods.alt) * model::ALT)
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
        Key::SuperLeft => "meta_left",
        Key::SuperRight => "meta_right",
        Key::IntlBackslash => "intl_backslash",
        _ if between(key, Key::Num0, Key::Num9) => return Some(format!("digit_{}", key.name())),
        _ if between(key, Key::A, Key::Z) => {
            return Some(format!("key_{}", key.name().to_lowercase()));
        }
        // The server knows none past this.
        _ if between(key, Key::F1, Key::F25) => return Some(key.name().to_lowercase()),
        _ => return None,
    };
    Some(code.into())
}

#[cfg(test)]
#[path = "../../tests/unit/ui_input.rs"]
mod tests;
