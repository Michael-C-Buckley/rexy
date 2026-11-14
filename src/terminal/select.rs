//! A pane's selection, which Ghostty keeps: it stays with its text as output
//! arrives and history scrolls. What the pointer means by a press, a drag and
//! a release is Ghostty's to say too.
use anyhow::Result;
use libghostty_vt::{
    Terminal,
    fmt::Formatter,
    point::{Coordinate, Point},
    selection::gesture::{
        Autoscroll, AutoscrollTickEvent, DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent,
    },
};
use std::time::{Duration, Instant};

use super::FRAME_INTERVAL;

/// Ghostty is told where the pointer is in pixels. A cell is this many across
/// and down, whatever it is on screen.
const CELL: u32 = 100;
/// Presses no further apart than this are one click after another.
const REPEAT: Duration = Duration::from_millis(500);
/// Blanks alone part words, so that a path or an address is one word.
const BLANKS: [char; 2] = [' ', '\t'];

pub struct Selecting {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    tick: AutoscrollTickEvent<'static>,
    began: Instant,
    /// The cell a drag was last at, and whether it is held past the top or the
    /// bottom, where the view follows it.
    cell: Coordinate,
    pub scrolling: bool,
    ticked: Instant,
}

impl Selecting {
    pub fn new() -> Result<Self> {
        let mut press = PressEvent::new()?;
        press
            .set_word_boundary_codepoints(&BLANKS)?
            .set_repeat_distance(CELL.into())?
            .set_repeat_interval(REPEAT)?;
        let (mut drag, mut tick) = (DragEvent::new()?, AutoscrollTickEvent::new()?);
        drag.set_word_boundary_codepoints(&BLANKS)?;
        tick.set_word_boundary_codepoints(&BLANKS)?;
        Ok(Self {
            gesture: Gesture::new()?,
            press,
            drag,
            tick,
            began: Instant::now(),
            cell: Coordinate { x: 0, y: 0 },
            scrolling: false,
            ticked: Instant::now(),
        })
    }

    /// The button goes down: once to let go of what is selected, twice for a
    /// word and three times for a line. To `extend` is to drag on from the last
    /// press instead, if that left anything selected.
    pub fn press(&mut self, terminal: &Terminal, at: [f32; 2], extend: bool) -> Result<()> {
        if extend && terminal.selection()?.is_some() && self.gesture.click_count(terminal)? > 0 {
            return self.drag(terminal, at, false);
        }
        let (cell, [x, y]) = place(&grid(terminal)?, at);
        self.press
            .set_position(x, y)?
            .set_time(self.began.elapsed())?;
        let pressed = terminal.grid_ref(Point::Viewport(cell))?;
        let selected = self.press.apply(&mut self.gesture, terminal, pressed)?;
        terminal.set_selection(selected.as_ref())?;
        Ok(())
    }

    /// The pointer moves with the button down. A `block` is selected by columns.
    pub fn drag(&mut self, terminal: &Terminal, at: [f32; 2], block: bool) -> Result<()> {
        let geometry = grid(terminal)?;
        let (cell, [x, y]) = place(&geometry, at);
        self.drag.set_position(x, y)?.set_rectangle(block)?;
        self.tick.set_position(x, y)?.set_rectangle(block)?;
        self.cell = cell;
        let dragged = terminal.grid_ref(Point::Viewport(cell))?;
        let selected = (self.drag).apply(&mut self.gesture, terminal, dragged, geometry)?;
        terminal.set_selection(selected.as_ref())?;
        self.scrolling = self.gesture.autoscroll(terminal)? != Autoscroll::None;
        Ok(())
    }

    pub fn release(&mut self, terminal: &Terminal) -> Result<()> {
        ReleaseEvent::new()?.apply(&mut self.gesture, terminal, None)?;
        self.scrolling = false;
        Ok(())
    }

    /// Move the view a row toward a drag held past its edge, and the selection
    /// with it, if a frame has passed since the last. True when it moved.
    pub fn tick(&mut self, terminal: &Terminal) -> Result<bool> {
        if !self.scrolling || self.ticked.elapsed() < FRAME_INTERVAL {
            return Ok(false);
        }
        self.ticked = Instant::now();
        let geometry = grid(terminal)?;
        let selected = (self.tick).apply(&mut self.gesture, terminal, self.cell, geometry)?;
        // A drag that began on another screen selects nothing on this one.
        if selected.is_some() {
            terminal.set_selection(selected.as_ref())?;
        }
        self.scrolling = self.gesture.autoscroll(terminal)? != Autoscroll::None;
        Ok(true)
    }
}

/// The grid as Ghostty measures it.
fn grid(terminal: &Terminal) -> Result<Geometry> {
    Ok(Geometry {
        columns: terminal.cols()?.into(),
        cell_width: CELL,
        padding_left: 0,
        screen_height: u32::from(terminal.rows()?) * CELL,
    })
}

/// A place given in cells from the grid's top left corner, which may lie outside
/// the grid: the cell nearest to it, and the place by Ghostty's measure.
fn place(grid: &Geometry, at: [f32; 2]) -> (Coordinate, [f64; 2]) {
    let within = |at: f32, cells: u32| (at.max(0.0) as u32).min(cells.saturating_sub(1));
    let cell = Coordinate {
        x: within(at[0], grid.columns) as u16,
        y: within(at[1], grid.screen_height / CELL),
    };
    (cell, at.map(|at| f64::from(at) * f64::from(CELL)))
}

/// What is selected, as text. Rows that wrap run on, and the blanks that end a
/// line are left out.
pub fn text(terminal: &Terminal) -> Result<Option<String>> {
    let Some(mut formatter) = Formatter::new_selection(terminal)? else {
        return Ok(None);
    };
    formatter.set_unwrap(true)?.set_trim(true)?;
    let text = formatter.format_alloc(None)?;
    Ok(Some(String::from_utf8_lossy(&text).into_owned()))
}

#[cfg(test)]
#[path = "../../tests/unit/terminal_select.rs"]
mod tests;
