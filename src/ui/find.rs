//! The field that finds text in a pane, through all of its history.
use eframe::egui::{self, Align2, FontId, Key, Margin, Rect, UiBuilder};

use super::{
    App,
    style::{DIM, POPOVER, SMALL},
};
use crate::{model::Pane, terminal::Request};

/// Text being looked for in a pane.
pub struct Find {
    pane: String,
    text: String,
    /// The field has yet to take the keyboard.
    fresh: bool,
    /// The field was shown this frame. A pane that has gone takes it along.
    pub shown: bool,
}

impl App {
    /// Open the field that finds text in a pane, or take the keyboard back to it.
    pub(super) fn find(&mut self, pane: &str) {
        if let Some(find) = self.find.as_mut().filter(|find| find.pane == pane) {
            find.fresh = true;
            return;
        }
        self.find_close();
        self.find = Some(Find {
            pane: pane.into(),
            text: String::new(),
            fresh: true,
            shown: true,
        });
    }

    /// Go to the match before the one shown, while the field is open.
    pub(super) fn find_next(&mut self, pane: &str) {
        self.ask(pane, Request::FindNext);
    }

    /// Go to the match after the one shown, while the field is open.
    pub(super) fn find_previous(&mut self, pane: &str) {
        self.ask(pane, Request::FindPrevious);
    }

    /// Stop looking. The match last shown stays selected.
    pub(super) fn find_close(&mut self) {
        if let Some(find) = self.find.take() {
            self.ask(&find.pane, Request::Find(String::new()));
        }
    }

    /// Show the field at the top right of the pane it looks through. Enter goes
    /// to the match before, with Shift to the one after, and anything else that
    /// takes the keyboard from it closes it.
    pub(super) fn finding(&mut self, ui: &mut egui::Ui, pane: &Pane, within: Rect) {
        let Some(find) = self.find.as_mut().filter(|find| find.pane == pane.id) else {
            return;
        };
        find.shown = true;
        let said = match pane.frame.as_ref().and_then(|frame| frame.found) {
            None => String::new(),
            Some((_, 0)) => "No matches".into(),
            Some((at, of)) => format!("{at} of {of}"),
        };
        let size = egui::vec2(within.width().min(300.0), 32.0);
        let bar = Rect::from_min_size(within.right_top() - egui::vec2(size.x, 0.0), size);
        let font = FontId::proportional(SMALL);
        let said = ui.painter().layout_no_wrap(said, font, DIM);
        // The whole bar is the field, so that a click anywhere on it leaves it open.
        let margin = Margin {
            left: 10,
            right: (said.size().x + 20.0) as i8,
            ..Margin::ZERO
        };
        let field = egui::TextEdit::singleline(&mut find.text)
            .id(egui::Id::unique("find"))
            .hint_text("Find")
            .margin(margin)
            .align(Align2::LEFT_CENTER)
            .background_color(POPOVER)
            .min_size(bar.size());
        let field = ui.new_child(UiBuilder::new().max_rect(bar)).add(field);
        let count = bar.right_center() - egui::vec2(said.size().x + 10.0, said.size().y / 2.0);
        ui.painter().galley(count, said, DIM);
        let request = if std::mem::take(&mut find.fresh) {
            field.request_focus();
            None
        } else if field.changed() {
            Some(Request::Find(find.text.clone()))
        } else if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            field.request_focus();
            let back = ui.input(|i| i.modifiers.shift);
            Some(if back {
                Request::FindPrevious
            } else {
                Request::FindNext
            })
        } else if field.lost_focus() {
            return self.find_close();
        } else {
            None
        };
        if let Some(request) = request {
            self.ask(&pane.id, request);
        }
    }
}
