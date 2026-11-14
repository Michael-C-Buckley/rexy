//! The keys that are the window's own, by the keymap in force, and those that go
//! to the terminal. Other clients' requests are answered here too.
use eframe::egui::{self, Key, Modifiers, Popup};
use serde_json::Value;

use super::{
    App, input,
    keymap::{self, Binding, FORWARD, Step},
    palette,
};
use crate::{
    model::{Action, Pane, View},
    terminal::Request,
};

impl App {
    /// Take this frame's key presses that the keymap binds, and do what they are
    /// bound to. What is left is the terminal's.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context, view: &View) {
        let pressed = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        // A terminal's programs own Ctrl+C, X and V. In a text field they mean
        // what they usually do.
        if ctx.text_edit_focused() {
            for (key, event) in [(Key::C, egui::Event::Copy), (Key::X, egui::Event::Cut)] {
                if pressed(Modifiers::CTRL, key) {
                    ctx.input_mut(|i| i.events.push(event));
                }
            }
            if pressed(Modifiers::CTRL, Key::V) {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
        }
        // A server's keymap holds while the server can be asked to act on it.
        self.keys
            .follow(view.keymap.as_ref().filter(|_| view.connected));
        self.keyboard.prefix = None;
        let press = |(index, event): (usize, &egui::Event)| match *event {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers,
            } => Some((index, key, physical_key, repeat, modifiers)),
            _ => None,
        };
        let presses: Vec<_> =
            ctx.input(|i| i.events.iter().enumerate().filter_map(press).collect());
        let mut taken = Vec::new();
        for (index, key, physical, repeat, modifiers) in presses {
            let modifiers = self.keyboard.bits(modifiers);
            let spelled = input::spellings(key, physical, modifiers);
            let key = physical.unwrap_or(key);
            if spelled.is_some_and(|spelled| self.press(ctx, view, key, repeat, spelled)) {
                taken.push(index);
            }
        }
        if !taken.is_empty() {
            ctx.input_mut(|i| input::remove(&mut i.events, &taken));
        }
    }

    /// Take a key press, spelled as Rex spells it, and say whether it was the
    /// window's: if not, it is left to whatever has the keyboard.
    fn press(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        key: Key,
        repeat: bool,
        spelled: Vec<String>,
    ) -> bool {
        let map = self.keys.map.clone();
        let chord = spelled[0].clone();
        // A key held down from a press that was the window's does again only what
        // is the better for it, and nothing else: its repeats would answer for
        // the user, or put away the palette it opened.
        let held = self.keys.held.as_ref();
        if let Some((_, again)) = held.filter(|held| repeat && held.0 == chord).cloned() {
            if let Some(again) = again {
                self.run(ctx, view, &again.name, &again.args, "key");
            }
            return true;
        }
        let again = |bound: &Binding| map.repeats(&bound.name).then(|| bound.clone());
        // A text field keeps its keys, but for those that are about the field
        // itself: what puts the palette away, and what finds the next match.
        if self.busy(ctx) {
            let palette = Popup::is_id_open(ctx, palette::id());
            let about = |bound: &&Binding| {
                let puts_away = ["client.palette.toggle", "session.switch"].contains(&&*bound.name);
                let finds = self.find.is_some() && bound.name.starts_with("client.find.");
                ((palette && puts_away) || finds)
                    && bound.mode.is_none()
                    && matches!(&bound.keys[..], [key] if spelled.contains(key))
            };
            let bound = map.bindings.iter().find(about);
            if let Some(bound) = bound {
                self.run(ctx, view, &bound.name, &bound.args, "key");
            }
            self.keys.held = bound.map(|bound| (chord, again(bound)));
            return bound.is_some();
        }
        self.keys.pending.push(spelled);
        // A key that a function on the server answers to can always be sent there.
        let can = |bound: &Binding| {
            let served = || map.served.iter().any(|served| served.0 == bound.name);
            let own = self.standing(view).keeps(&bound.name, &bound.args);
            own || bound.name == FORWARD || served()
        };
        let step = keymap::resolve(&map, &self.keys.modes, &self.keys.pending, can);
        if step != Step::Wait {
            self.keys.pending.clear();
        }
        let (mut taken, mut repeated) = (true, None);
        match step {
            Step::Pass => taken = false,
            Step::Wait | Step::Void => {}
            Step::Leave => _ = self.keys.modes.pop(),
            Step::Run(bound) => {
                // A mode entered for one key has had it.
                let spent =
                    |(name, once): &(String, bool)| *once && Some(name) == bound.mode.as_ref();
                self.keys.modes.retain(|mode| !spent(mode));
                if bound.name == FORWARD {
                    // The key goes there as any key does, with the keys before it.
                    if let [prefix @ .., _] = bound.keys.as_slice()
                        && !prefix.is_empty()
                    {
                        self.keyboard.prefix = Some((key, prefix.to_vec()));
                    }
                    taken = false;
                } else {
                    self.run(ctx, view, &bound.name, &bound.args, "key");
                    repeated = again(bound);
                }
            }
        }
        self.keys.held = taken.then_some((chord, repeated));
        taken
    }

    /// Do something by its name, for the key or the row of the palette that asked:
    /// an action of this client's, or else one of the server's own.
    pub(super) fn run(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        name: &str,
        args: &Value,
        origin: &'static str,
    ) {
        match self.standing(view).plan(name, args, None) {
            Ok(deed) => self.act(ctx, self.current, deed, false),
            Err("not_supported") => self.send(Action::Perform {
                name: name.to_owned(),
                args: args.clone(),
                origin,
                pane: self.focused.clone(),
            }),
            Err(_) => {}
        }
    }

    /// Do what the buttons and menus asked for while they were drawn, and what
    /// was asked of this client from each server: by another client, which is
    /// answered, or by an action of the server's that this client began.
    pub(super) fn answer(&mut self, ctx: &egui::Context, views: &[View]) {
        for (name, pane) in std::mem::take(&mut self.todo) {
            let view = &views[self.current];
            if let Ok(deed) = self
                .standing(view)
                .plan(name, &Value::Null, pane.as_deref())
            {
                self.act(ctx, self.current, deed, true);
            }
        }
        for (server, view) in views.iter().enumerate() {
            for request in &view.asked {
                // A server's clients have no say over what another server's session
                // shows, nor over a session that is not on show: but to have one shown.
                let shown = request.session.is_none() || request.session == view.selected_session;
                let ours = server == self.current && shown;
                let plan = if request.name == "session.select" || ours {
                    let (name, pane) = (&request.name, request.pane.as_deref());
                    self.standing(view).plan(name, &request.args, pane)
                } else {
                    Err("refused")
                };
                let error = plan.as_ref().err().copied();
                // What asks the user first is answered now, not when the user decides.
                if let Ok(deed) = plan {
                    self.act(ctx, server, deed, false);
                }
                if let Some(call) = request.call.clone() {
                    self.tell(server, Action::Reply { call, error });
                }
            }
        }
    }

    /// Send this frame's keyboard events to the focused pane.
    pub(super) fn terminal_keys(&mut self, ctx: &egui::Context, pane: &Pane) {
        let selected = pane.frame.as_ref().is_some_and(|f| !f.selected.is_empty());
        // Escape leaves a selection instead of reaching the program.
        let left = selected && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let actions = ctx.input(|i| self.keyboard.translate(&pane.id, &i.events));
        // Typing lets go of one too. The pane's worker is not woken for nothing:
        // this is asked at every key typed.
        if selected && (left || actions.iter().any(input::clears_selection)) {
            self.ask(&pane.id, Request::Deselect);
        }
        for action in actions {
            self.send(action);
        }
    }
}
