//! One list to find things in by name: every session, the tabs of the one on
//! show or of them all, and what can be done just now.
use eframe::egui::{self, Color32, Key, Modifiers, Popup, RectAlign, Stroke, Vec2};
use serde_json::Value;

use super::{
    App,
    commands::{COMMANDS, Deed, label},
    heard::dot,
    style::{BODY, BORDER, Icon, Item, SMALL, TEXT, bold, caption, item},
};
use crate::{
    model::{Action, Session, Tab, View},
    program::Program,
};

/// The name the palette goes by, to be opened and put away.
pub fn id() -> egui::Id {
    egui::Id::unique("palette")
}

/// Whether what is typed finds something, by any of the words for it.
fn found(typed: &str, words: &[&str]) -> bool {
    let typed = typed.trim().to_lowercase();
    words.iter().any(|word| word.to_lowercase().contains(&typed))
}

/// The words a tab is found by: its name, what its panes are titled and the
/// programs in them, and its session's name where that has other tabs too. A
/// session's only tab is found with it, by its row.
fn words<'a>(session: &'a Session, tab: &'a Tab) -> Vec<&'a str> {
    let panes = tab.every().flat_map(|pane| [pane.title.as_str(), pane.program.name()]);
    let session = (session.tabs.len() > 1).then_some(session.name.as_str());
    std::iter::once(tab.name()).chain(panes).chain(session).collect()
}

/// A row of the list, and what taking it does.
struct Row {
    label: String,
    icon: Option<Program>,
    mark: Option<(Icon, Color32)>,
    /// The keys that do the same.
    keys: String,
    pick: Pick,
}

enum Pick {
    /// Show a session or a tab, or make a session, on some server.
    Show(usize, Action),
    /// Do something by its name: an action of this client's, or of the server's.
    Do(String),
}

impl App {
    /// The session on show, as a button that opens the palette.
    pub(super) fn switcher(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let current = self.current;
        let name = views[current].session().map_or("Sessions", |s| &s.name);
        // The tabs of the session on show speak for themselves, right beside this.
        let elsewhere = views.iter().enumerate().flat_map(|(server, view)| {
            let shown = view.selected_session.as_ref().filter(|_| server == current);
            let sessions = view.sessions.iter();
            sessions.filter(move |session| Some(&session.id) != shown)
        });
        let elsewhere = elsewhere.map(|session| session.activity()).max();
        let text = ui.painter().layout_no_wrap(name.to_owned(), bold(BODY), TEXT);
        let size = Vec2::new((text.size().x + 54.0).min(240.0), 28.0);
        let button = Item {
            menu: true,
            strong: true,
            mark: dot(ui.ctx(), elsewhere.unwrap_or_default()),
            label: name,
            ..Default::default()
        };
        if item(ui, size, button).0.clicked() {
            self.todo.push(("session.switch", None));
        }
    }

    /// The palette, while it is open: the sessions on every server, the tabs of
    /// the one on show, and what can be done just now, with the keys that do it.
    /// What is typed is looked for among the tabs of every session, too.
    pub(super) fn palette(&mut self, ctx: &egui::Context, views: &[View]) {
        if !Popup::is_id_open(ctx, id()) {
            return;
        }
        let (current, view, map) = (self.current, &views[self.current], self.keys.map.clone());
        let typed = self.filter.trim().to_owned();
        let named = (!typed.is_empty()).then(|| typed.clone());
        let found = |words: &[&str]| found(&typed, words);
        // With nothing typed it lists what it was opened for. What is typed is
        // looked for in all of it.
        let (places, deeds) = (named.is_some() || !self.commands, named.is_some() || self.commands);
        let row = |label: &str, icon, mark, pick| Row {
            label: label.to_owned(),
            icon,
            mark,
            keys: String::new(),
            pick,
        };
        let command = |label: &str, name: &str| Row {
            keys: map.shortcut(name),
            ..row(label, None, None, Pick::Do(name.to_owned()))
        };
        // The sessions of each server come under its name, and so first. The
        // one shown before this one is picked out, so that Enter goes back to
        // it, or failing that the one on show.
        let (mut rows, mut under, mut shown, mut back) = (Vec::new(), Vec::new(), 0, None);
        for (server, view) in views.iter().enumerate() {
            for session in view.sessions.iter().filter(|s| places && found(&[&s.name])) {
                if server == current && Some(&session.id) == view.selected_session.as_ref() {
                    shown = rows.len();
                }
                if self.visited[1]
                    .as_ref()
                    .is_some_and(|(at, id)| (*at, id) == (server, &session.id))
                {
                    back = Some(rows.len());
                }
                let show = Pick::Show(server, Action::SelectSession(session.id.clone()));
                rows.push(row(&session.name, None, dot(ctx, session.activity()), show));
            }
            under.push(rows.len());
        }
        // A name that finds no session offers to make one.
        let new = named.clone().filter(|_| rows.is_empty());
        // What comes after the sessions is headed by what it is.
        let mut heads = vec![(rows.len(), "Tabs")];
        // Those of the session on show come first, by their names alone and
        // with the keys of their places. What is typed is looked for in every
        // other session too, and a tab found there says where it is: in which
        // session, and on which server if that is another.
        let mut others = Vec::new();
        for (server, held) in views.iter().enumerate().filter(|_| places) {
            let shown = held.selected_session.as_ref().filter(|_| server == current);
            let elsewhere = (server != current).then(|| self.servers[server].label());
            let looked = |session: &&Session| named.is_some() || Some(&session.id) == shown;
            for session in held.sessions.iter().filter(looked) {
                let (away, of) = (Some(&session.id) != shown, session.tabs.len());
                let tabs = session.tabs.iter().enumerate();
                for (place, tab) in tabs.filter(|tab| found(&words(session, tab.1))) {
                    let label = [elsewhere, away.then_some(&*session.name), Some(tab.name())];
                    let label = label.into_iter().flatten().collect::<Vec<_>>().join(" › ");
                    let keys = if away { String::new() } else { map.tab(place, of) };
                    let to = Action::SelectTab { session: session.id.clone(), tab: tab.id.clone() };
                    let (icon, mark) = (Some(tab.program()), dot(ctx, tab.says().0));
                    let row = Row { keys, ..row(&label, icon, mark, Pick::Show(server, to)) };
                    if away { others.push(row) } else { rows.push(row) }
                }
            }
        }
        rows.append(&mut others);
        heads.push((rows.len(), "Commands"));
        // All that can be done just now, but to put this list away.
        for (name, title, _) in COMMANDS.iter().filter(|c| deeds && found(&[c.1, c.0])) {
            let plan = self.standing(view).plan(name, &Value::Null, None);
            if plan.is_ok_and(|deed| !matches!(deed, Deed::Palette { .. })) {
                let remote = self.remembered.kept.remote;
                rows.push(command(label(name, title, remote), name));
            }
        }
        // And those of the server's own actions that are for showing.
        let served = map.served.iter().filter(|served| deeds && !served.1.is_empty());
        for (name, title, _) in served.filter(|served| found(&[&served.1, &served.0])) {
            rows.push(command(title, name));
        }
        rows.extend(new.map(|name| {
            // On the server on show, which is said where there are others.
            let label = match self.servers.len() {
                1 => format!("New Session “{name}”"),
                _ => format!("New Session “{name}” on {}", self.servers[current].label()),
            };
            let make = Pick::Show(current, Action::CreateSession(Some(name)));
            row(&label, None, None, make)
        }));

        let (mut picked, mut pointed) = (None, None);
        let top = ctx.content_rect().center_top() + egui::vec2(0.0, 56.0);
        let popup = Popup::new(id(), ctx.clone(), top, egui::LayerId::background())
            .open_memory(None)
            .align(RectAlign::BOTTOM)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(520.0);
        popup.show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // The arrows walk the list while what is typed goes to the field.
            let (step, enter) = ui.input_mut(|i| {
                let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
                (isize::from(down) - isize::from(up), i.key_pressed(Key::Enter))
            });
            // The field is the head of the list, with a line under it and none
            // around it: nothing else here could be taking what is typed.
            let field = egui::TextEdit::singleline(&mut self.filter)
                .hint_text(match self.commands {
                    true => "Search commands…",
                    false => "Search sessions and tabs, or name a new session…",
                })
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 8)))
                .desired_width(f32::INFINITY);
            let field = ui.add(field);
            let rule = field.rect.bottom() + 3.0;
            let line = Stroke::new(1.0, BORDER);
            ui.painter().hline(ui.max_rect().x_range(), rule, line);
            ui.add_space(6.0);
            // The keyboard is its own while it is open, wherever a click lands.
            if !field.has_focus() {
                field.request_focus();
            }
            // What is typed starts it afresh.
            if field.changed() {
                self.cursor = Some(0);
            }
            // It opens from the top, whatever it was scrolled to when last open.
            let fresh = self.cursor.is_none();
            let moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO);
            let at = self.cursor.unwrap_or(back.unwrap_or(shown)).saturating_add_signed(step);
            let at = at.min(rows.len().saturating_sub(1));
            self.cursor = Some(at);
            let mut row = |ui: &mut egui::Ui, index: usize, row: &Row| {
                // Its keys stand where a mark's word would.
                let keys = Some(row.keys.as_str()).filter(|keys| !keys.is_empty());
                let entry = Item {
                    selected: index == at,
                    icon: row.icon,
                    mark: row.mark,
                    keys,
                    label: &row.label,
                    ..Default::default()
                };
                let (response, _) = item(ui, Vec2::new(ui.available_width(), 32.0), entry);
                if index == at && (step != 0 || fresh) {
                    response.scroll_to_me(None);
                }
                // The pointer picks out the row it moves onto, as the arrows do:
                // one row is lit, and Enter takes that one.
                if moved && response.hovered() {
                    pointed = Some(index);
                }
                if response.clicked() || (index == at && enter) {
                    picked = Some(index);
                }
            };
            // No taller than the window has room for beneath it.
            let room = ui.ctx().content_rect().height() - 140.0;
            // And no shorter than what it lists, where that is less.
            let room = room.clamp(96.0, 420.0);
            let mut list = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room);
            if fresh {
                list = list.vertical_scroll_offset(0.0);
            }
            list.show(ui, |ui| {
                let mut listed = rows.iter().enumerate().peekable();
                for (server, view) in views.iter().enumerate() {
                    // A session made on a server takes the name typed, too.
                    if places && self.heading(ui, server, view, false) {
                        self.open(server, Action::CreateSession(named.clone()));
                        Popup::close_id(ui.ctx(), id());
                    }
                    while let Some((index, entry)) = listed.next_if(|row| row.0 < under[server]) {
                        row(ui, index, entry);
                    }
                }
                for (index, entry) in listed {
                    // A head with nothing found under it is not shown.
                    let head = heads.iter().rev().find(|head| head.0 <= index);
                    if let Some((_, head)) = head.filter(|head| head.0 == index) {
                        caption(ui, egui::RichText::new(*head).font(bold(SMALL)));
                    }
                    row(ui, index, entry);
                }
            });
        });
        self.cursor = pointed.or(self.cursor);
        let Some(row) = picked.map(|picked| rows.swap_remove(picked)) else {
            return;
        };
        Popup::close_id(ctx, id());
        match row.pick {
            Pick::Show(server, action) => self.open(server, action),
            Pick::Do(name) => self.run(ctx, view, &name, &Value::Null, "palette"),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_palette.rs"]
mod tests;
