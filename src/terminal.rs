//! Ghostty belongs to one worker per pane. The UI only borrows immutable frames.
//!
//! A pane starts with a streamed Ghostty snapshot, then carries VT output with
//! nonce-tagged Rex metadata. Neither socket reads nor UI callbacks decode it.
use anyhow::{Context, Result, anyhow, ensure};
use ghostty_vt::{
    RenderState, Terminal,
    render::{CellIterator, CursorVisualStyle, Dirty, RowIterator},
    screen::CellWide,
    snapshot,
    style::{RgbColor, Style, StyleColor},
    terminal::{Mode, Point, PointCoordinate, ScrollViewport},
};
use serde_json::Value;
use std::{
    io::{self, Read},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicI32, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

const CHUNK_BYTES: usize = 64 * 1024;
const QUEUE_CHUNKS: usize = 16;
const MAX_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
const MAX_APC_BYTES: usize = 1024 * 1024;
const SCROLLBACK_BYTES: usize = 32 * 1024 * 1024;
const MAX_LINK_BYTES: usize = 2048;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const RENDER_HOLD_LIMIT: Duration = Duration::from_secs(1);
/// The snapshot envelope, and the header of each record after it, are ten bytes.
const SNAPSHOT_ENVELOPE: &[u8; 10] = b"GHOSTSNP\x01\x00";
const FINISH_RECORD: u16 = 6;

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
    /// Distance in rows from the active terminal, zero when following output.
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
        ensure!(!self.input.is_disconnected(), "terminal worker stopped");
        self.shared
            .scroll
            .update(Relaxed, Relaxed, |old| old.saturating_add(lines));
        self.wake();
        Ok(())
    }

    pub fn scroll_to_bottom(&self) {
        self.shared.bottom.store(true, Relaxed);
        self.shared.scroll.store(0, Relaxed);
        self.wake();
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

/// Bounds the wire envelope before Ghostty sees allocation-driving record
/// lengths. Stops exactly at FINISH even if snapshot and live VT share a frame.
struct SnapshotSource<R> {
    inner: R,
    header: [u8; 10],
    header_used: usize,
    envelope: bool,
    /// Bytes left in the current record.
    payload: usize,
    /// The current record is FINISH; the snapshot ends with it.
    finish: bool,
    total: usize,
}

impl<R: Read> SnapshotSource<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            header: [0; 10],
            header_used: 0,
            envelope: true,
            payload: 0,
            finish: false,
            total: 0,
        }
    }
}

impl<R: Read> Read for SnapshotSource<R> {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if target.is_empty() || (self.finish && self.payload == 0) {
            return Ok(0);
        }
        if self.payload > 0 {
            let len = target.len().min(self.payload);
            let read = self.inner.read(&mut target[..len])?;
            self.payload -= read;
            self.total += read;
            return Ok(read);
        }
        let len = target.len().min(10 - self.header_used);
        let read = self.inner.read(&mut target[..len])?;
        self.header[self.header_used..self.header_used + read].copy_from_slice(&target[..read]);
        self.header_used += read;
        self.total += read;
        if self.header_used == 10 {
            self.header_used = 0;
            if self.envelope {
                if &self.header != SNAPSHOT_ENVELOPE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unsupported Ghostty snapshot",
                    ));
                }
                self.envelope = false;
            } else {
                let length = u32::from_le_bytes(self.header[2..6].try_into().unwrap()) as usize;
                if length > MAX_RECORD_BYTES
                    || self.total.saturating_add(length) > MAX_SNAPSHOT_BYTES
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Ghostty snapshot exceeds memory limits",
                    ));
                }
                self.payload = length;
                let record = u16::from_le_bytes(self.header[..2].try_into().unwrap());
                self.finish = record == FINISH_RECORD;
            }
        }
        Ok(read)
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

fn configure(terminal: &mut Terminal) -> Result<()> {
    // A remote pane must never read local paths through an image escape. Images
    // are intentionally deferred; text and snapshot state remain supported.
    terminal.set_kitty_image_storage_limit(0)?;
    terminal.set_kitty_image_from_file_allowed(false)?;
    terminal.set_kitty_image_from_shared_mem_allowed(false)?;
    // Do not call 0.8's temp-file getter/setter: its bool signature does not
    // match GhosttyString in the pinned C API. Zero storage disables all media.
    terminal.set_scrollback_max_bytes(Some(SCROLLBACK_BYTES))?;
    Ok(())
}

fn apply_controls(shared: &Shared, terminal: &mut Terminal) {
    if shared.bottom.swap(false, Relaxed) {
        terminal.scroll_viewport(ScrollViewport::Bottom);
    }
    let scroll = shared.scroll.swap(0, Relaxed);
    if scroll != 0 {
        terminal.scroll_viewport(ScrollViewport::Delta(scroll as isize));
    }
}

fn run_worker(receiver: flume::Receiver<Command>, shared: &Shared, nonce: u64) -> Result<()> {
    let mut input = Input {
        receiver,
        bytes: Vec::new(),
        offset: 0,
    };
    let mut renderer = Renderer::new()?;
    let mut last_frame = Instant::now() - FRAME_INTERVAL;
    // The 0.8 Rust wrapper's next_page assumes every record is a history page;
    // image records are legal too. Its complete decoder handles both correctly.
    // The Read source still streams through bounded chunks on this worker, but
    // the first frame waits for the complete snapshot to validate.
    let restored = {
        let source = SnapshotSource::new(&mut input);
        let mut decoder = snapshot::Decoder::new(source)?;
        decoder.set_max_continuation_bytes(MAX_APC_BYTES)?;
        decoder.decode()
    };
    let mut terminal = match restored {
        Ok(terminal) => terminal,
        Err(_) if input.receiver.is_disconnected() => return Ok(()),
        Err(error) => return Err(error).context("restoring terminal snapshot"),
    };
    configure(&mut terminal)?;
    let mut parser = ApcParser::new(nonce);
    let mut held_since = None;
    loop {
        apply_controls(shared, &mut terminal);
        let now = Instant::now();
        let mut deadline = last_frame + FRAME_INTERVAL;
        if terminal.mode(Mode::SYNC_OUTPUT)? {
            let since = *held_since.get_or_insert(now);
            if now.duration_since(since) < RENDER_HOLD_LIMIT {
                deadline = deadline.max(since + RENDER_HOLD_LIMIT);
            } else {
                terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
                held_since = None;
            }
        } else {
            held_since = None;
        }
        // Sleep until the next frame is due; once it is published, until more input.
        let wait = if now < deadline {
            Some(deadline - now)
        } else {
            if let Some(frame) = renderer.frame(&terminal)? {
                *shared.frame.write().unwrap() = Some(frame);
                (shared.repaint)();
            }
            last_frame = Instant::now();
            None
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
                    terminal.vt_write(&bytes);
                    // The server owns side effects and answers device queries.
                    // Draining prevents unused titles/bells/PTY replies accumulating.
                    terminal.events().for_each(drop);
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
            terminal.set_default_fg_color(parse_color(value["foreground"].as_str()))?;
            terminal.set_default_bg_color(parse_color(value["background"].as_str()))?;
            terminal.set_default_cursor_color(parse_color(value["cursor"].as_str()))?;
            terminal.set_default_color_palette(None)?;
            if let Some(entries) = value["palette"].as_array() {
                let mut palette = terminal.color_palette()?;
                for entry in entries.iter().filter_map(Value::as_str) {
                    if let Some((index, color)) = entry.split_once('=')
                        && let (Ok(index), Some(color)) =
                            (index.parse::<u8>(), parse_color(Some(color)))
                    {
                        palette.0[usize::from(index)] = color;
                    }
                }
                terminal.set_default_color_palette(Some(palette))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_color(value: Option<&str>) -> Option<RgbColor> {
    let value = value?.trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    let color = u32::from_str_radix(value, 16).ok()?;
    Some(RgbColor {
        r: (color >> 16) as u8,
        g: (color >> 8) as u8,
        b: color as u8,
    })
}

fn rgb(color: RgbColor) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

struct Renderer {
    state: RenderState,
    rows: RowIterator,
    cells: CellIterator,
    previous: Option<Arc<Frame>>,
}

impl Renderer {
    fn new() -> Result<Self> {
        Ok(Self {
            state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            previous: None,
        })
    }

    fn frame(&mut self, terminal: &Terminal) -> Result<Option<Arc<Frame>>> {
        let snapshot = self.state.update(terminal)?;
        let scrollbar = terminal.scrollbar()?;
        let scroll_offset = scrollbar
            .total
            .saturating_sub(scrollbar.offset + scrollbar.len) as usize;
        let mouse_tracking = terminal.is_mouse_tracking()?;
        let dirty = snapshot.dirty()?;
        if dirty == Dirty::Clean
            && self.previous.as_ref().is_some_and(|old| {
                old.scroll_offset == scroll_offset && old.mouse_tracking == mouse_tracking
            })
        {
            return Ok(None);
        }
        let colors = snapshot.colors()?;
        let fg = rgb(colors.foreground);
        let bg = rgb(colors.background);
        let cols = snapshot.cols()?;
        let rows = snapshot.rows()?;
        let full = dirty == Dirty::Full
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
                    let at = Point::Viewport(PointCoordinate { x: x16, y });
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
            cursor: if snapshot.cursor_visible()? {
                snapshot.cursor_viewport()?.map(|p| (p.x, p.y))
            } else {
                None
            },
            cursor_color: snapshot.cursor_color()?.map(rgb).unwrap_or(fg),
            cursor_shape: match snapshot.cursor_visual_style()? {
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::Hollow,
                _ => CursorShape::Block,
            },
        });
        snapshot.clean()?;
        self.previous = Some(frame.clone());
        Ok(Some(frame))
    }
}

#[cfg(test)]
#[path = "../tests/unit/terminal.rs"]
mod tests;
