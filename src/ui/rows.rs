//! The rows of sessions and tabs, in the sidebar or along the top: showing,
//! renaming, moving and closing them.
use eframe::egui::{
    self, Align2, DragAndDrop, Key, PointerButton, Pos2, Rect, Sense, Stroke, UiBuilder, Vec2,
};

use super::{
    App,
    close::{Ask, CLOSE, asking, closer},
    heard::dot,
    style::{ACCENT, DIM, Icon, Item, entry, item},
};
use crate::model::{self, Action, Edit, Session, Window};

/// A session or, with `window`, one of its tabs.
#[derive(Clone, PartialEq)]
pub(super) struct Target {
    pub(super) server: usize,
    pub(super) session: String,
    pub(super) window: Option<String>,
}

impl Target {
    /// The action that shows it.
    fn select(&self) -> Action {
        let session = self.session.clone();
        match self.window.clone() {
            Some(window) => Action::SelectWindow { session, window },
            None => Action::SelectSession(session),
        }
    }

    /// The action that changes it.
    pub(super) fn edit(self, edit: Edit) -> Action {
        Action::Edit {
            session: self.session,
            window: self.window,
            edit,
        }
    }
}

/// A name being typed over a session's or a tab's.
pub struct Rename {
    target: Target,
    text: String,
    /// The name the field opened with. Left as it was, it is not given anew: that
    /// would name a tab for good by what was running in it just then.
    was: String,
    /// The field has yet to be shown.
    fresh: bool,
    /// The field was shown this frame. A row that has gone takes its renaming with it.
    pub shown: bool,
}

/// What was asked of a session or a tab through its menu.
pub(super) enum Choice {
    Rename,
    /// Or the reverse, of a tab that is muted.
    Mute,
    Edit(Edit),
}

impl App {
    /// The name a session's or a tab's row goes by, which stays its own whatever
    /// rows come and go around it.
    pub(super) fn row(&self, target: &Target) -> egui::Id {
        let host = &self.servers[target.server].host.label;
        egui::Id::unique(("row", host, &target.session, &target.window))
    }

    /// A session's tabs, down the sidebar or along the top. `shown` is the one on show.
    pub(super) fn tabs(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        session: &Session,
        shown: Option<&str>,
        size: Vec2,
    ) {
        let windows = session.windows.iter().enumerate();
        let tabs: Vec<_> = windows
            .map(|(index, window)| {
                (
                    &window.id,
                    self.tab(ui, server, session, shown, index, size),
                )
            })
            .collect();
        self.reorder(ui, server, &tabs);
    }

    /// While one of these rows is dragged, mark where among them it would land, and
    /// move it there when it is let go. The rows are a server's sessions, or the tabs
    /// of one of them, each with the space it takes.
    pub(super) fn reorder(&mut self, ui: &egui::Ui, server: usize, rows: &[(&String, Rect)]) {
        let Some(drag) = DragAndDrop::payload::<Target>(ui.ctx()) else {
            return;
        };
        let id = drag.window.as_ref().unwrap_or(&drag.session);
        let from = rows.iter().position(|(row, _)| *row == id);
        let from = from.filter(|_| drag.server == server);
        let (Some(from), Some(pointer)) = (from, ui.input(|i| i.pointer.latest_pos())) else {
            return;
        };
        let across = ui.layout().is_horizontal();
        let along = |pos: Pos2| if across { pos.x } else { pos.y };
        // It lands ahead of the first row whose middle the pointer has yet to pass.
        let passed = |(_, space): &&(&String, Rect)| along(pointer) >= along(space.center());
        let to = rows.iter().take_while(passed).count();
        // Where it is already, there is nowhere to go.
        if to == from || to == from + 1 {
            return;
        }
        let (edge, row) = match rows.get(to) {
            Some((_, space)) => (along(space.min) - 1.0, space),
            None => (along(rows[to - 1].1.max) + 1.0, &rows[to - 1].1),
        };
        let line = Stroke::new(2.0, ACCENT);
        if across {
            ui.painter().vline(edge, row.y_range(), line);
        } else {
            ui.painter().hline(row.x_range(), edge, line);
        }
        if ui.input(|i| i.pointer.any_released()) {
            let before = rows.get(to).map(|(row, _)| (*row).clone());
            self.edit(Target::clone(&drag), Edit::MoveBefore(before));
        }
    }

    /// A session's name, heading its tabs in the sidebar.
    pub(super) fn session(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        sessions: &[Session],
        index: usize,
        shown: bool,
    ) {
        let session = &sessions[index];
        let target = Target {
            server,
            session: session.id.clone(),
            window: None,
        };
        let size = Vec2::new(ui.available_width(), 28.0);
        let id = self.row(&target);
        if self.renaming(ui, &target, id, size).is_some() {
            return;
        }
        let spared = session.windows.iter().all(Window::idle);
        let ask = self.closing.ask(id, spared);
        let row = Item {
            id: Some(id),
            caption: true,
            buttons: &[(Icon::Plus, "New tab"), (Icon::Close, "Close session")],
            confirm: (ask == Ask::Again).then_some(CLOSE),
            movable: true,
            label: &session.name,
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        self.picked(&response, &target, shown);
        self.wait(ui, response.rect, &response, ask);
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = ["Move Up", "Move Down", "Close Session"];
        let at = |index: usize| sessions.get(index).map(|s| s.id.clone());
        match (button, menu(&response, labels, None, index, at, ask)) {
            (Some(0), _) => self.open(server, Action::CreateWindow(session.id.clone())),
            // This ends every program in the session.
            (Some(_), _) | (None, Some(Choice::Edit(Edit::Close))) => {
                self.close(id, spared, true, server, target.edit(Edit::Close));
            }
            (None, Some(choice)) => self.choose(target, &session.name, choice),
            (None, None) => {}
        }
    }

    /// Take a click on a session's or a tab's row, if there was one, and show the
    /// row unless it is on show. The second click of two asks for a new name, and
    /// nothing tells a first click from a lone one, so the first has shown the
    /// row by then.
    fn picked(&mut self, response: &egui::Response, target: &Target, shown: bool) {
        if response.clicked() && !shown {
            self.open(target.server, target.select());
        }
    }

    /// A tab, in the sidebar or along the top. Returns the space it took.
    fn tab(
        &mut self,
        ui: &mut egui::Ui,
        server: usize,
        session: &Session,
        shown: Option<&str>,
        index: usize,
        size: Vec2,
    ) -> Rect {
        let across = ui.layout().is_horizontal();
        let window = &session.windows[index];
        let target = Target {
            server,
            session: session.id.clone(),
            window: Some(window.id.clone()),
        };
        let id = self.row(&target);
        // Along the top a tab is as wide as its name, up to the size given: its
        // icon and the room for its mark or its button come to as much again as
        // the row's own edges.
        let size = match across {
            true => {
                let font = egui::FontId::proportional(super::style::BODY);
                let name = ui.painter().layout_no_wrap(window.name().into(), font, DIM);
                Vec2::new((name.size().x + 70.0).clamp(96.0, size.x), size.y)
            }
            false => size,
        };
        if let Some(field) = self.renaming(ui, &target, id, size) {
            return field;
        }
        let muted = self.remembered.kept.muted.contains(&window.id);
        let (said, message) = window.says();
        let selected = shown == Some(&window.id);
        let spared = window.idle();
        let ask = self.closing.ask(id, spared);
        // What it says in full, for a row with no room for it, and with the line
        // its program adds.
        let tip = model::line(said.word().unwrap_or_default(), message);
        let row = Item {
            id: Some(id),
            icon: Some(window.program()),
            selected,
            mark: if muted {
                Some((Icon::Muted, DIM))
            } else {
                dot(said)
            },
            word: said.word(),
            buttons: &[(Icon::Close, "Close tab")],
            confirm: (ask == Ask::Again).then_some(CLOSE),
            tip: &tip,
            movable: true,
            label: window.name(),
            ..Default::default()
        };
        let (response, button) = item(ui, size, row);
        // The list turns to the tab on show when another comes to be it, by
        // whatever means, and stays where the user turns it otherwise.
        let list = id.with(self.remembered.kept.sidebar);
        if selected && self.followed != Some(list) {
            self.followed = Some(list);
            response.scroll_to_me(None);
        }
        self.picked(&response, &target, selected);
        self.wait(ui, response.rect, &response, ask);
        if response.drag_started_by(PointerButton::Primary) {
            DragAndDrop::set_payload(ui.ctx(), target.clone());
        }
        let labels = if across {
            ["Move Left", "Move Right", "Close Tab"]
        } else {
            ["Move Up", "Move Down", "Close Tab"]
        };
        let at = |index: usize| session.windows.get(index).map(|w| w.id.clone());
        match (button, menu(&response, labels, Some(muted), index, at, ask)) {
            (Some(_), _) | (None, Some(Choice::Edit(Edit::Close))) => {
                self.close(id, spared, true, server, target.edit(Edit::Close));
            }
            // The field opens on the name it goes by, given or not.
            (None, Some(choice)) => self.choose(target, window.name(), choice),
            (None, None) => {}
        }
        response.rect
    }

    /// Do what a session's or a tab's menu was asked. `name` is the one it goes by.
    pub(super) fn choose(&mut self, target: Target, name: &str, choice: Choice) {
        match choice {
            Choice::Rename => {
                self.rename = Some(Rename {
                    target,
                    text: name.to_owned(),
                    was: name.to_owned(),
                    fresh: true,
                    shown: true,
                });
            }
            Choice::Mute => {
                let tab = target.window.as_deref().unwrap_or_default();
                let muted = &mut self.remembered.kept.muted;
                if !muted.remove(tab) {
                    muted.insert(tab.to_owned());
                }
                self.desktop.mute(muted.clone());
            }
            Choice::Edit(edit) => self.edit(target, edit),
        }
    }

    fn edit(&mut self, target: Target, edit: Edit) {
        self.tell(target.server, target.edit(edit));
    }

    /// Show the field that stands in for a row while it is renamed, and return the
    /// space it took. None if this row is not the one.
    fn renaming(
        &mut self,
        ui: &mut egui::Ui,
        target: &Target,
        row: egui::Id,
        size: Vec2,
    ) -> Option<Rect> {
        let rename = self.rename.as_mut().filter(|r| &r.target == target)?;
        rename.shown = true;
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        // By the row's own name, so that the field keeps the keyboard when rows
        // before it come and go.
        let field = egui::TextEdit::singleline(&mut rename.text)
            .id(row.with("rename"))
            .hint_text("Name")
            .margin(egui::vec2(11.0, 0.0))
            .align(Align2::LEFT_CENTER)
            .min_size(rect.size());
        let mut output = field.show(&mut ui.new_child(UiBuilder::new().max_rect(rect)));
        if std::mem::take(&mut rename.fresh) {
            // Typing replaces the old name.
            let all = egui::text::CCursorRange::select_all_str(&rename.text);
            output.state.cursor.set_char_range(Some(all));
            output.state.store(ui.ctx(), output.response.id);
            output.response.request_focus();
        } else if output.response.lost_focus() {
            // Escape leaves the name as it was. Anything else, such as Enter or a
            // click elsewhere, takes the new one, if it is new.
            let name = self
                .rename
                .take()
                .filter(|rename| rename.text != rename.was);
            let name = name.map(|rename| rename.text);
            if let Some(name) = name.filter(|_| !ui.input(|i| i.key_pressed(Key::Escape))) {
                self.edit(target.clone(), Edit::Rename(name));
            }
        }
        Some(rect)
    }
}

/// What a session's or a tab's menu was asked, if anything. A double click asks for
/// a new name, as the menu's first entry does. A tab says whether it is `muted`.
/// The row is the one at `index` among its like, whose IDs `id` gives; `labels`
/// name moving it back, moving it on, and closing it, and `ask` says what
/// choosing to close it will do.
fn menu(
    response: &egui::Response,
    labels: [&str; 3],
    muted: Option<bool>,
    index: usize,
    id: impl Fn(usize) -> Option<String>,
    ask: Ask,
) -> Option<Choice> {
    let mut picked = response.double_clicked().then_some(Choice::Rename);
    asking(response, 200.0, |ui| {
        let mute = |muted| if muted { "Unmute Tab" } else { "Mute Tab" };
        let entries = [
            Some(("Rename", true, Choice::Rename)),
            muted.map(|muted| (mute(muted), true, Choice::Mute)),
            Some((
                labels[0],
                index > 0,
                Choice::Edit(Edit::MoveBefore(id(index.saturating_sub(1)))),
            )),
            Some((
                labels[1],
                id(index + 1).is_some(),
                // Before the one after the next, or last of all.
                Choice::Edit(Edit::MoveBefore(id(index + 2))),
            )),
        ];
        for (label, enabled, choice) in entries.into_iter().flatten() {
            let row = Item {
                label,
                ..Default::default()
            };
            if entry(ui, enabled, row).clicked() {
                picked = Some(choice);
                ui.close();
            }
        }
        if closer(ui, labels[2], ask) {
            picked = Some(Choice::Edit(Edit::Close));
        }
    });
    picked
}
