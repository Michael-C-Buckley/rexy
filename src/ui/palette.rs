//! One list to find things in by name: every session, the tabs of the one on
//! show, and what can be done just now.
use eframe::egui::{self, Color32, Key, Modifiers, Popup, RectAlign, Stroke, Vec2};
use serde_json::Value;

use super::{
    App,
    commands::{COMMANDS, Deed, label},
    heard::dot,
    style::{BODY, BORDER, Icon, Item, SMALL, TEXT, bold, caption, item},
};
use crate::{
    model::{Action, View},
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
            self.todo.push(("client.palette.toggle", None));
        }
    }

    /// The palette, while it is open: the sessions on every server, the tabs of
    /// the one on show, and what can be done just now, with the keys that do it.
    pub(super) fn palette(&mut self, ctx: &egui::Context, views: &[View]) {
        if !Popup::is_id_open(ctx, id()) {
            return;
        }
        let (current, view, map) = (self.current, &views[self.current], self.keys.map.clone());
        let typed = self.filter.trim().to_owned();
        let named = (!typed.is_empty()).then(|| typed.clone());
        let found = |words: &[&str]| found(&typed, words);
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
        // The sessions of each server come under its name, and so first.
        let (mut rows, mut under, mut shown) = (Vec::new(), Vec::new(), 0);
        for (server, view) in views.iter().enumerate() {
            for session in view.sessions.iter().filter(|s| found(&[&s.name])) {
                if server == current && Some(&session.id) == view.selected_session.as_ref() {
                    shown = rows.len();
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
        if let Some(session) = &view.selected_session {
            for tab in view.windows().iter().filter(|tab| found(&[tab.name()])) {
                let (session, window) = (session.clone(), tab.id.clone());
                let show = Pick::Show(current, Action::SelectWindow { session, window });
                let (icon, mark) = (Some(tab.program()), dot(ctx, tab.says().0));
                rows.push(row(tab.name(), icon, mark, show));
            }
        }
        heads.push((rows.len(), "Commands"));
        // All that can be done just now, but to put this list away.
        for (name, title, _) in COMMANDS.iter().filter(|c| found(&[c.1, c.0])) {
            let plan = self.standing(view).plan(name, &Value::Null, None);
            if plan.is_ok_and(|deed| !matches!(deed, Deed::Palette)) {
                let remote = self.remembered.kept.remote;
                rows.push(command(label(name, title, remote), name));
            }
        }
        // And those of the server's own actions that are for showing.
        let served = map.served.iter().filter(|served| !served.1.is_empty());
        for (name, title, _) in served.filter(|served| found(&[&served.1, &served.0])) {
            rows.push(command(title, name));
        }
        rows.extend(new.map(|name| {
            let label = format!("New Session “{name}”");
            let make = Pick::Show(current, Action::CreateSession(Some(name)));
            row(&label, None, None, make)
        }));

        let mut picked = None;
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
                .hint_text("Find or create…")
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
            // It opens at the session on show, and what is typed starts it afresh.
            if field.changed() {
                self.cursor = Some(0);
            }
            let at = self.cursor.unwrap_or(shown).saturating_add_signed(step);
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
                if index == at && step != 0 {
                    response.scroll_to_me(None);
                }
                if response.clicked() || (index == at && enter) {
                    picked = Some(index);
                }
            };
            // No taller than the window has room for beneath it.
            let room = ui.ctx().content_rect().height() - 140.0;
            let list = egui::ScrollArea::vertical().max_height(room.clamp(96.0, 420.0));
            list.show(ui, |ui| {
                let mut listed = rows.iter().enumerate().peekable();
                for (server, view) in views.iter().enumerate() {
                    // A session made on a server takes the name typed, too.
                    if self.heading(ui, server, view) {
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
