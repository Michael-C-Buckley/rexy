//! Per-pane view state and painting. Shaped text is cached by row identity.
use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use libghostty_vt::style::Underline;

use super::style;
use crate::{
    model::Size,
    terminal::{CursorShape, Frame, Row},
};

pub fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

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

    /// The same cell as a flat index, as selections use.
    pub fn index_at(&self, pos: Pos2, frame: &Frame) -> usize {
        let (x, y) = self.cell_at(pos, frame);
        usize::from(y) * usize::from(frame.cols).max(1) + usize::from(x)
    }

    /// The screen area of columns `start..end` in one row.
    pub fn span(&self, row: usize, start: usize, end: usize) -> Rect {
        Rect::from_min_size(
            self.rect.min + self.cell * egui::vec2(start as f32, row as f32),
            egui::vec2((end - start) as f32 * self.cell.x, self.cell.y),
        )
    }
}

/// A selection holds the frame it was made on until it is cleared.
pub struct Selection {
    /// Inclusive flat cell bounds, in the order they were dragged.
    pub range: (usize, usize),
    pub frame: Arc<Frame>,
}

impl Selection {
    pub fn text(&self) -> String {
        self.frame.selected_text(self.range.0, self.range.1)
    }
}

#[derive(Default)]
pub struct PaneView {
    pub last_size: Option<Size>,
    pub selection: Option<Selection>,
    pub wheel: f32,
    pub mouse_button: Option<egui::PointerButton>,
    pub mouse_cell: Option<(u16, u16)>,
    cached_rows: Vec<CachedRow>,
    cached_cell: Option<Vec2>,
}

struct CachedRow {
    source: Arc<Row>,
    glyphs: Vec<GlyphRun>,
    backgrounds: Vec<(usize, usize, Color32)>,
    decorations: Vec<(usize, usize, f32, Color32)>,
}

struct GlyphRun {
    column: usize,
    width: usize,
    galley: Arc<egui::Galley>,
}

impl PaneView {
    pub fn paint(&mut self, painter: &egui::Painter, grid: Grid, frame: &Frame, focused: bool) {
        if self.cached_cell != Some(grid.cell) {
            self.cached_rows.clear();
            self.cached_cell = Some(grid.cell);
        }
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(grid.rect));
        let (fg, bg) = (rgb(frame.fg), rgb(frame.bg));
        let cols = usize::from(frame.cols);
        let selection = self.selection.as_ref().map(|s| s.range);
        let visible = ((grid.rect.height() / grid.cell.y).ceil() as usize).min(frame.lines.len());
        self.cached_rows.truncate(visible);
        for (y, row) in frame.lines.iter().take(visible).enumerate() {
            let cached = self.cached_rows.get(y);
            if cached.is_none_or(|cached| !Arc::ptr_eq(&cached.source, row)) {
                let cached = CachedRow::new(painter.ctx(), row, grid.font_size);
                match self.cached_rows.get_mut(y) {
                    Some(stale) => *stale = cached,
                    None => self.cached_rows.push(cached),
                }
            }
            let cached = &self.cached_rows[y];
            for &(start, end, color) in &cached.backgrounds {
                if color != bg {
                    painter.rect_filled(grid.span(y, start, end), 0, color);
                }
            }
            if let Some((a, b)) = selection {
                let start = a.min(b).max(y * cols);
                let end = a.max(b).saturating_add(1).min((y + 1) * cols);
                if start < end {
                    painter.rect_filled(
                        grid.span(y, start - y * cols, end - y * cols),
                        0,
                        style::ACCENT.gamma_multiply(0.4),
                    );
                }
            }
            for glyph in &cached.glyphs {
                let clip = grid.span(y, glyph.column, glyph.column + glyph.width);
                let pos = Pos2::new(
                    clip.left(),
                    clip.top() + (grid.cell.y - glyph.galley.size().y) * 0.5,
                );
                let clipped = painter.with_clip_rect(painter.clip_rect().intersect(clip));
                clipped.galley(pos, Arc::clone(&glyph.galley), fg);
            }
            for &(start, end, height, color) in &cached.decorations {
                let span = grid.span(y, start, end);
                let level = span.top() + height * grid.cell.y;
                painter.line_segment(
                    [
                        Pos2::new(span.left(), level),
                        Pos2::new(span.right(), level),
                    ],
                    Stroke::new(1.0, color),
                );
            }
        }
        if selection.is_some() {
            let text = painter.layout_no_wrap(
                "Selection · Esc to resume".into(),
                FontId::proportional(style::SMALL),
                style::TEXT,
            );
            let rect = Rect::from_min_size(
                grid.rect.right_top() - egui::vec2(text.size().x + 20.0, 0.0),
                text.size() + egui::vec2(20.0, 10.0),
            );
            painter.rect_filled(rect, rect.height() / 2.0, style::POPOVER);
            painter.galley(rect.min + egui::vec2(10.0, 5.0), text, style::TEXT);
        } else if let Some((x, y)) = frame.cursor {
            let rect = grid.span(y.into(), x.into(), usize::from(x) + 1);
            let color = rgb(frame.cursor_color);
            let outline = Stroke::new(1.0, color);
            match frame.cursor_shape {
                CursorShape::Bar if focused => {
                    painter.rect_filled(rect.with_max_x(rect.left() + 2.0), 0, color);
                }
                CursorShape::Underline if focused => {
                    painter.rect_filled(rect.with_min_y(rect.bottom() - 2.0), 0, color);
                }
                CursorShape::Block if focused => {
                    painter.rect_filled(rect, 0, color.gamma_multiply(0.45));
                    painter.rect_stroke(rect, 0, outline, egui::StrokeKind::Inside);
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
            source: Arc::clone(row),
            glyphs: Vec::new(),
            backgrounds: Vec::new(),
            decorations: Vec::new(),
        };
        let mut x = 0;
        for run in row.cells.chunk_by(|a, b| a.bg == b.bg) {
            result.backgrounds.push((x, x + run.len(), rgb(run[0].bg)));
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
            let mut text = cell.text.clone();
            x += 1;
            // Monospace ASCII can be shaped in a single run. Fallback glyphs and
            // grapheme clusters are individually positioned on Ghostty's cell grid.
            if text.len() == 1 && text.is_ascii() {
                while let Some(next) = row.cells.get(x)
                    && !next.spacer_tail
                    && next.text.len() == 1
                    && next.text.is_ascii()
                    && next.fg == cell.fg
                    && next.style == cell.style
                {
                    text.push_str(&next.text);
                    x += 1;
                }
            } else if row.cells.get(x).is_some_and(|c| c.spacer_tail) {
                x += 1;
            }
            let color = if cell.style.faint {
                rgb(cell.fg).gamma_multiply(0.65)
            } else {
                rgb(cell.fg)
            };
            if !text.trim().is_empty() {
                let font_id = if cell.style.bold {
                    style::mono_bold(font_size)
                } else {
                    FontId::monospace(font_size)
                };
                let format = egui::TextFormat {
                    font_id,
                    color,
                    italics: cell.style.italic,
                    ..Default::default()
                };
                let mut job = egui::text::LayoutJob::default();
                job.append(&text, 0.0, format);
                result.glyphs.push(GlyphRun {
                    column: start,
                    width: x - start,
                    galley: ctx.fonts_mut(|f| f.layout_job(job)),
                });
            }
            let underline = cell.underline_color.map_or(color, rgb);
            if cell.style.underline != Underline::None {
                result.decorations.push((start, x, 0.90, underline));
                if cell.style.underline == Underline::Double {
                    result.decorations.push((start, x, 0.78, underline));
                }
            }
            if cell.style.strikethrough {
                result.decorations.push((start, x, 0.51, color));
            }
            if cell.style.overline {
                result.decorations.push((start, x, 0.05, color));
            }
        }
        result
    }
}

/// The inclusive bounds of the word, or the run of blanks, around a flat cell index.
pub fn word_at(frame: &Frame, index: usize) -> (usize, usize) {
    let cols = usize::from(frame.cols).max(1);
    let y = index / cols;
    let Some(row) = frame.lines.get(y) else {
        return (index, index);
    };
    let x = (index % cols).min(row.cells.len().saturating_sub(1));
    let blank = |at: usize| row.cells.get(at).is_none_or(|c| c.text.trim().is_empty());
    let kind = blank(x);
    let (mut start, mut end) = (x, x);
    while start > 0 && blank(start - 1) == kind {
        start -= 1;
    }
    while end + 1 < row.cells.len() && blank(end + 1) == kind {
        end += 1;
    }
    (y * cols + start, y * cols + end)
}
