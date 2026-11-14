//! The look: a neutral dark palette, a few flat controls, Inter and JetBrains Mono.
use std::sync::Arc;

use crate::program::Program;
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
    emath::GuiRounding,
    epaint::text::{FontTweak, VariationCoords},
    text::{LayoutJob, TextWrapping},
};

/// The ground where no terminal lends its own.
pub const VIEW: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x20);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x32);
pub const POPOVER: Color32 = Color32::from_rgb(0x36, 0x36, 0x3a);
pub const TEXT: Color32 = Color32::WHITE;
pub const DIM: Color32 = white(140);
pub const BORDER: Color32 = white(24);
pub const ACCENT: Color32 = Color32::from_rgb(0x35, 0x84, 0xe4);
/// Something is waiting for the user.
pub const AMBER: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x2e);
/// Fills of flat controls: under the pointer, selected, and pressed.
pub const HOVER: Color32 = white(18);
pub const ACTIVE: Color32 = white(28);
pub const PRESSED: Color32 = white(46);

/// Translucent white, for text and fills that sit on any ground.
const fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha)
}

pub const TOOLBAR: f32 = 44.0;
pub const BODY: f32 = 14.5;
pub const SMALL: f32 = 12.5;
const RADIUS: u8 = 6;
pub const SMALL_BUTTON: f32 = 24.0;
/// The side of a program's icon: a tile, wider than it is tall, on a square.
const TILE: f32 = 20.0;

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-bold".into()))
}

/// What surrounds a dark terminal takes its background, so the two read as one surface.
/// Its light ink would be lost on a light background, which stays the terminal's alone.
pub fn ground(terminal: Color32) -> Color32 {
    let [r, g, b, _] = terminal.to_array().map(u32::from);
    if (r * 299 + g * 587 + b * 114) / 1000 < 64 {
        terminal
    } else {
        VIEW
    }
}

pub fn apply(ctx: &egui::Context) {
    // Each variable font serves as a regular and a bold face. egui's own fonts
    // stay behind them for symbols and emoji.
    let mut fonts = egui::FontDefinitions::default();
    let inter: &[u8] = include_bytes!("../../assets/fonts/Inter.ttf");
    let mono: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono.ttf");
    let faces = [
        (FontFamily::Proportional, "bold", inter),
        (FontFamily::Monospace, "mono-bold", mono),
    ];
    for (regular, bold, bytes) in faces {
        let fallbacks = fonts.families[&regular].clone();
        for (family, weight) in [(regular, 400.0), (FontFamily::Name(bold.into()), 700.0)] {
            let name = format!("{family} {weight}");
            let tweak = FontTweak {
                coords: VariationCoords::new([(b"wght", weight)]),
                ..Default::default()
            };
            let data = FontData::from_static(bytes).tweak(tweak);
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
        visuals.window_fill = POPOVER;
        visuals.window_stroke = Stroke::NONE;
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
        visuals.selection.stroke = Stroke::new(2.0, ACCENT.gamma_multiply(0.7));
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
        style.animation_time = 0.0;
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
    Down,
    /// A shell's, for a program whose icon is not at hand.
    Prompt,
    Dot,
    /// A dot that has been silenced.
    Muted,
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
    let fill = if response.is_pointer_button_down_on() {
        PRESSED
    } else if response.hovered() {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    let color = if ui.is_enabled() { DIM } else { white(56) };
    icon.paint(ui.painter(), rect.center(), color);
}

/// A sidebar row or a tab: flat, and filled when selected or under the pointer.
#[derive(Default)]
pub struct Item<'a> {
    pub label: &'a str,
    /// The icon of its program, ahead of the label.
    pub icon: Option<Program>,
    pub selected: bool,
    pub align: Align,
    /// A lesser row that heads the rows beneath it.
    pub caption: bool,
    /// It opens a list.
    pub menu: bool,
    /// A mark at its trailing edge.
    pub mark: Option<(Icon, Color32)>,
    /// Small buttons that take the mark's place while the pointer is on the row.
    pub buttons: &'a [(Icon, &'a str)],
    /// It can be dragged to another place among its like.
    pub movable: bool,
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
    let (rect, response) = ui.allocate_exact_size(size, sense);
    // While anything is dragged, only it is lit, and no row offers its buttons.
    let dragging = ui.ctx().dragged_id().is_some();
    let over = response.dragged() || (!dragging && ui.rect_contains_pointer(rect));
    let (fill, color) = match (item.selected, over, item.caption) {
        (true, ..) => (ACTIVE, TEXT),
        (_, true, _) => (HOVER, TEXT),
        (_, _, true) => (Color32::TRANSPARENT, DIM),
        _ => (Color32::TRANSPARENT, white(190)),
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    // Room at the trailing edge is the label's to lose, so that it never moves.
    let marks = f32::from(u8::from(item.mark.is_some()) + u8::from(item.menu));
    let room = (12.0 + 14.0 * marks).max(4.0 + BUTTON * item.buttons.len() as f32);
    let mut text = rect.shrink2(egui::vec2(12.0, 0.0));
    text.max.x = rect.right() - room;
    if item.align == Align::Center {
        text.min.x = rect.left() + room;
    }
    let lead = if item.icon.is_some() { ICON } else { 0.0 };
    // A name too long for its row ends in an ellipsis.
    let size = if item.caption { SMALL } else { BODY };
    let mut label =
        LayoutJob::simple_singleline(item.label.to_owned(), FontId::proportional(size), color);
    label.wrap = TextWrapping::truncate_at_width(text.width() - lead);
    let label = ui.painter().layout_job(label);
    let anchor = Align2([item.align, Align::Center]);
    let place = anchor.align_size_within_rect(label.size() + egui::vec2(lead, 0.0), text);
    if let Some(program) = item.icon {
        let left = egui::pos2(place.left(), rect.center().y);
        let ink = if item.selected { TEXT } else { DIM };
        tile(ui.painter(), program, left, ink);
    }
    ui.painter()
        .galley(place.min + egui::vec2(lead, 0.0), label, color);
    let mut pressed = None;
    if over && !dragging && !item.buttons.is_empty() {
        let mut right = rect.right() - 4.0;
        for (index, (icon, tip)) in item.buttons.iter().enumerate().rev() {
            let center = egui::pos2(right - BUTTON / 2.0, rect.center().y);
            let button = Rect::from_center_size(center, Vec2::splat(BUTTON));
            let button_response = ui.interact(button, response.id.with(index), Sense::click());
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
        if let Some((mark, color)) = item.mark {
            mark.paint(ui.painter(), center, color);
        }
    }
    (response, pressed)
}

/// The one prominent action of a page or dialog.
pub fn suggested(label: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(label).font(bold(BODY)).color(TEXT)).fill(ACCENT)
}
