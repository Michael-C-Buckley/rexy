//! The look: a neutral dark palette, a few flat controls, Inter and JetBrains Mono.
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
    epaint::text::{FontTweak, VariationCoords},
};

/// The ground where no terminal lends its own.
pub const VIEW: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x20);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x32);
pub const POPOVER: Color32 = Color32::from_rgb(0x36, 0x36, 0x3a);
pub const TEXT: Color32 = Color32::WHITE;
pub const DIM: Color32 = white(140);
pub const BORDER: Color32 = white(24);
pub const ACCENT: Color32 = Color32::from_rgb(0x35, 0x84, 0xe4);
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

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-bold".into()))
}

/// The toolbar takes a dark terminal's background, so the two read as one surface.
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
        }
    }
}

/// A flat button that draws its own symbol.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
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
    response.on_hover_text(tip)
}

/// A sidebar row or a tab: flat, and filled when selected or under the pointer.
pub fn item(
    ui: &mut egui::Ui,
    size: Vec2,
    label: &str,
    selected: bool,
    align: Align,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (fill, color) = if selected {
        (ACTIVE, TEXT)
    } else if response.hovered() {
        (HOVER, TEXT)
    } else {
        (Color32::TRANSPARENT, white(190))
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    let text = rect.shrink2(egui::vec2(12.0, 0.0));
    let anchor = Align2([align, Align::Center]);
    ui.painter().with_clip_rect(text).text(
        anchor.pos_in_rect(&text),
        anchor,
        label,
        FontId::proportional(BODY),
        color,
    );
    response
}

/// The one prominent action of a page or dialog.
pub fn suggested(label: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(label).font(bold(BODY)).color(TEXT)).fill(ACCENT)
}
