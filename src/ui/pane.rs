//! One pane of the tab on show: its header, its terminal, and what the
//! pointer does there.
use eframe::egui::{self, Align, Align2, FontId, Layout, Rect, Sense, Stroke, UiBuilder};

use super::{
    App,
    close::{Ask, CLOSE, asking, closer, doomed},
    heard::dot,
    keys::SPLITS,
    pointer::{Pointer, Wheel},
    render::{Grid, rgb},
    style::{
        ACCENT, BODY, BORDER, DIM, Icon, POPOVER, SMALL, SMALL_BUTTON, TEXT, confirm, fitted,
        small_button,
    },
};
use crate::{
    link,
    model::{Action, Pane, Window},
    terminal::{Copied, Frame, Request},
};

const PANE_HEADER: f32 = 28.0;

/// A move through a pane's history. Negative is up.
pub enum Scroll {
    Lines(i32),
    Pages(i32),
    /// From one shell prompt to another, where the shell marks them (OSC 133).
    Prompts(i32),
    Bottom,
}

impl App {
    /// A pane of the tab on show. One that is not `live` shows the last screen of
    /// a connection that was lost.
    pub(super) fn terminal_pane(
        &mut self,
        ui: &mut egui::Ui,
        window: &Window,
        pane: &Pane,
        rect: Rect,
        live: bool,
    ) {
        let active = self.focused.as_deref() == Some(&pane.id);
        // The focused pane holds the keyboard whenever nothing else is taking it.
        let keyboard = active && !self.busy(ui.ctx());
        let mut actions = Vec::new();
        let (what, spared) = (doomed(&pane.id), pane.idle);
        let ask = self.closing.ask(what, spared);
        let over = ui.rect_contains_pointer(rect);
        // It was asked to close just now, with the pointer.
        let mut close = false;
        // Its header was clicked.
        let mut picked = false;
        // Its view is to move through its history.
        let mut scroll = None;
        let mut terminal_rect = rect;
        // One pane of several has a header, and so has one that fills the tab in
        // place of the rest.
        if window.split() {
            terminal_rect.min.y += PANE_HEADER;
            // A click on it takes the keyboard to the pane, as one in its terminal
            // does. Its buttons lie over it.
            let bar = rect.with_max_y(terminal_rect.top());
            let id = ui.scope_id().with(("header", &pane.id));
            picked = ui.interact(bar, id, Sense::click()).clicked();
            // The pane with the keyboard is marked along its top.
            if active {
                let line = Stroke::new(2.0, ACCENT);
                ui.painter().hline(bar.x_range(), bar.top() + 1.0, line);
            }
            let mut header = rect.shrink2(egui::vec2(4.0, 0.0));
            // The name sits a little low, toward the terminal it names.
            header.min.y += 2.0;
            header.max.y = terminal_rect.top();
            let layout = Layout::right_to_left(Align::Center);
            let mut buttons = ui.new_child(UiBuilder::new().max_rect(header).layout(layout));
            buttons.spacing_mut().item_spacing.x = 0.0;
            // Its buttons show under the pointer. The way back from a pane that
            // fills the tab shows always, where it would be among them.
            if !over || ask == Ask::Again {
                buttons.add_space(SMALL_BUTTON);
            } else if small_button(&mut buttons, Icon::Close, "Close Pane (Ctrl+Shift+W)").clicked()
            {
                close = true;
            }
            let (zoom, tip) = if window.zoomed {
                (Icon::Restore, "Restore Panes (Ctrl+Shift+Enter)")
            } else {
                (Icon::Zoom, "Zoom Pane (Ctrl+Shift+Enter)")
            };
            if (over || window.zoomed) && small_button(&mut buttons, zoom, tip).clicked() {
                let pane = pane.id.clone();
                actions.push(Action::Zoom { pane });
            }
            header.max.x -= 2.0 * SMALL_BUTTON + 4.0;
            // What it has to say sits ahead of them, as on its tab.
            if let (Some(word), Some((_, color))) = (pane.activity.word(), dot(pane.activity)) {
                let (edge, font) = (header.right_center(), FontId::proportional(SMALL));
                let word = ui
                    .painter()
                    .text(edge, Align2::RIGHT_CENTER, word, font, color);
                header.max.x = word.left() - 8.0;
            }
            // A title too long for the room left ends in an ellipsis.
            let color = if active { TEXT } else { DIM };
            let font = FontId::proportional(SMALL);
            let title = fitted(ui, &pane.title, font, color, header.width() - 8.0);
            let corner = header.left_center() + egui::vec2(8.0, -title.size().y / 2.0);
            ui.painter().galley(corner, title, color);
        }
        let state = self.panes.entry(pane.id.clone()).or_default();
        let frame = pane.frame.as_ref();
        if let Some(frame) = frame {
            ui.painter().rect_filled(terminal_rect, 0, rgb(frame.bg));
        }
        // The pane's worker, which keeps its selection. A lost connection has none.
        let terminal = self.servers[self.current].client.terminal(&pane.id).ok();
        // The selected text it was asked for has come: for the clipboard, or
        // for the desktop to paste with a middle click, in any program.
        for copied in terminal.iter().flat_map(|terminal| terminal.copied()) {
            ui.ctx().send_cmd(match copied {
                Copied::Clipboard(text) => egui::OutputCommand::CopyText(text),
                Copied::Selection(text) => egui::OutputCommand::TextSelectionSettled(text),
            });
        }
        let grid = Grid::new(
            ui.ctx(),
            terminal_rect.shrink2(egui::vec2(12.0, 8.0)),
            self.font_size,
        );
        let id = ui.scope_id().with(("terminal", &pane.id));
        let response = ui.interact(terminal_rect, id, Sense::click_and_drag());
        if keyboard {
            if !response.has_focus() {
                response.request_focus();
            }
            // Including the keys that would otherwise move it elsewhere.
            let keys = egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(id, keys));
        }
        // With Ctrl held, a link under the pointer is there to be followed, even in
        // a program that takes the mouse for itself.
        let link = frame.and_then(|frame| {
            let pointer = response
                .hover_pos()
                .filter(|_| ui.input(|i| i.modifiers.ctrl))?;
            let (x, y) = grid.cell_at(pointer, frame);
            link::at(frame, x.into(), y.into())
        });
        if response.hovered() {
            let cursor = match link {
                Some(_) => egui::CursorIcon::PointingHand,
                None => egui::CursorIcon::Text,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        // Where it leads is said in full: its text may claim anything.
        let response = match &link {
            Some(link) => response.on_hover_text_at_pointer(&link.target),
            None => response,
        };
        if response.clicked()
            && let Some(link) = &link
            && let Err(error) = link::open(&link.target)
        {
            self.toast
                .say(format!("The link could not be opened: {error}"));
        }
        let size = grid.size(ui.ctx().pixels_per_point());
        if state.last_size != Some(size) {
            let (pane, client) = (pane.id.clone(), &self.servers[self.current].client);
            // A size that could not be sent is sent when it can be.
            if client.try_send(Action::Resize { pane, size }).is_ok() {
                state.last_size = Some(size);
            }
        }
        // Shift overrides a program's mouse capture.
        let tracking = frame.is_some_and(|f| f.mouse_tracking) && !ui.input(|i| i.modifiers.shift);
        // A middle click pastes the desktop's last selection, made in any program,
        // unless a program that tracks the mouse has the click.
        let paste = |event: &egui::Event| match event {
            egui::Event::MiddleClickPaste { text, .. } => Some(text.clone()),
            _ => None,
        };
        let paste = ui.input(|i| i.events.iter().find_map(paste));
        let paste = paste.filter(|_| response.hovered() && !tracking);
        let pressed = response.clicked() || response.drag_started() || paste.is_some();
        if pressed || picked {
            self.focused = Some(pane.id.clone());
            // A pane that is this window's, and where the server has the keyboard
            // already, needs no telling.
            if !active || !pane.owner || window.focus.as_ref() != Some(&pane.id) {
                actions.push(Action::FocusPane {
                    pane: pane.id.clone(),
                });
            }
        }
        if let Some(text) = paste {
            // What is pasted is typed, and typing lets go of what is selected.
            if let Some(terminal) = &terminal {
                terminal.ask(Request::Deselect);
            }
            let pane = pane.id.clone();
            actions.push(Action::Paste { pane, text });
        }
        if let Some(frame) = frame {
            let events = ui.input(|i| i.events.clone());
            let pointer = Pointer {
                ui,
                response: &response,
                events: &events,
                grid,
                frame,
                terminal: terminal.as_ref(),
                pane: &pane.id,
                keyboard: &self.keyboard,
                tracking: tracking && link.is_none(),
            };
            pointer.forward(state, &mut actions);
            // A selection under way is seen through, whatever its program asks
            // for meanwhile.
            if !tracking || state.selecting {
                pointer.select(state);
            }
            scroll = pointer.wheel(Wheel::of(frame, tracking), state, &mut actions);
            // A screen that is no longer kept up shows a cursor nobody is at.
            state.paint(ui.painter(), grid, frame, active && live);
            for &(row, start, end) in link.iter().flat_map(|link| &link.spans) {
                let span = grid.span(row, start, end).intersect(grid.rect);
                let line = Stroke::new(1.0, rgb(frame.fg));
                ui.painter()
                    .hline(span.x_range(), span.bottom() - 1.0, line);
            }
            // Scrolled back, it says so at its foot, where a click returns it to
            // the last of its output. How far back is known only as of this
            // frame, so that is not said.
            if frame.scroll_offset > 0 {
                let font = FontId::proportional(SMALL);
                let said = ui
                    .painter()
                    .layout_no_wrap("Scrolled back".into(), font, TEXT);
                let size = said.size() + egui::vec2(20.0, 10.0);
                let pill = Rect::from_min_size(grid.rect.right_bottom() - size, size);
                let id = ui.scope_id().with(("scrolled", &pane.id));
                let back = ui
                    .interact(pill, id, Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("To the bottom (Shift+End)");
                if back.clicked() {
                    scroll = Some(Scroll::Bottom);
                }
                ui.painter().rect_filled(pill, size.y / 2.0, POPOVER);
                ui.painter()
                    .galley(pill.min + egui::vec2(10.0, 5.0), said, TEXT);
            }
            if active && response.has_focus() {
                compose(ui, grid, frame, &self.keyboard.preedit);
            }
            // And goes dim, halfway to its background.
            if !live {
                let veil = rgb(frame.bg).gamma_multiply(0.5);
                ui.painter().rect_filled(terminal_rect, 0, veil);
            }
        } else {
            ui.painter().text(
                grid.rect.center(),
                Align2::CENTER_CENTER,
                &pane.status,
                FontId::proportional(BODY),
                DIM,
            );
        }
        // One line between neighbours, none around the outside.
        let line = Stroke::new(1.0, BORDER);
        if pane.x > 0.0 {
            ui.painter().vline(rect.left(), rect.y_range(), line);
        }
        if pane.y > 0.0 {
            ui.painter().hline(rect.x_range(), rect.top(), line);
        }
        if !tracking {
            asking(&response).show(|ui| {
                let copy = egui::Button::new("Copy selection");
                let selected = frame.is_some_and(|frame| !frame.selected.is_empty());
                if ui.add_enabled(selected, copy).clicked()
                    && let Some(terminal) = &terminal
                {
                    terminal.ask(Request::Copy);
                    ui.close();
                }
                if ui.button("Paste").clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    self.focused = Some(pane.id.clone());
                    ui.close();
                }
                ui.separator();
                let pane = || pane.id.clone();
                let split = |(_, label, _, direction)| {
                    let pane = pane();
                    (label, true, Action::Split { pane, direction })
                };
                let zoom = if window.zoomed {
                    "Restore Panes"
                } else {
                    "Zoom Pane"
                };
                let zoom = (zoom, window.split(), Action::Zoom { pane: pane() });
                for (label, enabled, action) in SPLITS.into_iter().map(split).chain([zoom]) {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        actions.push(action);
                        ui.close();
                    }
                }
                close |= closer(ui, "Close Pane", ask);
            });
        }
        self.finding(ui, pane, grid.rect);
        self.wait(ui, rect, &response, ask);
        // Asked to close, it asks back from where its close button is, or would
        // be: it may have been asked with a key, and may have no header.
        if ask == Ask::Again {
            let edge = rect.right_top() + egui::vec2(-4.0, 1.0 + PANE_HEADER / 2.0);
            close |= confirm(ui, edge, what, CLOSE).clicked();
        }
        for action in actions {
            self.send(action);
        }
        if let Some(to) = scroll {
            self.scroll(&pane.id, to);
        }
        if close {
            let pane = pane.id.clone();
            self.close(what, spared, true, self.current, Action::ClosePane { pane });
        }
    }

    /// Ask something of the worker of a pane on show.
    pub(super) fn ask(&self, pane: &str, request: Request) {
        if let Ok(terminal) = self.servers[self.current].client.terminal(pane) {
            terminal.ask(request);
        }
    }

    /// Whether anything is selected in a pane, as of its last frame.
    pub(super) fn selected(&self, pane: &str) -> bool {
        let terminal = self.servers[self.current].client.terminal(pane);
        let frame = terminal.ok().and_then(|terminal| terminal.latest());
        frame.is_some_and(|frame| !frame.selected.is_empty())
    }

    /// Let go of what is selected in a pane. Its worker is not woken for nothing:
    /// this is asked at every key typed.
    pub(super) fn deselect(&mut self, pane: &str) {
        if self.selected(pane) {
            self.ask(pane, Request::Deselect);
        }
    }

    /// Copy what is selected in a pane to the clipboard. Its worker hands the
    /// text back, and the pane passes it on when next drawn.
    pub(super) fn copy(&self, pane: &str) {
        self.ask(pane, Request::Copy);
    }

    /// Select all that a pane holds, its history too.
    pub(super) fn select_all(&mut self, pane: &str) {
        self.ask(pane, Request::SelectAll);
    }

    /// Tell the pane with the keyboard when this window stops being the one in
    /// use, and when it is again, for a program there that asked to be told (DEC
    /// mode 1004). When the keyboard moves from one pane to another the server
    /// tells both of itself.
    pub(super) fn window_focus(&self, ctx: &egui::Context) {
        let turned = |event: &egui::Event| match event {
            egui::Event::WindowFocused(gained) => Some(*gained),
            _ => None,
        };
        let gained = ctx.input(|i| i.events.iter().rev().find_map(turned));
        if let (Some(gained), Some(pane)) = (gained, self.focused.clone()) {
            // A server that is not there is not told, and nothing is said of that.
            let client = &self.servers[self.current].client;
            let _ = client.try_send(Action::Focus { pane, gained });
        }
    }

    /// Move a pane's view through its history. No server is told.
    pub(super) fn scroll(&mut self, pane: &str, to: Scroll) {
        let terminal = self.servers[self.current].client.terminal(pane);
        let moved = terminal.and_then(|terminal| {
            let moved = match to {
                Scroll::Lines(lines) => terminal.scroll(lines),
                Scroll::Pages(pages) => terminal.scroll_pages(pages),
                Scroll::Prompts(prompts) => terminal.scroll_prompts(prompts),
                Scroll::Bottom => {
                    terminal.scroll_to_bottom();
                    Ok(())
                }
            };
            moved.map_err(|error| error.to_string())
        });
        if let Err(error) = moved {
            self.toast.say(error);
        }
    }
}

/// Place the IME at the terminal cursor and show its composition there.
fn compose(ui: &egui::Ui, grid: Grid, frame: &Frame, preedit: &str) {
    let (x, y) = frame.cursor.unwrap_or_default();
    let cursor = grid.span(y.into(), x.into(), usize::from(x) + 1);
    ui.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            rect: grid.rect,
            cursor_rect: cursor,
            should_interrupt_composition: false,
        })
    });
    if !preedit.is_empty() {
        let galley = ui.painter().layout_no_wrap(
            preedit.to_owned(),
            FontId::monospace(grid.font_size),
            TEXT,
        );
        let background = Rect::from_min_size(cursor.min, galley.size()).expand(3.0);
        ui.painter().rect_filled(background, 4, POPOVER);
        ui.painter().galley(cursor.min, galley, TEXT);
    }
}
