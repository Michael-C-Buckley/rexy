//! The client's own actions, under the names Rex gives them. One table says what
//! each is called and which keys it has until a server says otherwise. Keys, menus,
//! the palette and other clients' requests all ask `plan` what doing one comes to.
use eframe::egui::{self, Popup};
use serde_json::{Value, json};

use super::{
    App, DEFAULT_FONT_SIZE, FONT_SIZES,
    close::doomed,
    input,
    keymap::Keys,
    palette,
    rows::{Choice, Target},
    style::{Icon, icon_button},
};
use crate::{
    model::{Action, Direction, Edit, View, Window},
    terminal::{Request, Scroll},
};

/// A command: the name and the title that Rex's action reference gives it, and
/// whether its key, held down, does it again.
pub type Command = (&'static str, &'static str, bool);
const ONCE: bool = false;
const HELD: bool = true;

/// Rex has no name for the two scrolls to an end. Theirs are this client's own.
pub const COMMANDS: &[Command] = &[
    ("session.new", "New Session", ONCE),
    ("session.switch", "Change Session…", ONCE),
    ("session.select", "Go to Session", ONCE),
    ("session.next", "Next Session", HELD),
    ("session.previous", "Previous Session", HELD),
    ("session.rename", "Rename Session", ONCE),
    // Its sessions are read again by connecting again.
    ("session.refresh", "Reconnect", ONCE),
    ("client.tab.new", "New Tab", ONCE),
    ("client.tab.close", "Close Tab", ONCE),
    ("client.tab.rename", "Rename Tab", ONCE),
    ("client.tab.next", "Show Next Tab", HELD),
    ("client.tab.previous", "Show Previous Tab", HELD),
    ("client.tab.goto", "Go to Tab", ONCE),
    ("client.tab.move.backward", "Move Tab Backward", ONCE),
    ("client.tab.move.forward", "Move Tab Forward", ONCE),
    ("pane.split.right", "Split Pane Right", ONCE),
    ("pane.split.down", "Split Pane Down", ONCE),
    ("pane.focus.left", "Focus Pane Left", HELD),
    ("pane.focus.right", "Focus Pane Right", HELD),
    ("pane.focus.up", "Focus Pane Up", HELD),
    ("pane.focus.down", "Focus Pane Down", HELD),
    ("pane.focus_next", "Focus Next Pane", HELD),
    ("pane.focus_previous", "Focus Previous Pane", HELD),
    ("pane.resize.left", "Resize Pane Left", HELD),
    ("pane.resize.right", "Resize Pane Right", HELD),
    ("pane.resize.up", "Resize Pane Up", HELD),
    ("pane.resize.down", "Resize Pane Down", HELD),
    ("pane.balance", "Balance Panes", ONCE),
    ("pane.zoom", "Zoom Pane", ONCE),
    ("pane.close", "Close Pane", ONCE),
    ("pane.copy", "Copy from Pane", ONCE),
    ("pane.paste", "Paste into Pane", ONCE),
    ("pane.select_all", "Select All in Pane", ONCE),
    ("pane.send_key", "Send Key", ONCE),
    ("client.prompt.previous", "Jump to Previous Prompt", HELD),
    ("client.prompt.next", "Jump to Next Prompt", HELD),
    ("client.find.open", "Find…", ONCE),
    ("client.find.next", "Find Next", HELD),
    ("client.find.previous", "Find Previous", HELD),
    ("client.scroll.page_up", "Scroll Page Up", HELD),
    ("client.scroll.page_down", "Scroll Page Down", HELD),
    ("client.scroll.top", "Scroll to Top", ONCE),
    ("client.scroll.bottom", "Scroll to Bottom", ONCE),
    ("client.font.increase", "Increase Font Size", HELD),
    ("client.font.decrease", "Decrease Font Size", HELD),
    ("client.font.reset", "Reset Font Size", ONCE),
    ("client.sidebar.toggle", "Toggle Sidebar", ONCE),
    (
        "client.sidebar.activity.toggle",
        "Show What Is at Work",
        ONCE,
    ),
    ("client.palette.toggle", "Command Palette…", ONCE),
    ("client.mode.enter", "Enter Mode", ONCE),
    ("client.mode.exit", "Exit Mode", ONCE),
    ("client.mode.exit_all", "Exit All Modes", ONCE),
];

/// The actions Rex also knows by one name for every direction, which is then
/// among their arguments.
const DIRECTED: [Command; 4] = [
    ("pane.split", "Split Pane", ONCE),
    ("pane.focus", "Focus Pane", ONCE),
    ("pane.resize", "Resize Pane", HELD),
    ("client.tab.move", "Move Tab", ONCE),
];

/// The keys this client has for them until a server says otherwise, as Rex
/// spells keys. They are a Linux keyboard's: Ctrl alone belongs to the terminal's
/// programs. Of several for one action the last is tried first, and is the one shown.
pub(super) const KEYS: &[(&str, &str)] = &[
    ("ctrl+shift+n", "session.new"),
    ("ctrl+k", "session.switch"),
    ("ctrl+shift+t", "client.tab.new"),
    ("ctrl+pagedown", "client.tab.next"),
    ("ctrl+tab", "client.tab.next"),
    ("ctrl+pageup", "client.tab.previous"),
    ("ctrl+shift+tab", "client.tab.previous"),
    ("ctrl+shift+pageup", "client.tab.move.backward"),
    ("ctrl+shift+pagedown", "client.tab.move.forward"),
    ("ctrl+shift+d", "pane.split.right"),
    ("ctrl+alt+d", "pane.split.down"),
    ("ctrl+alt+left", "pane.focus.left"),
    ("ctrl+alt+right", "pane.focus.right"),
    ("ctrl+alt+up", "pane.focus.up"),
    ("ctrl+alt+down", "pane.focus.down"),
    ("ctrl+alt+shift+left", "pane.resize.left"),
    ("ctrl+alt+shift+right", "pane.resize.right"),
    ("ctrl+alt+shift+up", "pane.resize.up"),
    ("ctrl+alt+shift+down", "pane.resize.down"),
    ("ctrl+alt+=", "pane.balance"),
    ("ctrl+shift+enter", "pane.zoom"),
    ("ctrl+shift+w", "pane.close"),
    // The keys some keyboards have for these. Rex has no spelling for them, and
    // keeps this one as it comes.
    ("[Copy]", "pane.copy"),
    ("[Paste]", "pane.paste"),
    ("ctrl+shift+c", "pane.copy"),
    ("ctrl+shift+v", "pane.paste"),
    ("ctrl+shift+f", "client.find.open"),
    ("ctrl+shift+up", "client.prompt.previous"),
    ("ctrl+shift+down", "client.prompt.next"),
    ("shift+pageup", "client.scroll.page_up"),
    ("shift+pagedown", "client.scroll.page_down"),
    ("shift+home", "client.scroll.top"),
    ("shift+end", "client.scroll.bottom"),
    // On most keyboards a plus is typed with Shift.
    ("ctrl++", "client.font.increase"),
    ("ctrl+shift+=", "client.font.increase"),
    ("ctrl+=", "client.font.increase"),
    ("ctrl+-", "client.font.decrease"),
    ("ctrl+0", "client.font.reset"),
    ("ctrl+shift+b", "client.sidebar.toggle"),
    ("ctrl+shift+p", "client.palette.toggle"),
];

/// The name an action goes by in the table.
pub fn named(name: &str, args: &Value) -> String {
    let directed = DIRECTED.iter().any(|directed| directed.0 == name);
    match args["direction"].as_str().filter(|_| directed) {
        Some(direction) => format!("{name}.{direction}"),
        None => name.to_owned(),
    }
}

/// A command by its name, if it is one.
pub fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.0 == name)
}

/// What menus and the palette call a command.
pub fn title(name: &str) -> &'static str {
    command(name).map_or("", |command| command.1)
}

/// The way a command goes that is named for one: a step along x or y.
fn toward(name: &str) -> Option<[f32; 2]> {
    Some(match name.rsplit_once('.')?.1 {
        "left" => [-1.0, 0.0],
        "right" => [1.0, 0.0],
        "up" => [0.0, -1.0],
        "down" => [0.0, 1.0],
        _ => return None,
    })
}

/// The move through a pane's history that a command makes, if it makes one.
fn scroll(name: &str) -> Option<Scroll> {
    Some(match name {
        "client.scroll.page_up" => Scroll::Pages(-1),
        "client.scroll.page_down" => Scroll::Pages(1),
        // Further than any history goes.
        "client.scroll.top" => Scroll::Lines(i32::MIN),
        "client.scroll.bottom" => Scroll::Bottom,
        "client.prompt.previous" => Scroll::Prompts(-1),
        "client.prompt.next" => Scroll::Prompts(1),
        _ => return None,
    })
}

/// What a server's keymap is asked with: what this client can do itself, and
/// the keys it does it by unless the account's `init.lua` says otherwise.
pub fn request() -> Value {
    let action = |(name, title, repeats): &Command| {
        let kind = "client";
        json!({"name": name, "kind": kind, "title": title, "repeats": repeats})
    };
    let actions: Vec<Value> = COMMANDS.iter().chain(&DIRECTED).map(action).collect();
    let bound = |(key, name): &(&str, &str)| json!({"keys": [key], "action": name});
    let bindings: Vec<Value> = KEYS.iter().map(bound).collect();
    json!({"actions": actions, "defaults": [{"name": "default", "bindings": bindings}]})
}

/// What doing a command comes to.
pub(super) enum Deed {
    /// Asking it of the server.
    Ask(Action),
    /// Taking the keyboard to a pane.
    Focus(String),
    Font(f32),
    Sidebar,
    /// Turning the sidebar to the list of what is at work, or back.
    Agents,
    Palette,
    /// Asking something of a pane's own terminal, which no server is told of:
    /// about what is selected or looked for in it, or to move through its history.
    Tell(String, Request),
    /// Opening the field that finds text in a pane.
    Find(String),
    /// Pasting what the desktop's clipboard holds into a pane.
    Paste(String),
    /// Closing a pane by an action, unless programs may be lost by it and this
    /// is the first asking: by its name, and whether nothing would be lost.
    Close(egui::Id, bool, Action),
    /// Closing a tab of a session, likewise.
    CloseTab(String, String, bool),
    /// Opening the field that renames a session, or the tab of it that is
    /// given, on the name it goes by.
    Rename(String, Option<String>, String),
    /// Entering a mode, for one key only or until it is left.
    Enter(String, bool),
    /// Leaving the last mode entered, or them all.
    Exit(bool),
}

/// How the window stands, as far as what can be done in it goes.
pub(super) struct Standing<'a> {
    /// What the server a command is for has.
    pub view: &'a View,
    /// The pane with the keyboard.
    pub focused: Option<&'a str>,
    pub font_size: f32,
    pub sidebar: bool,
    pub keys: &'a Keys,
}

impl Standing<'_> {
    /// What a command would do just now: to the `pane` given, or else to the
    /// one with the keyboard. In place of that, the code of the error another
    /// client is answered with: `not_supported` for a name that is not in the
    /// table, and `refused` for what cannot be done.
    pub fn plan(&self, name: &str, args: &Value, pane: Option<&str>) -> Result<Deed, &'static str> {
        let name = named(name, args);
        if command(&name).is_none() {
            return Err("not_supported");
        }
        self.deed(&name, args, pane).ok_or("refused")
    }

    /// Whether a key bound to a command is the window's. It is whether or not
    /// the command can be done just now: to a program, Ctrl+Shift+C is Ctrl+C.
    /// Only the keys that move through a pane's history are the program's
    /// when there is none to move through.
    pub fn keeps(&self, name: &str, args: &Value) -> bool {
        match self.plan(name, args, None) {
            Ok(_) => true,
            Err(code) => code == "refused" && scroll(name).is_none(),
        }
    }

    fn deed(&self, name: &str, args: &Value, pane: Option<&str>) -> Option<Deed> {
        let view = self.view;
        let text = |key: &str| args[key].as_str().map(str::to_owned);
        // The next of some things or the one before, round and round.
        let on = name.ends_with("next");
        let round = |at: usize, of: usize| (at + if on { 1 } else { of - 1 }) % of;
        let (tab, tabs) = (view.current_window(), view.windows());
        let at = tabs
            .iter()
            .position(|tab| Some(&tab.id) == view.selected_window.as_ref());
        let pane = pane.or(self.focused);
        let pane = pane.and_then(|id| tab?.panes.iter().find(|pane| pane.id == id));
        // What the pane shows, once its terminal has drawn.
        let frame = pane.and_then(|pane| pane.frame.as_ref());
        let session = || view.selected_session.clone();
        let show = |index: usize| {
            let window = tabs.get(index)?.id.clone();
            let session = session()?;
            Some(Deed::Ask(Action::SelectWindow { session, window }))
        };
        let moved = |before: Option<&Window>| {
            Some(Deed::Ask(Action::Edit {
                session: session()?,
                window: Some(tab?.id.clone()),
                edit: Edit::MoveBefore(before.map(|tab| tab.id.clone())),
            }))
        };
        // First what needs no server.
        Some(match name {
            "client.sidebar.toggle" => Deed::Sidebar,
            "client.sidebar.activity.toggle" => Deed::Agents,
            "client.palette.toggle" | "session.switch" => Deed::Palette,
            "client.font.increase" => Deed::Font(self.font_size + 1.0),
            "client.font.decrease" => Deed::Font(self.font_size - 1.0),
            "client.font.reset" => Deed::Font(DEFAULT_FONT_SIZE),
            "session.refresh" => Deed::Ask(Action::Reconnect),
            "client.mode.enter" => {
                let name = text("name").filter(|name| self.keys.map.has_mode(name))?;
                Deed::Enter(name, args["once"] == true)
            }
            "client.mode.exit" | "client.mode.exit_all" if !self.keys.modes.is_empty() => {
                Deed::Exit(name.ends_with("all"))
            }
            // Then what a pane's own terminal does with what it shows, which may
            // be the last screen of a connection that was lost.
            "pane.copy" => {
                frame.filter(|frame| !frame.selected.is_empty())?;
                Deed::Tell(pane?.id.clone(), Request::Copy)
            }
            "pane.select_all" => Deed::Tell(frame.and(pane)?.id.clone(), Request::SelectAll),
            "client.find.open" => Deed::Find(frame.and(pane)?.id.clone()),
            // The match before the one shown, or the one after it, of a search
            // under way.
            "client.find.next" | "client.find.previous" => {
                frame?.found?;
                Deed::Tell(pane?.id.clone(), Request::Step { older: on })
            }
            // And what moves through a pane's history: unless its program has the
            // whole screen, which keeps none.
            _ if scroll(name).is_some() => {
                frame.filter(|frame| !frame.alternate_screen)?;
                Deed::Tell(pane?.id.clone(), Request::Scroll(scroll(name)?))
            }
            // The rest is done by a server.
            _ if !view.connected => return None,
            "session.new" => Deed::Ask(Action::CreateSession(None)),
            "session.select" => {
                let listed = |id: &String| view.sessions.iter().any(|session| &session.id == id);
                let session = text("session_id").filter(listed)?;
                Deed::Ask(match text("window_id") {
                    Some(window) => Action::SelectWindow { session, window },
                    None => Action::SelectSession(session),
                })
            }
            "session.next" | "session.previous" => {
                let sessions = &view.sessions;
                let at = sessions
                    .iter()
                    .position(|s| Some(&s.id) == session().as_ref())?;
                let to = &sessions[round(at, sessions.len())];
                Deed::Ask(Action::SelectSession(to.id.clone()))
            }
            // A session's name is in the sidebar, and nowhere else.
            "session.rename" => {
                let session = view.session().filter(|_| self.sidebar)?;
                Deed::Rename(session.id.clone(), None, session.name.clone())
            }
            "client.tab.new" => Deed::Ask(Action::CreateWindow(session()?)),
            "client.tab.close" => Deed::CloseTab(session()?, tab?.id.clone(), tab?.idle()),
            "client.tab.rename" => {
                let (id, name) = (tab?.id.clone(), tab?.name().to_owned());
                Deed::Rename(session()?, Some(id), name)
            }
            "client.tab.next" | "client.tab.previous" => show(round(at?, tabs.len()))?,
            // From the first, which is 1, or from the last, which is -1.
            "client.tab.goto" => {
                let index = args["index"].as_i64()?;
                let from = if index < 0 { tabs.len() as i64 } else { -1 };
                show(usize::try_from(index + from).ok()?)?
            }
            "client.tab.move.backward" => moved(Some(tabs.get(at?.checked_sub(1)?)?))?,
            // Before the one after the next, or last of all.
            "client.tab.move.forward" if at? + 1 < tabs.len() => moved(tabs.get(at? + 2))?,
            "pane.split.right" | "pane.split.down" => {
                let direction = match name.ends_with("right") {
                    true => Direction::Right,
                    false => Direction::Down,
                };
                let pane = pane?.id.clone();
                Deed::Ask(Action::Split { pane, direction })
            }
            "pane.focus.left" | "pane.focus.right" | "pane.focus.up" | "pane.focus.down" => {
                Deed::Focus(tab?.beside(&pane?.id, toward(name)?)?.id.clone())
            }
            // The border of the split the pane is in moves that way.
            "pane.resize.left" | "pane.resize.right" | "pane.resize.up" | "pane.resize.down" => {
                Deed::Ask(Action::Divide(vec![tab?.nudged(&pane?.id, toward(name)?)?]))
            }
            // Even already, there is nothing to ask.
            "pane.balance" => {
                let borders = Some(tab?.balanced()).filter(|borders| !borders.is_empty());
                Deed::Ask(Action::Divide(borders?))
            }
            "pane.focus_next" | "pane.focus_previous" if tab?.panes.len() > 1 => {
                let (pane, panes) = (pane?, &tab?.panes);
                let at = panes.iter().position(|held| held.id == pane.id)?;
                Deed::Focus(panes[round(at, panes.len())].id.clone())
            }
            "pane.zoom" => {
                let pane = pane.filter(|_| tab.is_some_and(Window::split))?.id.clone();
                Deed::Ask(Action::Zoom { pane })
            }
            "pane.close" => {
                let (what, spared, pane) = (doomed(&pane?.id), pane?.idle, pane?.id.clone());
                Deed::Close(what, spared, Action::ClosePane { pane })
            }
            "pane.paste" => Deed::Paste(pane?.id.clone()),
            "pane.send_key" => Deed::Ask(input::press(&pane?.id, args["key"].as_str()?)?),
            _ => return None,
        })
    }
}

impl App {
    pub(super) fn standing<'a>(&'a self, view: &'a View) -> Standing<'a> {
        Standing {
            view,
            focused: self.focused.as_deref(),
            font_size: self.remembered.kept.font_size,
            sidebar: self.remembered.kept.sidebar,
            keys: &self.keys,
        }
    }

    /// A button that does a command, when it can be done. Under the pointer it
    /// says what it does, with the keys that do the same.
    pub(super) fn button(
        &mut self,
        ui: &mut egui::Ui,
        view: &View,
        icon: Icon,
        label: &str,
        name: &'static str,
    ) {
        let can = self.standing(view).plan(name, &Value::Null, None).is_ok();
        ui.add_enabled_ui(can, |ui| {
            if icon_button(ui, icon, &self.keys.map.tip(label, name)).clicked() {
                self.todo.push((name, None));
            }
        });
    }

    /// Do what a command comes to, on the server it was planned for. `near` says
    /// the pointer asked.
    pub(super) fn act(&mut self, ctx: &egui::Context, server: usize, deed: Deed, near: bool) {
        let row = |session, window| Target {
            server,
            session,
            window,
        };
        match deed {
            Deed::Ask(action) => self.open(server, action),
            Deed::Focus(pane) => {
                self.focused = Some(pane.clone());
                self.send(Action::FocusPane { pane });
            }
            Deed::Font(size) => {
                self.remembered.kept.font_size = size.clamp(FONT_SIZES[0], FONT_SIZES[1]);
            }
            Deed::Sidebar => self.remembered.kept.sidebar ^= true,
            // The list is the sidebar's, which is brought out for it.
            Deed::Agents => {
                self.agents ^= true;
                self.remembered.kept.sidebar |= self.agents;
            }
            // It opens with nothing typed.
            Deed::Palette => {
                self.filter.clear();
                self.cursor = None;
                Popup::toggle_id(ctx, palette::id());
            }
            Deed::Tell(pane, request) => self.ask(&pane, request),
            Deed::Find(pane) => self.find(&pane),
            // The clipboard answers in a moment, to the pane with the keyboard.
            Deed::Paste(pane) => {
                self.focused = Some(pane);
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            Deed::Close(what, spared, action) => self.close(what, spared, near, server, action),
            Deed::CloseTab(session, window, spared) => {
                let target = row(session, Some(window));
                let (what, action) = (self.row(&target), target.edit(Edit::Close));
                self.close(what, spared, near, server, action);
            }
            Deed::Rename(session, window, name) => {
                self.choose(row(session, window), &name, Choice::Rename);
            }
            Deed::Enter(name, once) => self.keys.modes.push((name, once)),
            Deed::Exit(all) => {
                let left = self.keys.modes.len().saturating_sub(1);
                self.keys.modes.truncate(if all { 0 } else { left });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_commands.rs"]
mod tests;
