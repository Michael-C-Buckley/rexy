//! Ghostty belongs to one worker per pane. The UI only borrows immutable frames.
//!
//! A pane starts with a streamed Ghostty snapshot, then carries VT output with
//! nonce-tagged Rex metadata. Neither socket reads nor UI callbacks decode it.
use anyhow::{Context, Result, anyhow, ensure};
use libghostty_vt::{
    RenderState, Terminal,
    color::{Palette, Rgb},
    mode::Mode,
    point::{Coordinate, Point},
    render::{CellIterator, Colors, CursorVisualStyle, Dirty, RowIterator},
    screen::{CellWide, Screen},
    snapshot,
    style::{Style, StyleColor},
    terminal::ScrollViewport,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    io::{self, Read},
    rc::Rc,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicI32, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

const CHUNK_BYTES: usize = 64 * 1024;
const QUEUE_CHUNKS: usize = 16;
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_APC_BYTES: usize = 1024 * 1024;
const SCROLLBACK_BYTES: usize = 32 * 1024 * 1024;
const MAX_LINK_BYTES: usize = 2048;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const RENDER_HOLD_LIMIT: Duration = Duration::from_secs(1);

pub type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    Hollow,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    /// The trailing cell of a wide glyph: do not draw text.
    pub spacer_tail: bool,
    /// An unused last column when a wide glyph wraps to the following row.
    pub spacer_head: bool,
    /// Effective colors: reverse-video has already been applied.
    pub fg: u32,
    pub bg: u32,
    pub underline_color: Option<u32>,
    pub style: Style,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
    /// The links its programs have marked: columns from one up to another, and
    /// where they lead.
    pub links: Vec<(usize, usize, Arc<str>)>,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Arc<Row>>,
    pub fg: u32,
    pub bg: u32,
    pub cursor: Option<(u16, u16)>,
    pub cursor_color: u32,
    pub cursor_shape: CursorShape,
    pub mouse_tracking: bool,
    /// The alternate screen is showing, which keeps no history to scroll through.
    pub alternate_screen: bool,
    /// The program would have the wheel send arrow keys there (DEC mode 1007).
    pub alternate_scroll: bool,
    /// Distance in rows from the active terminal, zero when following output.
    /// Scrolled back, it is as of this frame: history that grows under a view
    /// it leaves as it was draws no new one.
    pub scroll_offset: usize,
}

impl Frame {
    /// Copy inclusive flat cell bounds, retaining graphemes and soft line wraps.
    pub fn selected_text(&self, start: usize, end: usize) -> String {
        let cols = usize::from(self.cols);
        if cols == 0 || self.lines.is_empty() {
            return String::new();
        }
        let limit = cols.saturating_mul(self.lines.len());
        let (mut start, end) = (start.min(end), start.max(end).min(limit - 1));
        if start >= limit {
            return String::new();
        }
        // Clicking the trailing half of a wide glyph selects the glyph too.
        while start % cols > 0
            && self.lines[start / cols]
                .cells
                .get(start % cols)
                .is_some_and(|cell| cell.spacer_tail)
        {
            start -= 1;
        }
        let mut out = String::new();
        for (y, row) in self
            .lines
            .iter()
            .enumerate()
            .take(end / cols + 1)
            .skip(start / cols)
        {
            let left = if y == start / cols { start % cols } else { 0 };
            let right = if y == end / cols {
                end % cols
            } else {
                cols - 1
            };
            let mut line = String::new();
            for cell in row.cells.iter().take(right + 1).skip(left) {
                if !cell.spacer_tail && !cell.spacer_head {
                    if cell.text.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(&cell.text);
                    }
                }
            }
            if row.wrapped && y < end / cols {
                out.push_str(&line);
            } else {
                out.push_str(line.trim_end_matches(' '));
                if y < end / cols {
                    out.push('\n');
                }
            }
        }
        out
    }
}

enum Command {
    Bytes(Vec<u8>),
    Wake,
}

struct Shared {
    frame: RwLock<Option<Arc<Frame>>>,
    error: Mutex<Option<String>>,
    scroll: AtomicI32,
    prompts: AtomicI32,
    bottom: AtomicBool,
    repaint: Notify,
}

/// Dropping the last handle disconnects the bounded queue and wakes the worker,
/// including a worker blocked in snapshot decoding. No worker retains a sender.
#[derive(Clone)]
pub struct TerminalHandle {
    input: flume::Sender<Command>,
    shared: Arc<Shared>,
}

impl TerminalHandle {
    pub fn new(nonce: u64, repaint: Notify) -> Result<Self> {
        let (input, receiver) = flume::bounded(QUEUE_CHUNKS);
        let shared = Arc::new(Shared {
            frame: RwLock::new(None),
            error: Mutex::new(None),
            scroll: AtomicI32::new(0),
            prompts: AtomicI32::new(0),
            bottom: AtomicBool::new(false),
            repaint,
        });
        let worker_shared = shared.clone();
        thread::Builder::new()
            .name("rex-terminal".into())
            .spawn(move || {
                if let Err(error) = run_worker(receiver, &worker_shared, nonce) {
                    *worker_shared.error.lock().unwrap() = Some(format!("{error:#}"));
                    (worker_shared.repaint)();
                }
            })
            .context("starting terminal worker")?;
        Ok(Self { input, shared })
    }

    /// Backpressure is intentional: terminal bytes must never be discarded.
    pub async fn feed(&self, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() <= CHUNK_BYTES {
            return self.send(bytes).await;
        }
        for chunk in bytes.chunks(CHUNK_BYTES) {
            self.send(chunk.to_vec()).await?;
        }
        Ok(())
    }

    async fn send(&self, bytes: Vec<u8>) -> Result<()> {
        let sent = self.input.send_async(Command::Bytes(bytes)).await;
        sent.map_err(|_| {
            anyhow!(
                self.status()
                    .unwrap_or_else(|| "terminal worker stopped".into())
            )
        })
    }

    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared.frame.read().unwrap().clone()
    }

    /// The error that stopped the worker, if any.
    pub fn status(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Negative is up. Wheel deltas are coalesced, so output cannot starve scroll.
    pub fn scroll(&self, lines: i32) -> Result<()> {
        self.nudge(&self.shared.scroll, lines)
    }

    /// Negative is up, a screenful at a time.
    pub fn scroll_pages(&self, pages: i32) -> Result<()> {
        let rows = self.latest().map_or(0, |frame| i32::from(frame.rows));
        self.scroll(pages.saturating_mul(rows))
    }

    /// To the shell prompt before the view when negative, or after it, where the
    /// shell marks its prompts (OSC 133).
    pub fn scroll_prompts(&self, prompts: i32) -> Result<()> {
        self.nudge(&self.shared.prompts, prompts)
    }

    pub fn scroll_to_bottom(&self) {
        self.shared.bottom.store(true, Relaxed);
        self.shared.scroll.store(0, Relaxed);
        self.shared.prompts.store(0, Relaxed);
        self.wake();
    }

    fn nudge(&self, control: &AtomicI32, by: i32) -> Result<()> {
        ensure!(!self.input.is_disconnected(), "terminal worker stopped");
        control.update(Relaxed, Relaxed, |old| old.saturating_add(by));
        self.wake();
        Ok(())
    }

    fn wake(&self) {
        // If the queue is full the worker is already awake. No bytes are lost.
        let _ = self.input.try_send(Command::Wake);
    }
}

/// Consumes at most one 64-KiB chunk at a time; unread VT bytes stay here after
/// the snapshot decoder reaches FINISH. Read runs only on the terminal thread.
struct Input {
    receiver: flume::Receiver<Command>,
    bytes: Vec<u8>,
    offset: usize,
}

impl Input {
    fn next(&mut self, timeout: Option<Duration>) -> Option<Command> {
        if self.offset < self.bytes.len() {
            let bytes = self.bytes.split_off(self.offset);
            self.bytes.clear();
            self.offset = 0;
            return Some(Command::Bytes(bytes));
        }
        match timeout {
            Some(timeout) => match self.receiver.recv_timeout(timeout) {
                Ok(command) => Some(command),
                Err(flume::RecvTimeoutError::Timeout) => Some(Command::Wake),
                Err(flume::RecvTimeoutError::Disconnected) => None,
            },
            None => self.receiver.recv().ok(),
        }
    }
}

impl Read for Input {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        while self.offset == self.bytes.len() {
            match self.receiver.recv() {
                Ok(Command::Bytes(bytes)) => {
                    self.bytes = bytes;
                    self.offset = 0;
                }
                Ok(Command::Wake) => {}
                Err(_) => return Ok(0),
            }
        }
        let len = target.len().min(self.bytes.len() - self.offset);
        target[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
        self.offset += len;
        Ok(len)
    }
}

#[derive(Debug, PartialEq)]
enum Part {
    Output(Vec<u8>),
    Metadata(Value),
}

/// Only APCs bearing this stream's nonce are metadata. All other bytes retain
/// their exact order, including a partial escape prefix across socket frames.
struct ApcParser {
    prefix: Vec<u8>,
    pending: Vec<u8>,
}

impl ApcParser {
    fn new(nonce: u64) -> Self {
        Self {
            prefix: format!("\x1b_rex1;{nonce};").into_bytes(),
            pending: Vec::new(),
        }
    }

    fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<Part>> {
        let mut parts = Vec::new();
        let mut output = Vec::new();
        while !bytes.is_empty() {
            if self.pending.is_empty() {
                let plain = bytes
                    .iter()
                    .position(|&byte| byte == 0x1b)
                    .unwrap_or(bytes.len());
                output.extend_from_slice(&bytes[..plain]);
                bytes = &bytes[plain..];
                if bytes.is_empty() {
                    break;
                }
            }
            self.pending.push(bytes[0]);
            bytes = &bytes[1..];
            loop {
                if self.pending.starts_with(&self.prefix) {
                    ensure!(
                        self.pending.len() <= MAX_APC_BYTES,
                        "Rex metadata exceeds 1 MiB"
                    );
                    if self.pending.ends_with(b"\x1b\\") {
                        if !output.is_empty() {
                            parts.push(Part::Output(std::mem::take(&mut output)));
                        }
                        let json = &self.pending[self.prefix.len()..self.pending.len() - 2];
                        parts.push(Part::Metadata(
                            serde_json::from_slice(json).context("invalid Rex metadata")?,
                        ));
                        self.pending.clear();
                    }
                    break;
                }
                if self.prefix.starts_with(&self.pending) {
                    break;
                }
                output.push(self.pending.remove(0));
            }
        }
        if !output.is_empty() {
            parts.push(Part::Output(output));
        }
        Ok(parts)
    }
}

/// Restore a terminal from a snapshot. Ghostty checks the envelope and every
/// record as it streams in, and reads no further than the snapshot's end. It
/// bounds each record but not their sum, which is cut short here.
fn restore(input: impl Read) -> Result<Terminal<'static, 'static>, libghostty_vt::Error> {
    let mut input = input.take(MAX_SNAPSHOT_BYTES);
    let mut decoder = snapshot::Decoder::new(&mut input)?;
    decoder.set_max_continuation_bytes(MAX_APC_BYTES)?;
    // Images are not shown, so those a snapshot carries are not kept either.
    decoder.set_kitty_image_storage_limit(0)?;
    // History is kept compressed, a page at a time as it arrives. A page opens
    // again when it is scrolled to.
    decoder.set_compress_history_enabled(true)?;
    decoder.decode()
}

fn configure(terminal: &mut Terminal) -> Result<()> {
    // A remote pane must never read local paths through an image escape. Images
    // are intentionally deferred; text and snapshot state remain supported.
    terminal.set_kitty_image_storage_limit(0)?;
    terminal.set_kitty_image_from_file_allowed(false)?;
    terminal.set_kitty_image_from_shared_mem_allowed(false)?;
    terminal.set_kitty_image_temp_file_dir(None)?;
    terminal.set_scrollback_max_bytes(Some(SCROLLBACK_BYTES))?;
    Ok(())
}

/// Scroll as the handle asked. True when the view moved.
fn apply_controls(shared: &Shared, terminal: &mut Terminal) -> Result<bool> {
    let bottom = shared.bottom.swap(false, Relaxed);
    let prompts = shared.prompts.swap(0, Relaxed);
    let lines = shared.scroll.swap(0, Relaxed);
    if !bottom && prompts == 0 && lines == 0 {
        return Ok(false);
    }
    let before = terminal.scrollbar()?.offset;
    if bottom {
        terminal.scroll_viewport(ScrollViewport::Bottom);
    }
    if prompts != 0 {
        terminal.scroll_viewport(ScrollViewport::DeltaPrompt(prompts as isize));
    }
    if lines != 0 {
        terminal.scroll_viewport(ScrollViewport::Delta(lines as isize));
    }
    Ok(terminal.scrollbar()?.offset != before)
}

fn run_worker(receiver: flume::Receiver<Command>, shared: &Arc<Shared>, nonce: u64) -> Result<()> {
    let mut input = Input {
        receiver,
        bytes: Vec::new(),
        offset: 0,
    };
    // The snapshot streams through bounded chunks on this worker, but the first
    // frame waits for the whole of it to validate.
    let mut terminal = match restore(&mut input) {
        Ok(terminal) => terminal,
        Err(_) if input.receiver.is_disconnected() => return Ok(()),
        Err(error) => return Err(error).context("restoring terminal snapshot"),
    };
    configure(&mut terminal)?;
    // The terminal calls back in the middle of a write, so the renderer is shared
    // with that callback and never borrowed across a write or a resize.
    let renderer = Rc::new(RefCell::new(Renderer::new()?));
    // A snapshot can be taken inside a hold, of whose beginning nothing will tell.
    renderer.borrow_mut().held = terminal.mode(Mode::SYNC_OUTPUT)?.then(Instant::now);
    terminal.on_render_hold({
        let (renderer, shared) = (renderer.clone(), shared.clone());
        move |terminal, held| {
            let Ok(mut renderer) = renderer.try_borrow_mut() else {
                return;
            };
            renderer.held = held.then(Instant::now);
            // As a hold begins the terminal is exactly the frame its program wants
            // left on screen; what follows in the same write is the next one, half
            // drawn. A program can begin holds without number in one write, so
            // no more of them are drawn than of any other frame, and one that
            // comes too soon after another waits for its hold to end. A failure
            // here shows again when the worker next draws.
            if held
                && renderer.caught.elapsed() >= FRAME_INTERVAL
                && renderer.publish(terminal, &shared).unwrap_or(false)
            {
                renderer.caught = renderer.drawn;
            }
        }
    })?;
    let mut parser = ApcParser::new(nonce);
    // A frame is owed whatever a program holds back: the first after a restore,
    // and where the user has scrolled to.
    let mut owed = true;
    loop {
        owed |= apply_controls(shared, &mut terminal)?;
        let now = Instant::now();
        let wait = {
            let mut renderer = renderer.borrow_mut();
            // The terminal has no clock, so a hold that never ends is ended here.
            if renderer
                .held
                .is_some_and(|since| now.duration_since(since) >= RENDER_HOLD_LIMIT)
            {
                terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
                renderer.held = None;
            }
            let due = renderer.drawn + FRAME_INTERVAL;
            // Sleep out a hold, or until the next frame is due; once that is
            // published, until more input.
            match renderer.held {
                Some(since) if !owed => Some(since + RENDER_HOLD_LIMIT - now),
                _ if now < due => Some(due - now),
                _ => {
                    renderer.publish(&terminal, shared)?;
                    owed = false;
                    None
                }
            }
        };
        let Some(command) = input.next(wait) else {
            return Ok(());
        };
        let Command::Bytes(bytes) = command else {
            continue;
        };
        for part in parser.feed(&bytes)? {
            match part {
                Part::Output(bytes) => {
                    // The server owns side effects and answers device queries: what
                    // the terminal would say or do of itself here goes unheard.
                    terminal.vt_write(&bytes);
                }
                Part::Metadata(value) => apply_metadata(&mut terminal, &value)?,
            }
        }
    }
}

fn apply_metadata(terminal: &mut Terminal, value: &Value) -> Result<()> {
    match value["type"].as_str() {
        Some("resize") => {
            let dimension = |key: &str| -> Result<u16> {
                value[key]
                    .as_u64()
                    .filter(|v| (1..=65535).contains(v))
                    .map(|v| v as u16)
                    .with_context(|| format!("invalid terminal {key}"))
            };
            let pixels = |key: &str| value[key].as_u64().unwrap_or(0).min(u32::MAX.into()) as u32;
            let cols = dimension("cols")?;
            let rows = dimension("rows")?;
            ensure!(
                u32::from(cols) * u32::from(rows) <= 4_000_000,
                "terminal grid is too large"
            );
            terminal.resize(
                cols,
                rows,
                pixels("cell_width_px"),
                pixels("cell_height_px"),
            )?;
        }
        Some("theme") => {
            // One left out, empty or unreadable is the built-in one.
            let color = |key: &str| value[key].as_str().and_then(|v| Rgb::parse(v).ok());
            terminal.set_default_fg_color(color("foreground"))?;
            terminal.set_default_bg_color(color("background"))?;
            terminal.set_default_cursor_color(color("cursor"))?;
            terminal.set_default_color_palette(None)?;
            if let Some(entries) = value["palette"].as_array() {
                let mut palette = terminal.default_color_palette()?;
                for entry in entries.iter().filter_map(Value::as_str) {
                    if let Ok((index, color)) = Palette::parse_palette_entry(entry) {
                        palette.set(index, color);
                    }
                }
                terminal.set_default_color_palette(Some(palette))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn rgb(color: Rgb) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

struct Renderer {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    previous: Option<Arc<Frame>>,
    /// The colours of that frame. Ghostty marks nothing dirty when only they change.
    colors: Option<Colors>,
    /// When the last new frame was drawn, when the last was drawn as a hold began,
    /// and since when its program has held frames back (synchronized output, DEC
    /// mode 2026).
    drawn: Instant,
    caught: Instant,
    held: Option<Instant>,
}

impl Renderer {
    fn new() -> Result<Self> {
        Ok(Self {
            state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            previous: None,
            colors: None,
            drawn: Instant::now() - FRAME_INTERVAL,
            caught: Instant::now() - FRAME_INTERVAL,
            held: None,
        })
    }

    /// Draw the terminal as it stands, and hand the UI the frame if it is a new one.
    fn publish(&mut self, terminal: &Terminal<'static, '_>, shared: &Shared) -> Result<bool> {
        let Some(frame) = self.frame(terminal)? else {
            return Ok(false);
        };
        *shared.frame.write().unwrap() = Some(frame);
        (shared.repaint)();
        self.drawn = Instant::now();
        Ok(true)
    }

    fn frame(&mut self, terminal: &Terminal<'static, '_>) -> Result<Option<Arc<Frame>>> {
        let snapshot = self.state.update(terminal)?;
        let scrollbar = terminal.scrollbar()?;
        let scroll_offset = scrollbar
            .total
            .saturating_sub(scrollbar.offset + scrollbar.len) as usize;
        let mouse_tracking = terminal.is_mouse_tracking()?;
        let alternate_screen = terminal.active_screen()? == Screen::Alternate;
        let alternate_scroll = terminal.mode(Mode::ALT_SCROLL)?;
        let dirty = snapshot.dirty()?;
        let cursor = snapshot.cursor()?;
        let cursor_shape = match cursor.visual_style {
            CursorVisualStyle::Bar => CursorShape::Bar,
            CursorVisualStyle::Underline => CursorShape::Underline,
            CursorVisualStyle::BlockHollow => CursorShape::Hollow,
            _ => CursorShape::Block,
        };
        let cursor = cursor
            .viewport
            .filter(|_| cursor.visible)
            .map(|p| (p.x, p.y));
        let colors = snapshot.colors()?;
        let recolored = self.colors.as_ref() != Some(&colors);
        // Ghostty marks nothing dirty when only the cursor, the colours or a mode
        // changes. It marks all of it dirty when the view moves, by the user's hand
        // or because its history was cut away, and the rows a clear shifts. History
        // that only grows under a view scrolled back changes how far back that is
        // and nothing in it, so that alone is no new frame.
        if dirty == Dirty::Clean
            && !recolored
            && self.previous.as_ref().is_some_and(|old| {
                (old.cursor, old.cursor_shape) == (cursor, cursor_shape)
                    && (old.scroll_offset == 0) == (scroll_offset == 0)
                    && old.mouse_tracking == mouse_tracking
                    && old.alternate_screen == alternate_screen
                    && old.alternate_scroll == alternate_scroll
            })
        {
            return Ok(None);
        }
        let fg = rgb(colors.foreground);
        let bg = rgb(colors.background);
        let cols = snapshot.cols()?;
        let rows = snapshot.rows()?;
        // Every cell holds the colours it is drawn in, so new ones are new cells.
        // A cursor that only moved leaves every row as it was.
        let full = dirty == Dirty::Full
            || recolored
            || self
                .previous
                .as_ref()
                .is_none_or(|old| old.cols != cols || old.rows != rows);
        let mut lines = Vec::with_capacity(usize::from(rows));
        let mut iterator = self.rows.update(&snapshot)?;
        while let Some(row) = iterator.next() {
            if !full
                && !row.dirty()?
                && let Some(previous) = self
                    .previous
                    .as_ref()
                    .and_then(|old| old.lines.get(lines.len()))
            {
                lines.push(previous.clone());
                continue;
            }
            let wrapped = row.raw_row()?.is_wrapped()?;
            let y = row.viewport_y()?;
            let mut cells = Vec::with_capacity(usize::from(cols));
            let mut links: Vec<(usize, usize, Arc<str>)> = Vec::new();
            let mut iterator = self.cells.update(row)?;
            while let Some(cell) = iterator.next() {
                let mut text = String::new();
                cell.graphemes_utf8(&mut text)?;
                let style = cell.style()?;
                let (mut fg, mut bg) = (
                    cell.fg_color()?.map(rgb).unwrap_or(fg),
                    cell.bg_color()?.map(rgb).unwrap_or(bg),
                );
                if style.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if style.invisible {
                    text.clear();
                }
                let underline_color = match style.underline_color {
                    StyleColor::None => None,
                    StyleColor::Palette(index) => Some(rgb(colors.palette[usize::from(index.0)])),
                    StyleColor::Rgb(color) => Some(rgb(color)),
                };
                let raw = cell.raw_cell()?;
                let x = cells.len();
                if raw.has_hyperlink()?
                    && let (Ok(x16), Ok(y)) = (u16::try_from(x), u32::try_from(y))
                {
                    let at = Point::Viewport(Coordinate { x: x16, y });
                    let mut target = [0; MAX_LINK_BYTES];
                    // One too long to hold is no link.
                    let length = terminal.grid_ref(at)?.hyperlink_uri(&mut target);
                    let length = length.ok().filter(|length| *length > 0);
                    let target = length.map(|length| &target[..length]);
                    if let Some(Ok(target)) = target.map(str::from_utf8) {
                        match links.last_mut() {
                            Some((_, end, last)) if *end == x && **last == *target => *end += 1,
                            _ => links.push((x, x + 1, target.into())),
                        }
                    }
                }
                let wide = raw.wide()?;
                cells.push(Cell {
                    text,
                    spacer_tail: wide == CellWide::SpacerTail,
                    spacer_head: wide == CellWide::SpacerHead,
                    fg,
                    bg,
                    underline_color,
                    style,
                });
            }
            lines.push(Arc::new(Row {
                cells,
                wrapped,
                links,
            }));
        }
        let frame = Arc::new(Frame {
            cols,
            rows,
            lines,
            fg,
            bg,
            scroll_offset,
            mouse_tracking,
            alternate_screen,
            alternate_scroll,
            cursor,
            cursor_color: colors.cursor.map(rgb).unwrap_or(fg),
            cursor_shape,
        });
        snapshot.clean()?;
        self.previous = Some(frame.clone());
        self.colors = Some(colors);
        Ok(Some(frame))
    }
}

#[cfg(test)]
#[path = "../tests/unit/terminal.rs"]
pub(crate) mod tests;
