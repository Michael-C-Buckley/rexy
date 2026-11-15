//! The row along the top that holds the tabs when there is no sidebar, and
//! the buttons both share.
use eframe::egui::{self, Align, FontId, Layout, Rect, Sense, UiBuilder, Vec2};

use super::{
    App,
    close::{Ask, CLOSE_TAB},
    commands::title,
    frame::{controls, titlebar},
    heard::{dot, worn},
    rows::Target,
    style::{
        BODY, DIM, Icon, Item, TEXT, TOOLBAR, bold, confirm, confirm_width, fitted, icon_button,
        item,
    },
};
use crate::model::{Action, View, Window};

impl App {
    /// The buttons that make another terminal: in a tab of its own, or beside
    /// the focused pane or below it. They stand together, apart from the window's own.
    fn makers(&mut self, ui: &mut egui::Ui, view: &View) {
        self.button(ui, view, Icon::Plus, "New tab", "client.tab.new");
        let splits = [
            (Icon::SplitRight, "pane.split.right"),
            (Icon::SplitDown, "pane.split.down"),
        ];
        for (icon, name) in splits {
            self.button(ui, view, icon, title(name), name);
        }
    }

    /// A button that lists the panes of the tab on show that are out of sight:
    /// behind a zoomed one, or in a floating layer that was put away. Picking one
    /// brings it out. With none, there is no button.
    pub(super) fn hidden(&mut self, ui: &mut egui::Ui, view: &View) {
        let hidden = view.current_window().map_or(&[][..], |tab| &tab.hidden);
        if hidden.is_empty() {
            return;
        }
        let button = icon_button(ui, Icon::Behind, "Panes out of sight");
        // It wears the most pressing thing any of them has to say.
        worn(ui, &button, hidden.iter().map(|pane| pane.activity).max());
        let mut picked = None;
        egui::Popup::menu(&button).width(260.0).show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for pane in hidden {
                let word = pane.word();
                let row = Item {
                    icon: Some(pane.program),
                    mark: dot(ui.ctx(), pane.activity),
                    word: word.as_deref(),
                    label: &pane.title,
                    ..Default::default()
                };
                if item(ui, Vec2::new(ui.available_width(), 32.0), row)
                    .0
                    .clicked()
                {
                    picked = Some(pane.id.clone());
                }
            }
        });
        if let Some(pane) = picked {
            self.send(Action::Reveal { pane });
        }
    }

    /// With the sidebar hidden, one row holds what it held: the sessions, behind a
    /// button, and the tabs of the one on show.
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, views: &[View]) {
        let view = &views[self.current];
        let mut bar = titlebar(ui, TOOLBAR, 8.0);
        let sidebar = "client.sidebar.toggle";
        self.button(&mut bar, view, Icon::Sidebar, "Tabs in a sidebar", sidebar);
        self.switcher(&mut bar, views);
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            controls(ui);
            ui.add_space(8.0);
            self.hidden(ui, view);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(4.0);
                let Some(session) = view.session() else {
                    return;
                };
                // Each is as wide as its name, and the wheel turns them when they
                // are too many for the row, whichever way it is turned. The row is
                // not theirs to drag: where no tab is, it is the window's handle.
                ui.style_mut().always_scroll_the_only_direction = true;
                let wheel = egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..Default::default()
                };
                // Room is kept at the end for the buttons that make another.
                let room = (ui.available_width() - 108.0).max(0.0);
                egui::ScrollArea::horizontal()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .scroll_source(wheel)
                    .max_width(room)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let size = Vec2::new(220.0, 28.0);
                        let shown = view.selected_window.as_deref();
                        self.tabs(ui, self.current, session, shown, size);
                    });
                self.makers(ui, view);
            });
        });
    }

    /// With the sidebar, the row over the terminals holds the buttons the
    /// sidebar has no room for, and says where the window is: the server, if it
    /// is not home, the session, its tab, and what is at the keyboard in it.
    /// The sidebar's rows say the same in less room, and cut short. While the
    /// sidebar lists what is at work instead, the session and its tab have no
    /// rows: the field that renames either is here, in place of its name, and
    /// so is the asking again of a tab asked to close.
    pub(super) fn header(&mut self, ui: &mut egui::Ui, view: &View) {
        let mut bar = titlebar(ui, TOOLBAR, 8.0);
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            controls(ui);
            ui.add_space(8.0);
            self.hidden(ui, view);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                self.makers(ui, view);
                let (session, tab) = (view.session(), view.current_window());
                let focused = self.focused.as_deref().and_then(|id| view.pane(id));
                let title = focused.map(|pane| pane.title.as_str());
                let plain = FontId::proportional(BODY);
                // The session and its tab, as the sidebar's rows go by, while it
                // has none of them.
                let named = session.filter(|_| self.agents).map(|session| Target {
                    server: self.current,
                    session: session.id.clone(),
                    window: None,
                });
                let tabbed = named.clone().zip(tab).map(|(named, tab)| Target {
                    window: Some(tab.id.clone()),
                    ..named
                });
                // The tab was asked to close, and asks again after the names, on
                // a button so wide.
                let asked = tabbed.clone();
                let asked = asked.filter(|tab| self.closing.waits(self.row(tab)));
                let asked = asked.map(|tab| (tab, confirm_width(ui, CLOSE_TAB)));
                let mut room = ui.available_rect_before_wrap();
                if let Some((_, width)) = asked {
                    room.max.x -= width + 10.0;
                }
                let names = [
                    (
                        (self.current > 0).then(|| self.servers[self.current].label()),
                        &plain,
                        DIM,
                        None,
                    ),
                    (
                        session.map(|session| session.name.as_str()),
                        &bold(BODY),
                        TEXT,
                        named,
                    ),
                    (tab.map(Window::name), &plain, DIM, tabbed),
                    // A tab that was given a name has this to add.
                    (
                        title.filter(|title| Some(*title) != tab.map(Window::name)),
                        &plain,
                        DIM,
                        None,
                    ),
                ];
                // They are drawn, not laid out: nothing of them takes the pointer
                // from the handle they lie on. But one being renamed gives its
                // place to the field for it.
                let mut at = room.left_center() + egui::vec2(8.0, 0.0);
                let mut field = None;
                for (name, font, color, target) in names {
                    let left = room.right() - 4.0 - at.x;
                    let Some(name) = name.filter(|_| left > 24.0) else {
                        continue;
                    };
                    let size = match target.filter(|target| self.renames(target)) {
                        Some(target) => {
                            let size = Vec2::new(left.min(260.0), 28.0);
                            let min = at - egui::vec2(0.0, size.y / 2.0);
                            field = Some((target, Rect::from_min_size(min, size)));
                            size
                        }
                        None => {
                            let name = fitted(ui, name, font.clone(), color, left);
                            let size = name.size();
                            ui.painter()
                                .galley(at - egui::vec2(0.0, size.y / 2.0), name, color);
                            size
                        }
                    };
                    at.x += size.x + 10.0;
                }
                if let Some((target, field)) = field {
                    let mut within = ui.new_child(UiBuilder::new().max_rect(field));
                    let row = self.row(&target);
                    self.renaming(&mut within, &target, row, field.size());
                }
                if let Some((target, width)) = asked {
                    let (edge, id) = (at + egui::vec2(width, 0.0), self.row(&target));
                    let asks = confirm(ui, edge, id.with("over"), CLOSE_TAB);
                    self.wait(ui, asks.rect, &asks, Ask::Again);
                    if asks.clicked() {
                        self.shut(target, tab.is_some_and(Window::idle), true);
                    }
                }
            });
        });
    }
}

/// A row of controls, such as the toolbar or a server's heading.
pub fn bar(ui: &mut egui::Ui, height: f32, inset: f32) -> egui::Ui {
    let size = Vec2::new(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let rect = rect.shrink2(egui::vec2(inset, 0.0));
    let layout = Layout::left_to_right(Align::Center);
    let mut bar = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    bar.spacing_mut().item_spacing.x = 2.0;
    bar
}
