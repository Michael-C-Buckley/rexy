//! Per-pane view state and painting. Shaped text is cached by row identity.
use std::{collections::HashMap, sync::Arc, time::Duration};

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use libghostty_vt::style::Underline;

use super::{boxes, style};
use crate::{
    model::Size,
    terminal::{CursorShape, Frame, Row},
};

pub fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// How long a blinking cursor is lit, and as long unlit, in seconds: Ghostty's own.
const BLINK: f64 = 0.6;
/// How far, in points, a glyph may reach past its cells before it is clipped.
const SPILL: f32 = 1.0;

/// Where a pane's cells are on screen.
#[derive(Clone, Copy)]
pub struct Grid {
    pub rect: Rect,
    pub cell: Vec2,
    pub font_size: f32,
}

impl Grid {
    pub fn new(ctx: &egui::Context, rect: Rect, font_size: f32) -> Self {
        let font = FontId::monospace(font_size);
        let width = ctx.fonts_mut(|f| f.glyph_width(&font, 'M'));
        Self {
            rect,
            cell: egui::vec2(width, (font_size * 1.35).ceil()),
            font_size,
        }
    }

    /// How many cells fit, and a cell's size in physical pixels.
    pub fn size(&self, pixels_per_point: f32) -> Size {
        Size {
            cols: (self.rect.width() / self.cell.x).clamp(2.0, 1000.0) as u16,
            rows: (self.rect.height() / self.cell.y).clamp(1.0, 1000.0) as u16,
            cell_width: (self.cell.x * pixels_per_point).ceil() as u16,
            cell_height: (self.cell.y * pixels_per_point).ceil() as u16,
        }
    }

    /// The column and row under a position, clamped to the frame.
    pub fn cell_at(&self, pos: Pos2, frame: &Frame) -> (u16, u16) {
        let offset = (pos - self.rect.min) / self.cell;
        (
            (offset.x.max(0.0) as u16).min(frame.cols.saturating_sub(1)),
            (offset.y.max(0.0) as u16).min(frame.rows.saturating_sub(1)),
        )
    }

    /// The screen area of columns `start..end` in one row.
    pub fn span(&self, row: usize, start: usize, end: usize) -> Rect {
        Rect::from_min_size(
            self.rect.min + self.cell * egui::vec2(start as f32, row as f32),
            egui::vec2((end - start) as f32 * self.cell.x, self.cell.y),
        )
    }
}

#[derive(Default)]
pub struct PaneView {
    pub last_size: Option<Size>,
    /// The button that selects is down, on a press this pane took.
    pub selecting: bool,
    pub wheel: f32,
    pub mouse_button: Option<egui::PointerButton>,
    pub mouse_cell: Option<(u16, u16)>,
    /// Where the cursor was last drawn, and when it came to be there.
    cursor: Option<((u16, u16), f64)>,
    /// Shaped rows, by the address of the row each was shaped from, and holds.
    cached_rows: HashMap<usize, CachedRow>,
    /// A galley of egui's own. The cached rows are good while egui still holds it.
    cached_while: Option<Arc<egui::Galley>>,
}

struct CachedRow {
    /// Held so that no other row comes to live at its address.
    _source: Arc<Row>,
    glyphs: Vec<GlyphRun>,
    backgrounds: Vec<(usize, usize, Color32)>,
    /// The characters that draw lines and blocks, each by its column: they are
    /// drawn to fill their cells, which a font's glyphs for them do not.
    boxes: Vec<(usize, boxes::Drawing, Color32)>,
    /// Columns, the height in the cell as a fraction, and the kind of line.
    decorations: Vec<(usize, usize, f32, Color32, Underline)>,
}

struct GlyphRun {
    column: usize,
    width: usize,
    galley: Arc<egui::Galley>,
}

impl PaneView {
    /// Draw a terminal's screen. `focused` says the keyboard is in it, and
    /// `prompt` that its shell is at its prompt. A cursor that blinks asks to
    /// be drawn again when it next changes.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        grid: Grid,
        frame: &Frame,
        focused: bool,
        prompt: bool,
    ) {
        // A galley is shaped for one font size and display scale, and points into
        // egui's font atlas. egui drops every galley it holds when the fonts or the
        // atlas are made anew, and gives a new one for a new size or scale. So the
        // cached rows last for as long as it hands back the same one.
        let font = FontId::monospace(grid.font_size);
        let kept = painter.layout_no_wrap("M".into(), font, Color32::PLACEHOLDER);
        if !(self.cached_while.as_ref()).is_some_and(|cached| Arc::ptr_eq(cached, &kept)) {
            self.cached_rows.clear();
            self.cached_while = Some(kept);
        }
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(grid.rect));
        let (fg, bg) = (rgb(frame.fg), rgb(frame.bg));
        let visible = ((grid.rect.height() / grid.cell.y).ceil() as usize).min(frame.lines.len());
        let selection = style::tints(painter.ctx()).accent.gamma_multiply(0.4);
        // What the characters that draw lines and blocks fill, and how thick a
        // light line of theirs is.
        let mut fills = egui::Mesh::default();
        let light = (grid.font_size / 14.0).max(1.0 / painter.pixels_per_point());
        // A row keeps its shaped text wherever on screen it moves to, and loses it
        // when it is shown no more.
        let mut shaped = std::mem::take(&mut self.cached_rows);
        for (y, row) in frame.lines.iter().take(visible).enumerate() {
            let source = Arc::as_ptr(row).addr();
            let cached = shaped
                .remove(&source)
                .unwrap_or_else(|| CachedRow::new(painter.ctx(), row, grid.font_size));
            let cached = self.cached_rows.entry(source).or_insert(cached);
            for &(start, end, color) in &cached.backgrounds {
                if color != bg {
                    painter.rect_filled(grid.span(y, start, end), 0, color);
                }
            }
            if let Some(&Some((start, end))) = frame.selected.get(y) {
                painter.rect_filled(grid.span(y, start.into(), end.into()), 0, selection);
            }
            for &(column, drawing, color) in &cached.boxes {
                let cell = grid.span(y, column, column + 1);
                boxes::paint(&painter, &mut fills, cell, drawing, color, light);
            }
            for glyph in &cached.glyphs {
                let cells = grid.span(y, glyph.column, glyph.column + glyph.width);
                let pos = Pos2::new(
                    cells.left(),
                    cells.top() + (grid.cell.y - glyph.galley.size().y) * 0.5,
                );
                // Each clip of its own is a draw call of its own, so only a glyph
                // from a font too wide for its cells gets one. Slanted text is as
                // wide as its cells, and may lean into the one that follows.
                let limit = cells.width() + SPILL;
                let galley = Arc::clone(&glyph.galley);
                if galley.size().x > limit && galley.mesh_bounds.right() > limit {
                    let clip = painter.clip_rect().intersect(cells);
                    painter.with_clip_rect(clip).galley(pos, galley, fg);
                } else {
                    painter.galley(pos, galley, fg);
                }
            }
            for &(start, end, height, color, kind) in &cached.decorations {
                let span = grid.span(y, start, end);
                let level = span.top() + height * grid.cell.y;
                let cell = grid.cell.x;
                let line = [
                    Pos2::new(span.left(), level),
                    Pos2::new(span.right(), level),
                ];
                let stroke = Stroke::new(1.0, color);
                // Each pattern repeats with the cell, so runs side by side join up.
                match kind {
                    Underline::Curly => {
                        let wave = |step: usize| {
                            let along = step as f32 / 8.0;
                            let rise = (along * std::f32::consts::TAU).sin() * grid.cell.y * 0.07;
                            Pos2::new(span.left() + along * cell, level - rise)
                        };
                        let points = (0..=(end - start) * 8).map(wave).collect();
                        painter.add(Shape::line(points, stroke));
                    }
                    Underline::Dotted => {
                        let first = [line[0] + egui::vec2(cell / 4.0, 0.0), line[1]];
                        painter.extend(Shape::dotted_line(&first, color, cell / 2.0, 0.75));
                    }
                    Underline::Dashed => {
                        let (dash, gap) = ([cell / 2.0], [cell / 2.0]);
                        let dashes =
                            Shape::dashed_line_with_offset(&line, stroke, &dash, &gap, cell / 4.0);
                        painter.extend(dashes);
                    }
                    _ => {
                        painter.line_segment(line, stroke);
                    }
                }
            }
        }
        if !fills.is_empty() {
            painter.add(fills);
        }
        if let Some((x, y)) = frame.cursor {
            let rect = grid.span(y.into(), x.into(), usize::from(x) + 1);
            let color = rgb(frame.cursor_color);
            let outline = Stroke::new(1.0, color);
            // A shell at its prompt has a bar, as Ghostty gives it there. A
            // program has the shape it asks for, and a block if it asks for none.
            let shape = match frame.cursor_shape {
                CursorShape::Block if prompt => CursorShape::Bar,
                shape => shape,
            };
            // It blinks where its program asks that or nothing of it, and at a
            // prompt, while the keyboard and the window are here. It is lit as
            // it moves.
            let (now, here) = painter.ctx().input(|i| (i.time, i.focused));
            let since = match self.cursor {
                Some((at, since)) if at == (x, y) => since,
                _ => now,
            };
            self.cursor = Some(((x, y), since));
            let still = now - since;
            if focused && here && (frame.cursor_blink || prompt) {
                let left = Duration::from_secs_f64(BLINK - still % BLINK);
                painter.ctx().request_repaint_after(left);
                if (still / BLINK) as u64 % 2 == 1 {
                    return;
                }
            }
            match shape {
                CursorShape::Bar if focused => {
                    painter.rect_filled(rect.with_max_x(rect.left() + 2.0), 0, color);
                }
                CursorShape::Underline if focused => {
                    painter.rect_filled(rect.with_min_y(rect.bottom() - 2.0), 0, color);
                }
                // Solid, as Ghostty's is, with what is under it in the colour of
                // the ground.
                CursorShape::Block if focused => {
                    painter.rect_filled(rect, 0, color);
                    let column = usize::from(x);
                    let row = frame.lines.get(usize::from(y));
                    let under = row.and_then(|row| Some((row, row.cells.get(column)?)));
                    if let Some((row, cell)) = under {
                        let font = match row.look(cell).style.bold {
                            true => style::mono_bold(grid.font_size),
                            false => FontId::monospace(grid.font_size),
                        };
                        let (text, ink) = (row.text(column, column + 1), rgb(frame.bg));
                        let galley = painter.layout_no_wrap(text.to_owned(), font, ink);
                        let down = (grid.cell.y - galley.size().y) * 0.5;
                        painter.galley(rect.min + egui::vec2(0.0, down), galley, ink);
                    }
                }
                // An unfocused pane always shows a hollow cursor.
                _ => {
                    painter.rect_stroke(rect, 0, outline, egui::StrokeKind::Inside);
                }
            }
        }
    }
}

impl CachedRow {
    fn new(ctx: &egui::Context, row: &Arc<Row>, font_size: f32) -> Self {
        let mut result = Self {
            _source: Arc::clone(row),
            glyphs: Vec::new(),
            backgrounds: Vec::new(),
            boxes: Vec::new(),
            decorations: Vec::new(),
        };
        let mut x = 0;
        for run in row.cells.chunk_by(|a, b| row.look(a).bg == row.look(b).bg) {
            let ground = rgb(row.look(&run[0]).bg);
            result.backgrounds.push((x, x + run.len(), ground));
            x += run.len();
        }
        x = 0;
        while x < row.cells.len() {
            let cell = &row.cells[x];
            if cell.spacer_tail {
                x += 1;
                continue;
            }
            let start = x;
            x += 1;
            // Monospace ASCII can be shaped in a single run. Fallback glyphs and
            // grapheme clusters are individually positioned on Ghostty's cell grid.
            // A cell whose text is one byte long holds one ASCII character.
            let ascii = |at: usize| row.text(at, at + 1).len() == 1;
            if ascii(start) {
                while ascii(x) && row.cells[x].look == cell.look {
                    x += 1;
                }
            } else if row.cells.get(x).is_some_and(|c| c.spacer_tail) {
                x += 1;
            }
            let text = row.text(start, x);
            let look = row.look(cell);
            let color = if look.style.faint {
                rgb(look.fg).gamma_multiply(0.65)
            } else {
                rgb(look.fg)
            };
            let mut letters = text.chars();
            let alone = letters.next().filter(|_| letters.next().is_none());
            if let Some(drawing) = alone.and_then(boxes::drawing) {
                result.boxes.push((start, drawing, color));
            } else if !text.trim().is_empty() {
                let font_id = if look.style.bold {
                    style::mono_bold(font_size)
                } else {
                    FontId::monospace(font_size)
                };
                let format = egui::TextFormat {
                    font_id,
                    color,
                    italics: look.style.italic,
                    ..Default::default()
                };
                let mut job = egui::text::LayoutJob::default();
                job.append(text, 0.0, format);
                result.glyphs.push(GlyphRun {
                    column: start,
                    width: x - start,
                    galley: ctx.fonts_mut(|f| f.layout_job(job)),
                });
            }
            let underline = look.underline_color.map_or(color, rgb);
            let kind = look.style.underline;
            if kind != Underline::None {
                result.decorations.push((start, x, 0.90, underline, kind));
                if kind == Underline::Double {
                    result.decorations.push((start, x, 0.78, underline, kind));
                }
            }
            if look.style.strikethrough {
                result
                    .decorations
                    .push((start, x, 0.51, color, Underline::Single));
            }
            if look.style.overline {
                result
                    .decorations
                    .push((start, x, 0.05, color, Underline::Single));
            }
        }
        result
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_render.rs"]
mod tests;
