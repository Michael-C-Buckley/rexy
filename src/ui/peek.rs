//! A look at a pane from wherever the window is: its card over the tab on
//! show, with the keyboard, so that what its program asks can be answered
//! without going to it. The pane is not taken, and stays the size it is.
use eframe::egui::{self, Align, Align2, FontId, Layout, Rect, Sense, UiBuilder, Vec2};

use super::{
    App,
    agents::place,
    pane::{PANE_HEADER, heading, hold, room},
    pointer::{Pointer, Wheel},
    render::{Grid, PaneView, rgb},
    style::{BODY, CARD, CARD_RADIUS, DIM, Icon, SMALL_BUTTON, TEXT, small_button},
};
use crate::{model::View, terminal::Request};

/// The command that looks at the next pane that wants the user.
pub const NEXT: &str = "client.activity.peek";
/// How much of the terminals' part of the window it may fill, each way.
const MOST: Vec2 = Vec2::new(0.88, 0.82);
/// The smallest its text is made to fit a terminal larger than that, as a part
/// of the size text is here. Past that it shows the foot of the terminal,
/// where what is asked is.
const LEAST: f32 = 0.6;
/// The columns and rows it makes room for until its terminal has drawn.
const UNDRAWN: Vec2 = Vec2::new(80.0, 12.0);

/// The pane looked at, with its server, and what is kept of its drawing.
pub struct Peek {
    pub server: usize,
    pub pane: String,
    drawn: PaneView,
    /// It has yet to be drawn: the click that asked for it is not one that
    /// sends it away, and its server has yet to be seen to feed it.
    new: bool,
}

impl App {
    /// Look at a pane of a server's, which is not on show.
    pub(super) fn peek(&mut self, server: usize, pane: &str) {
        // One server's is let go for another's, and the same server is only told
        // which it is now.
        if self.peek.as_ref().is_some_and(|peek| peek.server != server) {
            self.unpeek();
        }
        self.servers[server].client.peek(Some(pane));
        let drawn = PaneView::default();
        self.peek = Some(Peek { server, pane: pane.to_owned(), drawn, new: true });
    }

    /// Look at it no more.
    pub(super) fn unpeek(&mut self) {
        if let Some(peek) = self.peek.take()
            && let Some(server) = self.servers.get(peek.server)
        {
            server.client.peek(None);
        }
    }

    /// The pane looked at, over all else, with what the wheel does there.
    pub(super) fn peeking(&mut self, ctx: &egui::Context, views: &[View], within: Rect) {
        let Some(Peek { server, pane, new, .. }) = &mut self.peek else {
            return;
        };
        let (server, new) = (*server, std::mem::take(new));
        // A pane that has closed, or has come on show itself, is looked at no
        // more: nor one whose server is lost, or cannot feed it.
        let view = views.get(server).filter(|view| view.connected);
        let view = view.filter(|view| new || view.peeked.as_ref() == Some(pane));
        let shown = view.and_then(View::current_tab).filter(|_| server == self.current);
        let found = view.and_then(|view| view.find(pane));
        let found = found.filter(|(_, tab, _)| shown.is_none_or(|shown| shown.id != tab.id));
        let Some((session, tab, pane)) = found else {
            return self.unpeek();
        };
        // Its card is a pane's, around a terminal whose text is as large as the
        // window's, or as much smaller as fits it in the room there is.
        let edges = within.size() - room(within, true).size();
        let most = within.size() * MOST - edges;
        let size = self.remembered.kept.font_size;
        let cells = pane
            .frame
            .as_ref()
            .map_or(UNDRAWN, |frame| egui::vec2(f32::from(frame.cols), f32::from(frame.rows)));
        let mut grid = Grid::new(ctx, Rect::ZERO, size);
        let fit = (most / (cells * grid.cell)).min_elem();
        if fit < 1.0 {
            grid = Grid::new(ctx, Rect::ZERO, (size * fit.max(LEAST)).floor());
        }
        let all = cells * grid.cell;
        // It goes by where it is, and by its server where that is not the one
        // on show: with what its program says, and for how long it has.
        let (said, lasted) = self.told(ctx, pane);
        let mut title = place(session, tab);
        if server != self.current {
            title = format!("{} › {title}", self.servers[server].label());
        }
        if !said.is_empty() {
            title = format!("{title}  —  {said}");
        }
        let keyboard = !self.busy(ctx);
        let (mut go, mut close, mut scroll) = (false, false, None);
        let mut actions = Vec::new();
        // What is behind it steps back: the keyboard is not there for now.
        let behind = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("behind a peek"));
        let veil = egui::Color32::from_black_alpha(96);
        ctx.layer_painter(behind).rect_filled(within, CARD_RADIUS, veil);
        let area = egui::Area::new(egui::Id::unique("peek"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(within.center());
        let ground = pane.frame.as_ref().map_or(CARD, |frame| rgb(frame.bg));
        let shown = area.show(ctx, |ui| {
            let card = egui::Frame::popup(ui.style()).inner_margin(0).fill(ground);
            card.show(ui, |ui| {
                let (card, response) =
                    ui.allocate_exact_size(all.min(most) + edges, Sense::click());
                // The key that looked at it looks at the next, and closes it
                // after the last.
                let mut bar =
                    card.with_max_y(card.top() + PANE_HEADER).shrink2(egui::vec2(4.0, 0.0));
                let layout = Layout::right_to_left(Align::Center);
                let mut buttons = ui.new_child(UiBuilder::new().max_rect(bar).layout(layout));
                buttons.spacing_mut().item_spacing.x = 0.0;
                let tip = self.keys.map.tip("Next, or Close", NEXT);
                close = small_button(&mut buttons, Icon::Close, &tip).clicked();
                go = small_button(&mut buttons, Icon::Zoom, "Go to Pane").clicked();
                bar.max.x -= 2.0 * SMALL_BUTTON + 4.0;
                heading(ui, bar, pane, &title, TEXT, &lasted.unwrap_or_default());
                if keyboard {
                    hold(ui, &response);
                }
                let within = room(card, true);
                let (Some(frame), Some(peek)) = (&pane.frame, &mut self.peek) else {
                    let font = FontId::proportional(BODY);
                    let center = within.center();
                    ui.painter().text(center, Align2::CENTER_CENTER, &pane.status, font, DIM);
                    return;
                };
                // A terminal taller than the room shows its foot.
                let corner = egui::pos2(within.left(), within.bottom() - all.y);
                let grid = Grid { rect: Rect::from_min_size(corner, all), ..grid };
                // The wheel is for what it is for in the pane.
                let events = ui.input(|input| input.events.clone());
                let tracking = frame.mouse_tracking && !ui.input(|input| input.modifiers.shift);
                let pointer = Pointer {
                    ui,
                    response: &response,
                    events: &events,
                    grid,
                    frame,
                    terminal: None,
                    pane: &pane.id,
                    keyboard: &self.keyboard,
                    tracking,
                };
                scroll = pointer.wheel(Wheel::of(frame, tracking), &mut peek.drawn, &mut actions);
                peek.drawn.paint(&ui.painter_at(within), grid, frame, keyboard, pane.idle);
            });
        });
        for action in actions {
            self.tell(server, action);
        }
        if let Some(to) = scroll {
            self.ask(server, &pane.id, Request::Scroll(to));
        }
        if go {
            self.reveal(server, &pane.id);
        } else if close || (shown.response.clicked_elsewhere() && !new) {
            self.unpeek();
        }
    }
}
