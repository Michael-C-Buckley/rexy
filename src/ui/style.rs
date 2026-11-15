//! The look: a neutral dark palette, a few flat controls, Inter and JetBrains Mono.
use std::sync::Arc;

use crate::program::Program;
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
    emath::GuiRounding,
    epaint::FontColorTransferFunction,
    epaint::text::{FontTweak, VariationCoords},
    text::{LayoutJob, TextWrapping},
};

/// The window's ground where no terminal lends its own, and a card that has
/// no terminal to take its ground from.
pub const VIEW: Color32 = Color32::from_rgb(0x16, 0x16, 0x19);
pub const CARD: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0d);
pub const POPOVER: Color32 = Color32::from_rgb(0x26, 0x26, 0x2b);
pub const TEXT: Color32 = Color32::WHITE;
pub const DIM: Color32 = white(150);
pub const BORDER: Color32 = white(16);
/// The line around the window, which has to part it from whatever lies behind.
pub const EDGE_LINE: Color32 = white(34);
/// What marks the one in use: light enough to be a line or a dot on the ground.
pub const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x9c, 0xf5);
/// Darker, to carry white text.
const ACCENT_FILL: Color32 = Color32::from_rgb(0x35, 0x84, 0xe4);
/// Something is waiting for the user.
pub const AMBER: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x2e);
/// Something has failed.
pub const RED: Color32 = Color32::from_rgb(0xf6, 0x61, 0x51);
/// Something has finished.
pub const GREEN: Color32 = Color32::from_rgb(0x57, 0xe3, 0x89);
/// What follows a name and says more of the same thing: fainter than `DIM`.
const FAINT: Color32 = white(110);
/// Fills of flat controls: selected, and pressed. Under the pointer one is as
/// white as `HOVER` says, of 255.
const HOVER: f32 = 18.0;
pub const ACTIVE: Color32 = white(28);
pub const PRESSED: Color32 = white(46);
/// Fills of rows, which lie many together: faint under the pointer, so that the
/// one selected is plain among them.
const OVER: f32 = 12.0;
const SELECTED: f32 = 26.0;
/// How long a fill takes to come and to go, and a menu to appear, in seconds.
pub const EASE: f32 = 0.1;
/// The fill of what ends programs if pressed. Darker than `RED`, to carry white text.
pub const DANGER: Color32 = Color32::from_rgb(0xc0, 0x1c, 0x28);

/// Translucent white, for text and fills that sit on any ground.
const fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha)
}

/// How far a fill has come, from none to all of it, that is there while `on`:
/// it takes a moment over it either way. `id` is whose fill it is.
pub fn eased(ui: &egui::Ui, id: egui::Id, on: bool) -> f32 {
    ui.ctx().animate_bool_with_time(id, on, EASE)
}

pub const TOOLBAR: f32 = 40.0;
pub const BODY: f32 = 13.5;
pub const SMALL: f32 = 12.0;
const RADIUS: u8 = 7;
/// A card's corners, and the space between two cards and around them all.
pub const CARD_RADIUS: u8 = 9;
pub const GUTTER: f32 = 6.0;
/// The space to each side of the word in a button that asks.
const PILL: f32 = 8.0;
pub const SMALL_BUTTON: f32 = 24.0;
/// The side of a program's icon: a tile, wider than it is tall, on a square.
const TILE: f32 = 20.0;

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-bold".into()))
}

/// What surrounds a dark terminal is its background, a little lighter, so that
/// the terminal lies on it as a card of the same hue. The light ink that is
/// written there would be lost around a light terminal, which gets the plain ground.
pub fn ground(terminal: Color32) -> Color32 {
    lifted(terminal, 55).unwrap_or(VIEW)
}

/// What is laid over the window, a menu or a note, is of the same hue again and
/// lighter than the ground, to stand off it.
pub fn popover(terminal: Color32) -> Color32 {
    lifted(terminal, 115).unwrap_or(POPOVER)
}

/// A dark colour brought so many thousandths of the way to white, and no light one.
fn lifted(color: Color32, by: u32) -> Option<Color32> {
    let [r, g, b, _] = color.to_array().map(u32::from);
    let lift = |channel: u32| (channel + (255 - channel) * by / 1000) as u8;
    ((r * 299 + g * 587 + b * 114) / 1000 < 64)
        .then(|| Color32::from_rgb(lift(r), lift(g), lift(b)))
}

pub fn apply(ctx: &egui::Context) {
    // Each variable font serves as a regular and a bold face. egui's own fonts
    // stay behind them for symbols, and behind those eframe finds a font of this
    // machine's for what none of them can draw.
    let mut fonts = egui::FontDefinitions::default();
    let inter = FontData::from_static(include_bytes!("../../assets/fonts/Inter.ttf"));
    let mono = FontData::from_static(include_bytes!("../../assets/fonts/JetBrainsMono.ttf"));
    let faces = [
        (FontFamily::Proportional, "bold", inter),
        (FontFamily::Monospace, "mono-bold", mono),
    ];
    for (regular, bold, face) in faces {
        let fallbacks = fonts.families[&regular].clone();
        for (family, weight) in [(regular, 400.0), (FontFamily::Name(bold.into()), 700.0)] {
            let name = format!("{family} {weight}");
            let tweak = FontTweak {
                coords: VariationCoords::new([(b"wght", weight)]),
                ..Default::default()
            };
            let data = face.clone().tweak(tweak);
            fonts.font_data.insert(name.clone(), Arc::new(data));
            let stack = [name].into_iter().chain(fallbacks.clone()).collect();
            fonts.families.insert(family, stack);
        }
    }
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let visuals = &mut style.visuals;
        visuals.panel_fill = Color32::TRANSPARENT;
        // Light text on a dark ground is drawn heavier than its outlines by
        // default, which makes a regular weight read as a bold one. This keeps
        // it nearer what the font's designer drew.
        visuals.text_options.color_transfer_function = FontColorTransferFunction::Gamma(0.72);
        visuals.window_fill = POPOVER;
        visuals.window_stroke = Stroke::new(1.0, white(20));
        visuals.window_corner_radius = 12.into();
        visuals.menu_corner_radius = 10.into();
        visuals.popup_shadow = egui::Shadow {
            offset: [0, 4],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(96),
        };
        visuals.extreme_bg_color = ACTIVE;
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        visuals.selection.stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.8));
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        let states = [
            (&mut visuals.widgets.inactive, ACTIVE),
            (&mut visuals.widgets.hovered, white(38)),
            (&mut visuals.widgets.active, PRESSED),
            (&mut visuals.widgets.open, PRESSED),
        ];
        for (widget, fill) in states {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke = Stroke::new(1.0, TEXT);
            widget.corner_radius = RADIUS.into();
            widget.expansion = 0.0;
        }
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 34.0;
        style.text_styles = [
            (egui::TextStyle::Body, FontId::proportional(BODY)),
            (egui::TextStyle::Button, FontId::proportional(BODY)),
            (egui::TextStyle::Small, FontId::proportional(SMALL)),
            (egui::TextStyle::Monospace, FontId::monospace(13.5)),
            (egui::TextStyle::Heading, bold(24.0)),
        ]
        .into();
        style.animation_time = EASE;
    });
    // A terminal owns Ctrl+/- itself. Avoid a second, implicit zoom of the whole UI.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

#[derive(Clone, Copy)]
pub enum Icon {
    Sidebar,
    Plus,
    SplitRight,
    SplitDown,
    Retry,
    Close,
    /// Let a pane fill its tab, and give the tab back to them all.
    Zoom,
    Restore,
    /// Panes that are out of sight.
    Behind,
    Down,
    /// A shell's, for a program whose icon is not at hand.
    Prompt,
    Dot,
    /// A dot that has been silenced.
    Muted,
    /// The window's own: out of the way, and over the whole screen or back.
    Minimize,
    Maximize,
    /// A glass to find things with.
    Search,
}

impl Icon {
    fn paint(self, painter: &egui::Painter, center: Pos2, color: Color32) {
        let stroke = Stroke::new(1.5, color);
        let line = |from: [f32; 2], to: [f32; 2]| {
            painter.line_segment([center + Vec2::from(from), center + Vec2::from(to)], stroke);
        };
        let window = || {
            let rect = Rect::from_center_size(center, egui::vec2(16.0, 14.0));
            painter.rect_stroke(rect, 3, stroke, StrokeKind::Inside);
        };
        match self {
            Icon::Plus => {
                line([-6.0, 0.0], [6.0, 0.0]);
                line([0.0, -6.0], [0.0, 6.0]);
            }
            Icon::Sidebar => {
                window();
                line([-2.5, -6.0], [-2.5, 6.0]);
            }
            Icon::SplitRight => {
                window();
                line([0.0, -6.0], [0.0, 6.0]);
            }
            Icon::SplitDown => {
                window();
                line([-7.0, 0.0], [7.0, 0.0]);
            }
            Icon::Close => {
                line([-4.0, -4.0], [4.0, 4.0]);
                line([-4.0, 4.0], [4.0, -4.0]);
            }
            // Two corners, drawn apart or drawn together.
            Icon::Zoom | Icon::Restore => {
                let (corner, arm) = match self {
                    Icon::Zoom => (5.0, -4.0),
                    _ => (2.5, 4.0),
                };
                for side in [1.0, -1.0] {
                    let (x, y) = (side * corner, -side * corner);
                    line([x, y], [x + side * arm, y]);
                    line([x, y], [x, y - side * arm]);
                }
            }
            // One window, and the edges of another that shows from behind it.
            Icon::Behind => {
                let front =
                    Rect::from_center_size(center + egui::vec2(-1.5, 1.5), egui::vec2(13.0, 11.0));
                painter.rect_stroke(front, 3, stroke, StrokeKind::Inside);
                line([-4.0, -6.5], [7.5, -6.5]);
                line([7.5, -6.5], [7.5, 3.0]);
            }
            Icon::Down => {
                line([-4.0, -2.0], [0.0, 2.0]);
                line([0.0, 2.0], [4.0, -2.0]);
            }
            Icon::Prompt => {
                line([-6.0, -4.0], [-2.0, 0.0]);
                line([-2.0, 0.0], [-6.0, 4.0]);
                line([0.5, 4.5], [6.0, 4.5]);
            }
            Icon::Dot => {
                painter.circle_filled(center, 3.5, color);
            }
            Icon::Muted => {
                painter.circle_stroke(center, 3.5, stroke);
                line([-5.0, 5.0], [5.0, -5.0]);
            }
            Icon::Search => {
                painter.circle_stroke(center + egui::vec2(-1.5, -1.5), 4.5, stroke);
                line([2.0, 2.0], [6.0, 6.0]);
            }
            Icon::Minimize => line([-4.0, 3.5], [4.0, 3.5]),
            Icon::Maximize => {
                let rect = Rect::from_center_size(center, Vec2::splat(9.0));
                painter.rect_stroke(rect, 2, stroke, StrokeKind::Inside);
            }
            Icon::Retry => {
                // Most of a circle, clockwise, ending in an arrowhead on the right.
                let arc = (4..=24).map(|step| {
                    let angle = step as f32 * std::f32::consts::TAU / 24.0;
                    center + 6.0 * Vec2::angled(angle)
                });
                painter.line(arc.collect(), stroke);
                line([6.0, 0.0], [2.5, -3.0]);
                line([6.0, 0.0], [8.5, -3.5]);
            }
        }
    }
}

/// Draw a program's icon, as sharp as the screen allows. Where it is not at hand
/// a prompt stands in for it, in `ink`.
fn tile(painter: &egui::Painter, program: Program, left: Pos2, ink: Color32) {
    let Some(drawing) = program.drawing() else {
        return Icon::Prompt.paint(painter, left + egui::vec2(TILE / 2.0, 0.0), ink);
    };
    let ctx = painter.ctx();
    let scale = ctx.pixels_per_point();
    let pixels = (TILE * scale).round() as usize;
    let key = egui::Id::unique((program.name(), pixels));
    let texture = ctx.data(|data| data.get_temp::<egui::TextureHandle>(key));
    let texture = texture.unwrap_or_else(|| {
        let image = raster(drawing, pixels);
        let texture = ctx.load_texture(program.name(), image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|data| data.insert_temp(key, texture.clone()));
        texture
    });
    // One texel to a pixel: an icon that straddles pixels blurs.
    let corner = (left - egui::vec2(0.0, TILE / 2.0)).round_to_pixels(scale);
    let rect = Rect::from_min_size(corner, Vec2::splat(pixels as f32 / scale));
    let whole = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
    painter.image(texture.id(), rect, whole, Color32::WHITE);
}

/// An icon in its own colours.
fn raster(drawing: &str, pixels: usize) -> egui::ColorImage {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(drawing, &usvg::Options::default());
    let tree = tree.expect("a bundled icon is sound");
    let side = pixels as u32;
    let mut canvas = tiny_skia::Pixmap::new(side, side).expect("an icon has a size");
    let scale = pixels as f32 / tree.size().width();
    let scale = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, scale, &mut canvas.as_mut());
    let colors = canvas.pixels().iter().map(|pixel| {
        let rgba = [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()];
        Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
    });
    egui::ColorImage::new([pixels; 2], colors.collect())
}

/// The window's icon, for a desktop that takes it from the window. One that goes
/// by the desktop entry draws the same picture.
pub fn icon() -> egui::IconData {
    const SIDE: usize = 128;
    let image = raster(include_str!("../../assets/dev.rex-lite.client.svg"), SIDE);
    let rgba = image.pixels.iter().flat_map(Color32::to_srgba_unmultiplied);
    egui::IconData {
        rgba: rgba.collect(),
        width: SIDE as u32,
        height: SIDE as u32,
    }
}

/// A flat button that draws its own symbol.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    button(ui, 32.0, icon, tip)
}

/// A lesser one, as small as those of a row.
pub fn small_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    button(ui, SMALL_BUTTON, icon, tip)
}

fn button(ui: &mut egui::Ui, side: f32, icon: Icon, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    paint_button(ui, rect, &response, icon);
    response.on_hover_text(tip)
}

fn paint_button(ui: &egui::Ui, rect: Rect, response: &egui::Response, icon: Icon) {
    let lit = eased(ui, response.id.with("over"), response.hovered());
    let fill = match response.is_pointer_button_down_on() {
        true => PRESSED,
        false => white((HOVER * lit) as u8),
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    let color = if ui.is_enabled() { DIM } else { white(56) };
    icon.paint(ui.painter(), rect.center(), color);
}

/// One of the window's own buttons: round, and always there to be seen.
pub fn round_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SMALL_BUTTON), Sense::click());
    let lit = eased(ui, response.id.with("over"), response.hovered());
    let fill = match response.is_pointer_button_down_on() {
        true => PRESSED,
        false => white((20.0 + HOVER * lit) as u8),
    };
    ui.painter()
        .circle_filled(rect.center(), SMALL_BUTTON / 2.0, fill);
    icon.paint(ui.painter(), rect.center(), white(205));
    response.on_hover_text(tip)
}

/// Text on one line, which ends in an ellipsis if it is too long for `width`.
pub fn fitted(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

/// A button that asks to be pressed, to close what a press before it would not,
/// lest programs end by a slip. Its trailing edge is at `edge`, where that press was.
pub fn confirm(ui: &egui::Ui, edge: Pos2, id: egui::Id, word: &str) -> egui::Response {
    let word = ui
        .painter()
        .layout_no_wrap(word.to_owned(), bold(SMALL), TEXT);
    let size = egui::vec2(word.size().x + 2.0 * PILL, SMALL_BUTTON);
    let rect = Rect::from_min_size(edge - egui::vec2(size.x, size.y / 2.0), size);
    let response = ui.interact(rect, id, Sense::click());
    ui.painter().rect_filled(rect, RADIUS, DANGER);
    ui.painter()
        .galley(rect.center() - word.size() / 2.0, word, TEXT);
    response
}

/// A sidebar row or a tab: flat, and filled when selected or under the pointer.
#[derive(Default)]
pub struct Item<'a> {
    pub label: &'a str,
    /// What else there is to say of it, after its name and fainter, in whatever
    /// room the name leaves.
    pub more: &'a str,
    /// A name of its own, for a row that must stay itself while the rows before
    /// it come and go: its menu and its buttons go by it.
    pub id: Option<egui::Id>,
    /// The icon of its program, ahead of the label.
    pub icon: Option<Program>,
    pub selected: bool,
    pub align: Align,
    /// A lesser row that heads the rows beneath it.
    pub caption: bool,
    /// It opens a list.
    pub menu: bool,
    /// Its name is in bold.
    pub strong: bool,
    /// A mark at its trailing edge.
    pub mark: Option<(Icon, Color32)>,
    /// What the mark means, to say in its place and its colour. Only a row whose
    /// label starts at its edge has the room.
    pub word: Option<&'a str>,
    /// The keys that do what the row does, each on a key of its own at the
    /// trailing edge, joined as Rex writes them: `Ctrl+Shift+D`.
    pub keys: Option<&'a str>,
    /// Small buttons that take the mark's place while the pointer is on the row.
    pub buttons: &'a [(Icon, &'a str)],
    /// The last of them was pressed, and asks with this word to be pressed again.
    pub confirm: Option<&'a str>,
    /// More to say of the row under the pointer, after its name if that was cut short.
    pub tip: &'a str,
    /// It can be dragged to another place among its like.
    pub movable: bool,
    /// It waits to be chosen a second time, to close what may be busy.
    pub danger: bool,
    /// How far in what it says starts, for a row that belongs to the one above.
    pub inset: f32,
}

/// Draw a row. Returns its response, and which of its buttons was pressed.
pub fn item(ui: &mut egui::Ui, size: Vec2, item: Item<'_>) -> (egui::Response, Option<usize>) {
    const BUTTON: f32 = SMALL_BUTTON;
    const ICON: f32 = TILE + 7.0;
    let sense = if item.movable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = match item.id {
        Some(id) => {
            let (_, rect) = ui.allocate_space(size);
            (rect, ui.interact(rect, id, sense))
        }
        None => ui.allocate_exact_size(size, sense),
    };
    // While anything is dragged, only it is lit, and no row offers its buttons.
    let dragging = ui.ctx().dragged_id().is_some();
    let over = response.dragged() || (!dragging && ui.rect_contains_pointer(rect));
    // One that cannot be taken now does not answer the pointer.
    let over = over && ui.is_enabled();
    // Its fill comes and goes over a moment.
    let lit = eased(ui, response.id.with("over"), over);
    let chosen = eased(ui, response.id.with("selected"), item.selected);
    let fill = match item.danger {
        true => DANGER,
        false => white((SELECTED * chosen).max(OVER * lit) as u8),
    };
    let color = match (item.selected || over || item.danger, item.caption) {
        (true, _) => TEXT,
        (_, true) => DIM,
        _ => white(190),
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    // Room at the trailing edge is the label's to lose, so that it never moves.
    let word = item.word.filter(|_| item.align != Align::Center);
    let word = word.zip(item.mark).map(|(word, (_, color))| {
        let font = FontId::proportional(SMALL);
        ui.painter().layout_no_wrap(word.to_owned(), font, color)
    });
    let keys = caps(ui, item.keys.unwrap_or_default());
    let marks = f32::from(u8::from(item.mark.is_some()) + u8::from(item.menu));
    let marks = word
        .as_ref()
        .map_or(14.0 * marks, |word| word.size().x + 8.0);
    let marks = marks + keys.iter().map(|name| cap(name).x + 4.0).sum::<f32>();
    // The button that asks is where the last one was, and goes by its name: the
    // pointer is on it already.
    let last = item.buttons.len().checked_sub(1);
    let asks = item.confirm.zip(last).map(|(word, last)| {
        let edge = egui::pos2(rect.right() - 4.0, rect.center().y);
        confirm(ui, edge, response.id.with(("button", last)), word)
    });
    let room = (12.0 + marks).max(4.0 + BUTTON * item.buttons.len() as f32);
    let room = asks
        .as_ref()
        .map_or(room, |asks| room.max(asks.rect.width() + 10.0));
    let mut text = rect.shrink2(egui::vec2(12.0, 0.0));
    text.min.x += item.inset;
    text.max.x = rect.right() - room;
    let lead = if item.icon.is_some() { ICON } else { 0.0 };
    // A name too long for its row ends in an ellipsis.
    let font = match (item.caption, item.strong) {
        (true, _) => bold(SMALL),
        (_, true) => bold(BODY),
        _ => FontId::proportional(BODY),
    };
    let label = fitted(ui, item.label, font, color, text.width() - lead);
    // What else it has to say has the room its name leaves, if that is any.
    let left = text.width() - lead - label.size().x - 8.0;
    let more = (left > 24.0 && !item.more.is_empty())
        .then(|| fitted(ui, item.more, FontId::proportional(BODY), FAINT, left));
    let lost = !item.more.is_empty() && more.as_ref().is_none_or(|more| more.elided);
    let cut = label.elided;
    let needs = label.size() + egui::vec2(lead, 0.0);
    // A centred name is at the middle of its row while it fits there, and has
    // all the room there is when it does not.
    let middle = text.with_min_x(rect.left() + room);
    if item.align == Align::Center && needs.x <= middle.width() {
        text = middle;
    }
    let anchor = Align2([item.align, Align::Center]);
    let place = anchor.align_size_within_rect(needs, text);
    if let Some(program) = item.icon {
        let left = egui::pos2(place.left(), rect.center().y);
        let ink = if item.selected { ACCENT } else { DIM };
        tile(ui.painter(), program, left, ink);
    }
    let name = place.min + egui::vec2(lead, 0.0);
    if let Some(more) = more {
        let after = name + egui::vec2(label.size().x + 8.0, 0.0);
        ui.painter().galley(after, more, FAINT);
    }
    ui.painter().galley(name, label, color);
    let mut pressed = None;
    if let Some(asks) = asks {
        pressed = last.filter(|_| asks.clicked());
    } else if over && !dragging && !item.buttons.is_empty() {
        let mut right = rect.right() - 4.0;
        for (index, (icon, tip)) in item.buttons.iter().enumerate().rev() {
            let center = egui::pos2(right - BUTTON / 2.0, rect.center().y);
            let button = Rect::from_center_size(center, Vec2::splat(BUTTON));
            // By a name of its own: a row's tooltip goes by the row's and a number.
            let id = response.id.with(("button", index));
            let button_response = ui.interact(button, id, Sense::click());
            paint_button(ui, button, &button_response, *icon);
            if button_response.on_hover_text(*tip).clicked() {
                pressed = Some(index);
            }
            right -= BUTTON;
        }
    } else {
        let mut center = egui::pos2(rect.right() - 16.0, rect.center().y);
        if item.menu {
            Icon::Down.paint(ui.painter(), center, DIM);
            center.x -= 14.0;
        }
        if let Some(word) = word {
            let corner = egui::pos2(rect.right() - 12.0, rect.center().y)
                - word.size() * egui::vec2(1.0, 0.5);
            ui.painter().galley(corner, word, TEXT);
        } else if let Some((mark, color)) = item.mark {
            mark.paint(ui.painter(), center, color);
        }
        paint_caps(ui, keys, egui::pos2(rect.right() - 10.0, rect.center().y));
    }
    let response = if cut || lost || !item.tip.is_empty() {
        response.on_hover_ui(|ui| {
            if cut {
                ui.label(item.label);
            }
            if lost {
                ui.label(item.more);
            }
            if !item.tip.is_empty() {
                ui.label(item.tip);
            }
        })
    } else {
        response
    };
    (response, pressed)
}

/// The keys of a shortcut, each to go on a key of its own, as Rex joins them:
/// `Ctrl+Shift+D`.
fn caps(ui: &egui::Ui, keys: &str) -> Vec<Arc<egui::Galley>> {
    let mut names: Vec<_> = keys.split('+').filter(|key| !key.is_empty()).collect();
    // The plus key itself, which is also what joins the others.
    if keys.ends_with('+') {
        names.push("+");
    }
    let font = FontId::proportional(11.0);
    let key = |key: &str| {
        ui.painter()
            .layout_no_wrap(key.to_owned(), font.clone(), white(205))
    };
    names.into_iter().map(key).collect()
}

/// The room a key takes.
fn cap(name: &egui::Galley) -> Vec2 {
    egui::vec2(name.size().x + 12.0, 18.0)
}

/// Draw the keys of a shortcut, the last of them ending at `right`.
fn paint_caps(ui: &egui::Ui, keys: Vec<Arc<egui::Galley>>, mut right: Pos2) {
    for name in keys.into_iter().rev() {
        let size = cap(&name);
        let key = Rect::from_min_size(right - egui::vec2(size.x, 9.0), size);
        ui.painter().rect_filled(key, 5, white(22));
        ui.painter()
            .galley(key.center() - name.size() / 2.0, name, TEXT);
        right.x -= size.x + 4.0;
    }
}

/// What opens the palette, for the pointer: as wide as the room there is, it
/// says what it is for, and the keys that do the same where they fit.
pub fn search(ui: &mut egui::Ui, keys: &str) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let lit = eased(ui, response.id.with("over"), response.hovered());
    ui.painter()
        .rect_filled(rect, RADIUS, white((OVER + OVER * lit) as u8));
    let left = rect.left_center();
    Icon::Search.paint(ui.painter(), left + egui::vec2(16.0, 0.0), DIM);
    let font = FontId::proportional(BODY);
    let word = ui.painter().layout_no_wrap("Search".to_owned(), font, DIM);
    let keys = caps(ui, keys);
    let wide = keys.iter().map(|name| cap(name).x + 4.0).sum::<f32>();
    if 32.0 + word.size().x + 12.0 + wide <= rect.width() {
        paint_caps(ui, keys, rect.right_center() - egui::vec2(5.0, 0.0));
    }
    let at = left + egui::vec2(32.0, -word.size().y / 2.0);
    ui.painter().galley(at, word, DIM);
    response
}

/// An entry of a menu: a row as wide as the menu, with the keys that do the same
/// at its far edge. One that cannot be taken now is faint.
pub fn entry(ui: &mut egui::Ui, enabled: bool, row: Item<'_>) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 30.0);
    ui.add_enabled_ui(enabled, |ui| item(ui, size, row).0).inner
}

/// The one prominent action of a page or dialog.
pub fn suggested(label: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(label).font(bold(BODY)).color(TEXT)).fill(ACCENT_FILL)
}

#[cfg(test)]
#[path = "../../tests/unit/ui_style.rs"]
mod tests;
