//! One pane of the tab on show: its header, its terminal, and what the
//! pointer does there.
use eframe::egui::{
    self, Align, Align2, CornerRadius, FontId, Layout, Rect, Sense, Stroke, StrokeKind, UiBuilder,
};
use serde_json::Value;

use super::{
    App,
    close::{Ask, CLOSE, asking, closer, doomed},
    commands::title,
    heard::dot,
    pointer::{Pointer, Wheel},
    render::{Grid, rgb},
    style::{
        BODY, BORDER, CARD, CARD_RADIUS, DIM, ICON, Icon, Item, SMALL, SMALL_BUTTON, TEXT, badge,
        confirm, eased, entry, fitted, small_button, tints,
    },
};
use crate::{
    link,
    model::{Action, Pane, View, Window},
    terminal::{Copied, Frame, Request, Scroll},
};

const PANE_HEADER: f32 = 28.0;
/// How far toward its background a pane goes that is not the one typed in, and
/// one that shows a last screen.
const AWAY: f32 = 0.2;
const LAST: f32 = 0.5;

impl App {
    /// A pane of the tab on show: a card on the window's ground, `rect`, in its
    /// terminal's own colour. It may show a last screen: of a connection that
    /// was lost, or of its own terminal where that alone is fed no more. The
    /// card may be on its way to where it comes to `rest`.
    pub(super) fn terminal_pane(
        &mut self,
        ui: &mut egui::Ui,
        view: &View,
        window: &Window,
        pane: &Pane,
        rect: Rect,
        rest: Rect,
    ) {
        let frame = pane.frame.as_ref();
        let card = frame.map_or(CARD, |frame| rgb(frame.bg));
        ui.painter().rect_filled(rect, CARD_RADIUS, card);
        // One that has only begun to come in has room for nothing yet.
        if rect != rest && rect.width().min(rect.height()) < 48.0 {
            return;
        }
        // A screen with something to say of its terminal is its last.
        let live = view.connected && (pane.frame.is_none() || pane.status.is_empty());
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
        // The way back from a pane that fills the tab is where the way to it was.
        let (zoom_icon, zoom) = match window.zoomed {
            true => (Icon::Restore, "Restore Panes"),
            false => (Icon::Zoom, title("pane.zoom")),
        };
        // One pane of several has a header, and so has one that fills the tab in
        // place of the rest.
        if window.split() {
            terminal_rect.min.y += PANE_HEADER;
            // A click on it takes the keyboard to the pane, as one in its terminal
            // does. Its buttons lie over it.
            let bar = rect.with_max_y(terminal_rect.top());
            let id = ui.scope_id().with(("header", &pane.id));
            picked = ui.interact(bar, id, Sense::click()).clicked();
            let mut header = bar.shrink2(egui::vec2(4.0, 0.0));
            let layout = Layout::right_to_left(Align::Center);
            let mut buttons = ui.new_child(UiBuilder::new().max_rect(header).layout(layout));
            buttons.spacing_mut().item_spacing.x = 0.0;
            // Its buttons show on the pane with the keyboard, and under the
            // pointer. The way back from a pane that fills the tab shows always.
            let offered = over || active;
            if !offered || ask == Ask::Again {
                buttons.add_space(SMALL_BUTTON);
            } else {
                let tip = self.keys.map.tip(title("pane.close"), "pane.close");
                close = small_button(&mut buttons, Icon::Close, &tip).clicked();
            }
            if offered || window.zoomed {
                let tip = self.keys.map.tip(zoom, "pane.zoom");
                if small_button(&mut buttons, zoom_icon, &tip).clicked() {
                    self.todo.push(("pane.zoom", Some(pane.id.clone())));
                }
            }
            header.max.x -= 2.0 * SMALL_BUTTON + 4.0;
            // What it has to say sits ahead of them, as on its tab.
            let said = dot(ui.ctx(), pane.activity).map(|(_, color)| color);
            if let Some((word, color)) = pane.word().zip(said) {
                let (edge, font) = (header.right_center(), FontId::proportional(SMALL));
                let word = ui
                    .painter()
                    .text(edge, Align2::RIGHT_CENTER, word, font, color);
                header.max.x = word.left() - 8.0;
            }
            // Its program's icon leads, with the dot of what it has to say. A
            // title too long for the room left ends in an ellipsis.
            let color = if active { TEXT } else { DIM };
            let lead = header.left_center() + egui::vec2(8.0, 0.0);
            badge(ui.painter(), pane.program, lead, DIM, said);
            let font = FontId::proportional(SMALL);
            let title = fitted(ui, &pane.title, font, color, header.width() - 16.0 - ICON);
            let corner = lead + egui::vec2(ICON, -title.size().y / 2.0);
            ui.painter().galley(corner, title, color);
        }
        let state = self.panes.entry(pane.id.clone()).or_default();
        let within = room(rect, window.split());
        // What dims the terminal, where it is dimmed, keeps to the card's corners.
        let below = CornerRadius {
            sw: CARD_RADIUS,
            se: CARD_RADIUS,
            ..CornerRadius::ZERO
        };
        let corners = match window.split() {
            true => below,
            false => CornerRadius::same(CARD_RADIUS),
        };
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
        let grid = Grid::new(ui.ctx(), within, self.remembered.kept.font_size);
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
        // The terminal is as large as the card will be at rest, from the first:
        // its program is told its size once, not all the way there.
        let settled = Grid {
            rect: room(rest, window.split()),
            ..grid
        };
        let size = settled.size(ui.ctx().pixels_per_point());
        if state.last_size != Some(size) {
            let (pane, client) = (pane.id.clone(), &self.servers[self.current].client);
            // A size that could not be sent is sent when it can be.
            if client.try_send(Action::Resize { pane, size }).is_ok() {
                state.last_size = Some(size);
            }
        }
        // Shift overrides a program's mouse capture. A last screen has no program
        // to capture it.
        let tracking = frame.is_some_and(|f| f.mouse_tracking) && !ui.input(|i| i.modifiers.shift);
        let tracking = tracking && live;
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
                    .on_hover_text(self.keys.map.tip("To the bottom", "client.scroll.bottom"));
                if back.clicked() {
                    scroll = Some(Scroll::Bottom);
                }
                ui.painter()
                    .rect_filled(pill, size.y / 2.0, ui.visuals().window_fill);
                ui.painter()
                    .galley(pill.min + egui::vec2(10.0, 5.0), said, TEXT);
            }
            if active && response.has_focus() {
                compose(ui, grid, frame, &self.keyboard.preedit);
            }
            // And goes dim, halfway to its background. One of several that is
            // not the one typed in steps back a little, too.
            // It steps back and comes forward over a moment.
            let aside = egui::Id::unique(("aside", &pane.id));
            let aside = eased(ui, aside, !active && window.split());
            let away = match live {
                false => LAST,
                true => AWAY * aside,
            };
            if away > 0.0 {
                let veil = rgb(frame.bg).gamma_multiply(away);
                ui.painter().rect_filled(terminal_rect, corners, veil);
            }
            // Under a connection that stands, nothing else says what is wrong
            // with the one terminal: it is said over its last screen.
            if !live && view.connected {
                let font = FontId::proportional(SMALL);
                let said = fitted(ui, &pane.status, font, TEXT, grid.rect.width() - 20.0);
                let size = said.size() + egui::vec2(20.0, 10.0);
                let pill = Rect::from_min_size(grid.rect.min, size);
                ui.painter()
                    .rect_filled(pill, size.y / 2.0, ui.visuals().window_fill);
                ui.painter()
                    .galley(pill.min + egui::vec2(10.0, 5.0), said, TEXT);
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
        // A line around the card: faint, or in the colour of the one in use where
        // there are several to tell it from.
        let own = egui::Id::unique(("own", &pane.id));
        let own = eased(ui, own, active && window.split());
        let line = Stroke::new(1.0, BORDER.lerp_to_gamma(tints(ui.ctx()).accent, own));
        ui.painter()
            .rect_stroke(rect, CARD_RADIUS, line, StrokeKind::Inside);
        if !tracking {
            asking(&response, 270.0, |ui| {
                let entries = [
                    ("Copy selection", "pane.copy"),
                    ("Paste", "pane.paste"),
                    (title("pane.split.right"), "pane.split.right"),
                    (title("pane.split.down"), "pane.split.down"),
                    (zoom, "pane.zoom"),
                ];
                for (label, name) in entries {
                    if name == "pane.split.right" {
                        ui.separator();
                    }
                    // Each is there to be taken when it can be done to this pane.
                    let plan = self.standing(view).plan(name, &Value::Null, Some(&pane.id));
                    let keys = self.keys.map.shortcut(name);
                    let row = Item {
                        label,
                        keys: Some(&keys),
                        ..Default::default()
                    };
                    if entry(ui, plan.is_ok(), row).clicked() {
                        self.todo.push((name, Some(pane.id.clone())));
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
        // A last screen is there to be read. Nothing done to it is sent anywhere,
        // and nothing need be said of that: the banner above it says why.
        for action in actions.into_iter().filter(|_| live) {
            self.send(action);
        }
        if let Some(to) = scroll {
            self.ask(&pane.id, Request::Scroll(to));
        }
        if close {
            self.todo.push(("pane.close", Some(pane.id.clone())));
        }
    }

    /// Ask something of the worker of a pane on show, which no server is told of.
    pub(super) fn ask(&self, pane: &str, request: Request) {
        if let Ok(terminal) = self.servers[self.current].client.terminal(pane) {
            terminal.ask(request);
        }
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
}

/// The room a card gives its terminal: what its edges leave, under its header
/// where it has one. Under a header the terminal starts at once; with none, the
/// card keeps it from its own top edge.
fn room(card: Rect, header: bool) -> Rect {
    let mut room = card.shrink2(egui::vec2(12.0, 8.0));
    room.min.y = card.top() + if header { PANE_HEADER + 2.0 } else { 8.0 };
    room
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
        ui.painter()
            .rect_filled(background, 4, ui.visuals().window_fill);
        ui.painter().galley(cursor.min, galley, TEXT);
    }
}
