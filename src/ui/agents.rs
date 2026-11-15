//! The sidebar as a list of what is at work: every pane on every server whose
//! program has something to say, or is a coding agent at rest, under what it
//! is doing, with how long it has been so.
use std::time::{Duration, Instant};

use eframe::egui::{self, Vec2};

use super::{
    App,
    heard::{dot, worn},
    style::{Icon, Item, SMALL, bold, caption, item, toggle},
};
use crate::model::{self, Activity, Pane, View};

/// What the panes are sorted under, in the order they are listed.
const KINDS: [&str; 4] = ["Needs you", "Done", "Working", "Idle"];

/// Which of them a pane belongs under, if any: only a coding agent is listed
/// for having nothing to say.
fn kind(pane: &Pane) -> Option<usize> {
    Some(match pane.activity {
        Activity::Blocked(_) | Activity::Error | Activity::Rang => 0,
        Activity::Done => 1,
        Activity::Working => 2,
        Activity::Idle if pane.program.agent() => 3,
        Activity::Idle => return None,
    })
}

/// How long ago something was, in a word: `now`, `4m`, `2h`, `3d`.
pub fn ago(since: Duration) -> String {
    match since.as_secs() {
        0..60 => "now".to_owned(),
        seconds @ 60..3_600 => format!("{}m", seconds / 60),
        seconds @ 3_600..86_400 => format!("{}h", seconds / 3_600),
        seconds => format!("{}d", seconds / 86_400),
    }
}

impl App {
    /// The button that turns the sidebar to that list and back. It wears the
    /// most pressing thing any pane has to say, on any server.
    pub(super) fn watch(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let tip = match self.agents {
            true => "Sessions and tabs",
            false => "What is at work, on every server",
        };
        let name = "client.sidebar.activity.toggle";
        let button = toggle(ui, Icon::Agents, &self.keys.map.tip(tip, name), self.agents);
        let sessions = views.iter().flat_map(|view| &view.sessions);
        worn(
            ui,
            &button,
            sessions.map(|session| session.activity()).max(),
        );
        if button.clicked() {
            self.todo.push((name, None));
        }
    }

    /// The list itself, in place of the servers and their sessions.
    pub(super) fn agents(&mut self, ui: &mut egui::Ui, views: &[View]) {
        // Each pane listed, under its kind: since when it has been so, its
        // server, and the session it is in.
        let mut listed: [Vec<_>; 4] = Default::default();
        for (server, view) in views.iter().enumerate() {
            for (session, tab) in view.tabs() {
                for pane in tab.every() {
                    if let Some(kind) = kind(pane) {
                        let since = self.heard.get(&pane.id).and_then(|heard| heard.since);
                        listed[kind].push((since, server, session, pane));
                    }
                }
            }
        }
        if listed.iter().all(Vec::is_empty) {
            return caption(ui, egui::RichText::new("Nothing is at work."));
        }
        let now = Instant::now();
        let (mut picked, mut wait) = (None, None);
        for (kind, mut panes) in KINDS.iter().zip(listed) {
            if panes.is_empty() {
                continue;
            }
            // What they are doing heads them, with how many they are. The one
            // that has been so longest comes first.
            let head = format!("{kind} · {}", panes.len());
            caption(ui, egui::RichText::new(head).font(bold(SMALL)));
            panes.sort_by_key(|listed| listed.0);
            for (since, server, session, pane) in panes {
                // What it is waiting for or failed at, then what it says of
                // itself, and with neither its title.
                let word = pane
                    .activity
                    .word()
                    .filter(|_| pane.activity != Activity::Done);
                let said = model::line(word.unwrap_or_default(), &pane.saying());
                let more = if said.is_empty() { &pane.title } else { &said };
                // How far along it is, where it says, or how long it has been so.
                let along = pane.along().map(|percent| format!("{percent}%"));
                let lasted = since.map(|since| now.duration_since(since));
                let word = along.or_else(|| {
                    // The time is said anew when it comes to another minute.
                    let left = 60 - lasted?.as_secs() % 60;
                    wait = Some(wait.map_or(left, |wait: u64| wait.min(left)));
                    lasted.map(ago)
                });
                let shown = server == self.current && self.focused.as_ref() == Some(&pane.id);
                let host = &self.servers[server].host.label;
                let row = Item {
                    id: Some(egui::Id::unique(("at work", host, &pane.id))),
                    icon: Some(pane.program),
                    label: &session.name,
                    more,
                    selected: shown,
                    mark: dot(ui.ctx(), pane.activity),
                    word: word.as_deref(),
                    tip: &pane.title,
                    ..Default::default()
                };
                let size = Vec2::new(ui.available_width(), 30.0);
                if item(ui, size, row).0.clicked() {
                    picked = Some((server, pane.id.clone()));
                }
            }
        }
        if let Some(wait) = wait {
            ui.ctx().request_repaint_after(Duration::from_secs(wait));
        }
        if let Some((server, pane)) = picked {
            self.reveal(server, &pane);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_agents.rs"]
mod tests;
