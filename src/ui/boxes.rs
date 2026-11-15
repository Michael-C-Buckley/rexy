//! The characters that draw lines and blocks, drawn as shapes that fill their
//! cells: a font's own are made for its own line height, and leave gaps between
//! the rows of any other.
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Shape, Stroke, emath::GuiRounding as _};

/// The arms of each character from U+2500 to U+257F that is made of straight
/// lines from the middle of its cell: two bits each for left, right, up and
/// down, lowest first, 1 for a light line and 2 for a heavy one.
#[rustfmt::skip]
const ARMS: [u8; 128] = [
    0x05, 0x0a, 0x50, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x48, 0x84, 0x88,
    0x41, 0x42, 0x81, 0x82, 0x14, 0x18, 0x24, 0x28, 0x11, 0x12, 0x21, 0x22, 0x54, 0x58, 0x64, 0x94,
    0xa4, 0x68, 0x98, 0xa8, 0x51, 0x52, 0x61, 0x91, 0xa1, 0x62, 0x92, 0xa2, 0x45, 0x46, 0x49, 0x4a,
    0x85, 0x86, 0x89, 0x8a, 0x15, 0x16, 0x19, 0x1a, 0x25, 0x26, 0x29, 0x2a, 0x55, 0x56, 0x59, 0x5a,
    0x65, 0x95, 0xa5, 0x66, 0x69, 0x96, 0x99, 0x6a, 0x9a, 0xa6, 0xa9, 0xaa, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x10, 0x04, 0x40, 0x02, 0x20, 0x08, 0x80, 0x09, 0x90, 0x06, 0x60,
];

/// How a character is drawn here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drawing {
    /// Lines from the middle of the cell: left, right, up and down, each none,
    /// light or heavy.
    Arms([u8; 4]),
    /// A line across or down in so many dashes, light or heavy.
    Dashes { upright: bool, count: u8, heavy: bool },
    /// A quarter circle between two sides: toward the right or not, and down or not.
    Arc { right: bool, down: bool },
    /// Part of the cell, filled: its edges as eighths of the cell, from left,
    /// top, right and bottom.
    Block([u8; 4]),
    /// The whole cell, filled so many quarters solid.
    Shade(u8),
    /// The quarters of the cell that are filled: upper left, upper right, lower
    /// left and lower right, lowest bit first.
    Quarters(u8),
}

/// How a character is drawn, if it is one of those drawn here and not with a
/// font's glyph.
pub fn drawing(c: char) -> Option<Drawing> {
    use Drawing::*;
    let code = c as u32;
    Some(match code {
        0x2504..=0x250B | 0x254C..=0x254F => Dashes {
            upright: code & 2 != 0,
            count: match code {
                0x2504..=0x2507 => 3,
                0x2508..=0x250B => 4,
                _ => 2,
            },
            heavy: code & 1 != 0,
        },
        0x256D..=0x2570 => Arc { right: matches!(code, 0x256D | 0x2570), down: code < 0x256F },
        0x2500..=0x257F => {
            let arms = ARMS[(code - 0x2500) as usize];
            // Double lines and diagonals are left to the font.
            if arms == 0 {
                return None;
            }
            Arms([0, 2, 4, 6].map(|shift| arms >> shift & 3))
        }
        0x2580 => Block([0, 0, 8, 4]),
        // From an eighth to the whole of the cell, up from its foot.
        0x2581..=0x2588 => Block([0, (0x2588 - code) as u8, 8, 8]),
        // From seven eighths to one, in from its left.
        0x2589..=0x258F => Block([0, 0, (0x2590 - code) as u8, 8]),
        0x2590 => Block([4, 0, 8, 8]),
        0x2591..=0x2593 => Shade((code - 0x2590) as u8),
        0x2594 => Block([0, 0, 8, 1]),
        0x2595 => Block([7, 0, 8, 8]),
        0x2596..=0x259F => Quarters([4, 8, 1, 13, 9, 7, 11, 2, 6, 14][(code - 0x2596) as usize]),
        _ => return None,
    })
}

/// Draw a character in its cell, in whole pixels, so that it meets its like in
/// the cells around it. What it fills goes to `fills`, which are drawn together.
/// `light` is how thick a light line is for the text's size.
pub fn paint(
    painter: &egui::Painter,
    fills: &mut Mesh,
    cell: Rect,
    drawing: Drawing,
    color: Color32,
    light: f32,
) {
    let scale = painter.pixels_per_point();
    let snap = |at: f32| at.round_to_pixels(scale);
    // A line's thickness is a whole number of pixels, and it starts on one:
    // lines of one weight are then as thick in every cell. No line has none.
    let thick = |weight: u8| {
        let pixels = (light * f32::from(weight) * scale).round();
        pixels.max(f32::from(weight.min(1))) / scale
    };
    let middle = cell.center();
    // The span a line of this weight takes across its cell's middle, along x or y.
    let across = |weight: u8, x: bool| {
        let thick = thick(weight);
        let middle = if x { middle.x } else { middle.y };
        let start = snap(middle - thick / 2.0);
        (start, start + thick)
    };
    let edges = cell.round_to_pixels(scale);
    let mut fill = |x: (f32, f32), y: (f32, f32), color: Color32| {
        let rect = Rect::from_x_y_ranges(x.0..=x.1, y.0..=y.1);
        if rect.is_positive() {
            fills.add_colored_rect(rect, color);
        }
    };
    match drawing {
        Drawing::Arms([left, right, up, down]) => {
            // Each arm reaches the far side of the arms it meets, so that a
            // corner is square whatever their weights. Where it meets none, it
            // ends at the middle.
            let (x, y) = (across(up.max(down), true), across(left.max(right), false));
            if left > 0 {
                fill((edges.left(), x.1), across(left, false), color);
            }
            if right > 0 {
                fill((x.0, edges.right()), across(right, false), color);
            }
            if up > 0 {
                fill(across(up, true), (edges.top(), y.1), color);
            }
            if down > 0 {
                fill(across(down, true), (y.0, edges.bottom()), color);
            }
        }
        Drawing::Dashes { upright, count, heavy } => {
            let weight = 1 + u8::from(heavy);
            let (start, length) = match upright {
                true => (edges.top(), edges.height()),
                false => (edges.left(), edges.width()),
            };
            // Each dash and the gap after it share an equal part of the cell. A
            // cell too small for so many, each of them two pixels long with a
            // gap after it, has as many as it has room for.
            let fit = (length * scale / 2.6).floor().max(1.0) as u8;
            let count = count.min(fit);
            let part = length / f32::from(count);
            for dash in 0..count {
                let from = snap(start + part * (f32::from(dash) + 0.2));
                let along = (from, snap(from + part * 0.6).max(from + 1.0 / scale));
                match upright {
                    true => fill(across(weight, true), along, color),
                    false => fill(along, across(weight, false), color),
                }
            }
        }
        Drawing::Arc { right, down } => {
            // From the middle of one side round to the middle of another, as
            // far out as the nearer of them lets it turn.
            let (x, y) = (across(1, true), across(1, false));
            let center = Pos2::new((x.0 + x.1) / 2.0, (y.0 + y.1) / 2.0);
            let radius = (edges.width().min(edges.height()) / 2.0).max(1.0);
            let toward = egui::vec2(if right { 1.0 } else { -1.0 }, if down { 1.0 } else { -1.0 });
            let side = if right { edges.right() } else { edges.left() };
            let end = if down { edges.bottom() } else { edges.top() };
            let turn = center + radius * toward;
            let mut points = vec![Pos2::new(center.x, end)];
            const STEPS: usize = 8;
            for step in 0..=STEPS {
                let angle = step as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
                points.push(turn - radius * toward * egui::vec2(angle.cos(), angle.sin()));
            }
            points.push(Pos2::new(side, center.y));
            painter.add(Shape::line(points, Stroke::new(thick(1), color)));
        }
        Drawing::Block([left, top, right, bottom]) => {
            let x = |eighths: u8| snap(cell.left() + cell.width() * f32::from(eighths) / 8.0);
            let y = |eighths: u8| snap(cell.top() + cell.height() * f32::from(eighths) / 8.0);
            fill((x(left), x(right)), (y(top), y(bottom)), color);
        }
        Drawing::Shade(quarters) => {
            let (x, y) = ((edges.left(), edges.right()), (edges.top(), edges.bottom()));
            fill(x, y, color.gamma_multiply(f32::from(quarters) / 4.0));
        }
        Drawing::Quarters(filled) => {
            let (x, y) = (snap(middle.x), snap(middle.y));
            let (columns, rows) =
                ([(edges.left(), x), (x, edges.right())], [(edges.top(), y), (y, edges.bottom())]);
            for quarter in 0..4 {
                if filled >> quarter & 1 != 0 {
                    fill(columns[quarter % 2], rows[quarter / 2], color);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_boxes.rs"]
mod tests;
