//! What each key does: the keymap of the server on show, as Rex resolved it from
//! this client's own keys and the account's `init.lua`, or without one this
//! client's own keys alone. Sequences and modes are run here, as Rex leaves
//! them to its clients.
use std::sync::Arc;

use serde_json::Value;

use super::commands;

/// The action of a key that a function on the server is bound to: the key is
/// sent there, as any key is.
pub const FORWARD: &str = "client.key.forward";

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// One key as Rex spells it, or the keys of a sequence in order.
    pub keys: Vec<String>,
    /// The mode it belongs to, if it is not for all the time.
    pub mode: Option<String>,
    pub name: String,
    pub args: Value,
}

struct Mode {
    name: String,
    /// The keys it does not bind do nothing while it is entered.
    exclusive: bool,
    /// Keys that do nothing while it is entered, whatever is bound to them outside it.
    blocked: Vec<Vec<String>>,
}

#[derive(Default)]
pub struct Keymap {
    /// In the order to try them.
    pub bindings: Vec<Binding>,
    modes: Vec<Mode>,
    /// The server's own actions: each by its name, its title if it is one to
    /// show, and whether holding its key does it again.
    pub served: Vec<(String, String, bool)>,
}

impl Keymap {
    /// Read a server's answer to `client.keymap`.
    pub fn parse(reply: &Value) -> Self {
        let text = |value: &Value| value.as_str().map(str::to_owned);
        let keys = |value: &Value| items(value).filter_map(text).collect::<Vec<_>>();
        let binding = |bound: &Value| {
            Some(Binding {
                keys: keys(&bound["keys"]),
                mode: text(&bound["mode"]),
                name: commands::named(bound["action"].as_str()?, &bound["args"]),
                args: bound["args"].clone(),
            })
        };
        let mode = |mode: &Value| {
            Some(Mode {
                name: text(&mode["name"])?,
                exclusive: mode["exclusive"] == true,
                blocked: items(&mode["blocked"]).map(keys).collect(),
            })
        };
        let served = |action: &Value| {
            let title = text(&action["title"]).unwrap_or_default();
            Some((text(&action["name"])?, title, action["repeats"] == true))
        };
        Self {
            bindings: items(&reply["bindings"]).filter_map(binding).collect(),
            modes: items(&reply["modes"]).filter_map(mode).collect(),
            served: items(&reply["actions"]).filter_map(served).collect(),
        }
    }

    /// This client's own keys, which are all there is until a server has answered.
    /// A server tries the later of one layer's bindings first, and so does this.
    pub fn own() -> Self {
        let bound = |(key, name): &(&str, &str)| Binding {
            keys: vec![(*key).to_owned()],
            mode: None,
            name: (*name).to_owned(),
            args: Value::Null,
        };
        Self {
            bindings: commands::KEYS.iter().rev().map(bound).collect(),
            ..Self::default()
        }
    }

    pub fn has_mode(&self, name: &str) -> bool {
        self.modes.iter().any(|mode| mode.name == name)
    }

    /// Whether holding the key of an action does it again.
    pub fn repeats(&self, name: &str) -> bool {
        let served = self.served.iter().find(|served| served.0 == name);
        let own = commands::command(name).map(|own| own.2);
        own.or(served.map(|served| served.2)) == Some(true)
    }

    /// The keys that do something, as they are shown: `Ctrl+Shift+D`, or nothing
    /// for what has none. Of several, those that would be tried first.
    pub fn shortcut(&self, name: &str) -> String {
        let word = |word: &str| match word.trim_matches(['[', ']']) {
            "pageup" => "PageUp".to_owned(),
            "pagedown" => "PageDown".to_owned(),
            word => {
                let first = word.chars().take(1).flat_map(char::to_uppercase);
                first.chain(word.chars().skip(1)).collect()
            }
        };
        let key = |key: &String| key.split('+').map(word).collect::<Vec<_>>().join("+");
        let mut bound = self.bindings.iter();
        let bound = bound.find(|bound| bound.name == name && bound.mode.is_none());
        bound.map_or_else(String::new, |bound| {
            bound.keys.iter().map(key).collect::<Vec<_>>().join(", ")
        })
    }

    /// What a control says it does under the pointer, with the keys that do the same.
    pub fn tip(&self, label: &str, name: &str) -> String {
        match self.shortcut(name).as_str() {
            "" => label.to_owned(),
            keys => format!("{label} ({keys})"),
        }
    }
}

/// What a key press comes to.
#[derive(Debug, PartialEq)]
pub enum Step<'a> {
    /// Nothing: it is the terminal's.
    Pass,
    /// A sequence may go on from it.
    Wait,
    /// Nothing, and it goes nowhere: a mode has it ignored, or it was Escape
    /// in the middle of a sequence.
    Void,
    /// The last mode entered is left.
    Leave,
    Run(&'a Binding),
}

/// What the last key pressed comes to. `pressed` ends with it, after the keys of
/// the sequence it may go on, each in the ways Rex spells it. `modes` are those
/// entered, the last of them the first to be asked. Of several bindings for a
/// key, the first whose action `can` be done is the one.
pub fn resolve<'a>(
    map: &'a Keymap,
    modes: &[(String, bool)],
    pressed: &[Vec<String>],
    can: impl Fn(&Binding) -> bool,
) -> Step<'a> {
    let escape = pressed
        .last()
        .is_some_and(|key| key.iter().any(|key| key == "escape"));
    if escape && pressed.len() > 1 {
        return Step::Void;
    }
    let fits = |keys: &[String]| {
        let spelled = |(pressed, key): (&Vec<String>, _)| pressed.contains(key);
        keys.len() >= pressed.len() && pressed.iter().zip(keys).all(spelled)
    };
    let first = |mode: Option<&str>| {
        let mut bound = map.bindings.iter();
        let bound = bound.find(|b| b.mode.as_deref() == mode && fits(&b.keys) && can(b))?;
        Some(match bound.keys.len() > pressed.len() {
            true => Step::Wait,
            false => Step::Run(bound),
        })
    };
    for (name, _) in modes.iter().rev() {
        if let Some(step) = first(Some(name)) {
            return step;
        }
        // Escape leaves a mode that has no other use for it.
        if escape {
            return Step::Leave;
        }
        let mode = map.modes.iter().find(|mode| &mode.name == name);
        if mode.is_some_and(|mode| mode.exclusive || mode.blocked.iter().any(|keys| fits(keys))) {
            return Step::Void;
        }
    }
    first(None).unwrap_or(Step::Pass)
}

/// Where the keyboard stands with the keymap in force.
pub struct Keys {
    /// The server's answer the keymap was read from, or none for this client's own.
    reply: Option<Arc<Value>>,
    pub map: Arc<Keymap>,
    /// The modes entered, each with whether one key is all it was entered for.
    pub modes: Vec<(String, bool)>,
    /// The keys pressed so far of a sequence, each in the ways Rex spells it.
    pub pending: Vec<Vec<String>>,
    /// The key last taken from the terminal, as it is spelled, and what holding
    /// it does again.
    pub held: Option<(String, Option<Binding>)>,
}

impl Default for Keys {
    fn default() -> Self {
        Self::of(None)
    }
}

impl Keys {
    fn of(reply: Option<&Arc<Value>>) -> Self {
        Self {
            reply: reply.cloned(),
            map: Arc::new(reply.map_or_else(Keymap::own, |reply| Keymap::parse(reply))),
            modes: Vec::new(),
            pending: Vec::new(),
            held: None,
        }
    }

    /// Go by a server's answer, or by none. Modes and a sequence under way belong
    /// to the keymap they began in, and end with it.
    pub fn follow(&mut self, reply: Option<&Arc<Value>>) {
        if reply.map(Arc::as_ptr) != self.reply.as_ref().map(Arc::as_ptr) {
            *self = Self::of(reply);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_keymap.rs"]
mod tests;
