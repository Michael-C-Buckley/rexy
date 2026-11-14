//! The list of every session, to find one by name or make one.
use eframe::egui::{self, FontId, Key, Modifiers, Vec2};

use super::{
    App,
    heard::dot,
    style::{BODY, Item, TEXT, item},
};
use crate::model::{Action, Session, View};

impl App {
    /// The session on show, as a button that lists them all: to find one by name,
    /// or to make one with a name of the user's choosing.
    pub(super) fn switcher(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let current = self.current;
        let name = views[current].session().map_or("Sessions", |s| &s.name);
        // Every session, and whether it is the one on show.
        let sessions = || {
            let servers = views.iter().enumerate();
            servers.flat_map(|(server, view)| {
                let shown = view.selected_session.as_ref();
                let shown = shown.filter(|_| server == current);
                let sessions = view.sessions.iter();
                sessions.map(move |session| (Some(&session.id) == shown, session))
            })
        };
        // The tabs of the session on show speak for themselves, right beside this.
        let elsewhere = sessions().filter(|(shown, _)| !shown);
        let elsewhere = elsewhere.map(|(_, session)| session.activity()).max();
        let font = FontId::proportional(BODY);
        let text = ui.painter().layout_no_wrap(name.to_owned(), font, TEXT);
        let size = Vec2::new((text.size().x + 54.0).min(240.0), 30.0);
        let button = Item {
            menu: true,
            mark: dot(elsewhere.unwrap_or_default()),
            label: name,
            ..Default::default()
        };
        let (button, _) = item(ui, size, button);
        let id = egui::Popup::default_response_id(&button);
        let opened = !egui::Popup::is_id_open(ui.ctx(), id);
        if opened {
            // It opens with nothing typed, at the session on show.
            self.filter.clear();
            self.cursor = sessions().position(|(shown, _)| shown).unwrap_or(0);
        }
        let mut picked = None;
        egui::Popup::menu(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(280.0)
            .show(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // The arrows walk the list while what is typed goes to the field.
                let step = ui.input_mut(|i| {
                    let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                    isize::from(down) - isize::from(i.consume_key(Modifiers::NONE, Key::ArrowUp))
                });
                let field = egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter or create…")
                    .margin(egui::vec2(10.0, 8.0))
                    .desired_width(f32::INFINITY);
                let field = ui.add(field);
                if opened {
                    field.request_focus();
                }
                if field.changed() {
                    self.cursor = 0;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let name = self.filter.trim().to_owned();
                let wanted = name.to_lowercase();
                let found = |session: &&Session| session.name.to_lowercase().contains(&wanted);
                let count = sessions().filter(|(_, session)| found(session)).count();
                // After the sessions comes the row that makes one, by the name typed.
                self.cursor = self.cursor.saturating_add_signed(step).min(count);
                let create = || Action::CreateSession((!name.is_empty()).then(|| name.clone()));
                let cursor = self.cursor;
                let mut index = 0;
                // A row of the list. True if it was taken, by a click or with Enter.
                let mut row = |ui: &mut egui::Ui, label: &str, mark| {
                    let at = index == cursor;
                    index += 1;
                    let row = Item {
                        selected: at,
                        mark,
                        label,
                        ..Default::default()
                    };
                    let (response, _) = item(ui, Vec2::new(ui.available_width(), 32.0), row);
                    if at && step != 0 {
                        response.scroll_to_me(None);
                    }
                    response.clicked() || (at && enter)
                };
                let list = egui::ScrollArea::vertical().max_height(420.0);
                list.show(ui, |ui| {
                    for (server, view) in views.iter().enumerate() {
                        // A session made on a server takes the name typed, too.
                        if self.heading(ui, server, view) {
                            picked = Some((server, create()));
                        }
                        for session in view.sessions.iter().filter(found) {
                            if row(ui, &session.name, dot(session.activity())) {
                                picked = Some((server, Action::SelectSession(session.id.clone())));
                            }
                        }
                    }
                    ui.add_space(6.0);
                    let label = match name.as_str() {
                        "" => "New Session".to_owned(),
                        name => format!("New Session “{name}”"),
                    };
                    if row(ui, &label, None) {
                        picked = Some((current, create()));
                    }
                });
            });
        if let Some((server, action)) = picked {
            egui::Popup::close_id(ui.ctx(), id);
            self.open(server, action);
        }
    }
}
