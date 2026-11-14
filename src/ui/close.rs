//! Closing what may have programs running in it, which asks to be asked twice.
use std::time::{Duration, Instant};

use eframe::egui::{self, Rect};

use super::{App, style::DANGER};
use crate::model::Action;

/// What a button says that has been pressed to close something busy, and asks to
/// be pressed again.
pub const CLOSE: &str = "Close?";

/// What was asked to close while it may have had programs running, if anything,
/// and so waits to be asked again: once is easily a slip.
#[derive(Default)]
pub struct Closing(Option<Asked>);

struct Asked {
    /// The row of a session or a tab, or a pane, by name.
    what: egui::Id,
    /// It was asked with the pointer, and waits only while the pointer is by it.
    near: bool,
    /// The pointer was by it when last the window drew.
    kept: bool,
    until: Instant,
}

/// What pressing something that closes will do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    /// Close, there being nothing to lose.
    No,
    /// Wait to be pressed again.
    First,
    /// Close, having been pressed before.
    Again,
}

impl Closing {
    const TIME: Duration = Duration::from_secs(4);

    /// What pressing something that closes `what` will do. `spared` says that
    /// nothing would be lost by closing it.
    pub fn ask(&self, what: egui::Id, spared: bool) -> Ask {
        match &self.0 {
            Some(asked) if asked.what == what => Ask::Again,
            _ if spared => Ask::No,
            _ => Ask::First,
        }
    }

    /// Take a press of something that closes `what`, and say whether to close it.
    /// `near` says the press was the pointer's.
    pub fn press(&mut self, what: egui::Id, spared: bool, near: bool, now: Instant) -> bool {
        let first = self.ask(what, spared) == Ask::First;
        self.0 = first.then_some(Asked {
            what,
            near,
            kept: true,
            until: now + Self::TIME,
        });
        !first
    }

    /// The pointer is still by what waits.
    pub fn keep(&mut self) {
        if let Some(asked) = &mut self.0 {
            asked.kept = true;
        }
    }

    /// Stop waiting after a while, or once the pointer has left what it asked.
    /// Returns how long the wait still has to run: no time at all if it has just
    /// ended, and what asked has yet to be drawn as it was before.
    pub fn lapse(&mut self, now: Instant) -> Option<Duration> {
        let asked = self.0.as_mut()?;
        let left = asked.until.saturating_duration_since(now);
        let left_alone = !std::mem::take(&mut asked.kept) && asked.near;
        if left_alone || left.is_zero() {
            self.0 = None;
            return Some(Duration::ZERO);
        }
        Some(left)
    }
}

/// The name a pane goes by in being closed.
pub fn doomed(pane: &str) -> egui::Id {
    egui::Id::unique(("close", pane))
}

impl App {
    /// Close something, by the action that does it, unless programs may be lost by
    /// that and this is the first asking. `near` says the pointer did the asking.
    pub(super) fn close(
        &mut self,
        what: egui::Id,
        spared: bool,
        near: bool,
        server: usize,
        action: Action,
    ) {
        if self.closing.press(what, spared, near, Instant::now()) {
            self.tell(server, action);
        }
    }

    /// Go on waiting to be asked again to close something, while the pointer is
    /// on it, which is `within`, or in the menu `of` it.
    pub(super) fn wait(&mut self, ui: &egui::Ui, within: Rect, of: &egui::Response, ask: Ask) {
        if ask == Ask::Again && (ui.rect_contains_pointer(within) || of.context_menu_opened()) {
            self.closing.keep();
        }
    }
}

/// The menu of whatever was clicked with the other button. A click in it leaves
/// it open: its entries close it, but for one that asks to be chosen twice.
pub fn asking(response: &egui::Response) -> egui::Popup<'_> {
    egui::Popup::context_menu(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// The entry of a menu that closes something, set apart at its foot. True when
/// chosen. Chosen once, it stays under the pointer and shows that it waits to be
/// chosen again.
pub fn closer(ui: &mut egui::Ui, label: &str, ask: Ask) -> bool {
    ui.separator();
    let entry = egui::Button::new(label);
    let entry = match ask {
        Ask::Again => entry.fill(DANGER),
        _ => entry,
    };
    let chosen = ui.add(entry).clicked();
    if chosen && ask != Ask::First {
        ui.close();
    }
    chosen
}

#[cfg(test)]
#[path = "../../tests/unit/ui_close.rs"]
mod tests;
