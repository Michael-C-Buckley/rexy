use super::*;

fn terminal(cols: u16, rows: u16) -> Terminal<'static, 'static> {
    Terminal::new(cols, rows).unwrap()
}

/// What a terminal shows as it stands.
pub(crate) fn render(terminal: &Terminal<'static, '_>) -> Arc<Frame> {
    Renderer::new().unwrap().frame(terminal).unwrap().unwrap()
}

fn encode(terminal: &mut Terminal) -> Vec<u8> {
    let mut bytes = Vec::new();
    terminal.encode_snapshot(&mut bytes).unwrap();
    bytes
}

const WAIT: Duration = Duration::from_secs(3);
const QUIET: Duration = Duration::from_millis(80);

/// A worker fed these bytes, and the moment each of its frames was published.
fn worker(bytes: Vec<u8>) -> (TerminalHandle, flume::Receiver<Instant>) {
    let (wake, wakes) = flume::unbounded();
    let notify = move || {
        let _ = wake.send(Instant::now());
    };
    let handle = TerminalHandle::new(7, Arc::new(notify)).unwrap();
    write(&handle, &bytes);
    (handle, wakes)
}

fn write(handle: &TerminalHandle, bytes: &[u8]) {
    handle.input.send(Command::Bytes(bytes.to_vec())).unwrap();
}

/// The text of every row.
fn text(frame: &Frame) -> String {
    frame.selected_text(0, usize::MAX)
}

/// The text of the first row a worker last drew.
fn top(handle: &TerminalHandle) -> String {
    let frame = handle.latest().unwrap();
    frame.selected_text(0, usize::from(frame.cols) - 1)
}

struct Fragmented<R>(R);
impl<R: Read> Read for Fragmented<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let len = bytes.len().min(3);
        self.0.read(&mut bytes[..len])
    }
}

#[test]
fn fragmented_snapshot_preserves_state_and_leaves_live_tail_unread() {
    let mut original = terminal(24, 3);
    original.set_continuation_max_bytes(MAX_APC_BYTES).unwrap();
    original.vt_write("first\r\n界 e\u{301}\r\nlast\x1b[38;2;".as_bytes());
    let mut bytes = encode(&mut original);
    let snapshot_len = bytes.len();
    bytes.extend_from_slice(b"1;2;3mX");
    let mut input = io::Cursor::new(bytes);
    let mut restored = restore(Fragmented(&mut input)).unwrap();
    assert_eq!(input.position() as usize, snapshot_len);
    let mut live = Vec::new();
    input.read_to_end(&mut live).unwrap();
    restored.vt_write(&live);
    let frame = render(&restored);
    assert_eq!(frame.selected_text(24, 31), "界 e\u{301}");
    let last = &frame.lines[2];
    assert_eq!(last.text(4, 5), "X");
    assert_eq!(last.look(&last.cells[4]).fg, 0x010203);
}

#[test]
fn snapshot_rejects_crc_damage_and_one_cut_short() {
    let snapshot = encode(&mut terminal(20, 2));
    let mut damaged = snapshot.clone();
    damaged[16] ^= 1;
    assert!(restore(damaged.as_slice()).is_err());
    // As one past its cap is.
    assert!(restore(&snapshot[..snapshot.len() - 1]).is_err());
    assert!(restore(snapshot.as_slice()).is_ok());
}

#[test]
fn restored_history_is_compressed_and_opens_when_scrolled_to() {
    let mut original = terminal(20, 3);
    configure(&mut original).unwrap();
    // More of Ghostty's pages than the one the screen is on.
    for line in 0..10_000 {
        original.vt_write(format!("line {line}\r\n").as_bytes());
    }
    let mut restored = restore(encode(&mut original).as_slice()).unwrap();
    let usage = restored.memory_usage().unwrap();
    assert!(usage.compression_supported && usage.primary_compressed_pages > 0);
    restored.scroll_viewport(ScrollViewport::Top);
    assert_eq!(render(&restored).selected_text(0, 19), "line 0");
}

#[test]
fn image_records_in_snapshot_do_not_break_text_restore() {
    let mut original = terminal(20, 3);
    original.set_kitty_image_storage_limit(1024 * 1024).unwrap();
    // Raw RGB needs no PNG decoder or extra dependency.
    original.vt_write(b"\x1b_Ga=T,f=24,s=1,v=1,i=42;CQkJ\x1b\\\r\n");
    original.vt_write(b"text after image");
    let image = |terminal: &Terminal| {
        let images = terminal.kitty_graphics().unwrap().unwrap();
        images.image(42).is_some()
    };
    assert!(image(&original));
    let bytes = encode(&mut original);
    // The image is left behind, and none can be sent after.
    let mut restored = restore(bytes.as_slice()).unwrap();
    assert!(!image(&restored));
    configure(&mut restored).unwrap();
    assert_eq!(restored.kitty_image_storage_limit().unwrap(), Some(0));
    assert_eq!(
        restored.is_kitty_image_from_file_allowed().unwrap(),
        Some(false)
    );
    let frame = render(&restored);
    assert!(frame.selected_text(0, 59).contains("text after image"));
}

#[test]
fn dropping_last_handle_stops_worker_waiting_for_snapshot_bytes() {
    struct Finished(std::sync::mpsc::Sender<()>);
    impl Drop for Finished {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    for prefix in [Vec::new(), b"GHOSTSNP\x01\x00\x01\x00".to_vec()] {
        let (finished, done) = std::sync::mpsc::channel();
        let signal = Finished(finished);
        let handle = TerminalHandle::new(
            3,
            Arc::new(move || {
                let _ = &signal;
            }),
        )
        .unwrap();
        if !prefix.is_empty() {
            handle.input.send(Command::Bytes(prefix)).unwrap();
        }
        drop(handle);
        done.recv_timeout(Duration::from_secs(3))
            .expect("worker retained its channel or callback");
    }
}

#[test]
fn metadata_parser_preserves_byte_order_across_every_boundary() {
    let input = b"before\x1b[31m\x1b_rex1;42;{\"type\":\"resize\",\"cols\":90,\"rows\":24}\x1b\\after\x1b_rex1;9;{}\x1b\\end";
    for split in 0..=input.len() {
        let mut parser = ApcParser::new(42);
        let mut parts = parser.feed(&input[..split]).unwrap();
        parts.extend(parser.feed(&input[split..]).unwrap());
        let mut plain = Vec::new();
        let mut metadata = Vec::new();
        for part in parts {
            match part {
                Part::Output(bytes) => plain.extend(bytes),
                Part::Metadata(value) => metadata.push((plain.len(), value)),
            }
        }
        assert_eq!(plain, b"before\x1b[31mafter\x1b_rex1;9;{}\x1b\\end");
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].0, 11);
        assert_eq!(metadata[0].1["cols"], 90);
    }
}

#[test]
fn metadata_is_refused_when_malformed_or_past_its_cap() {
    let malformed = ApcParser::new(42).feed(b"\x1b_rex1;42;{\"type\":\x1b\\");
    assert!(format!("{:#}", malformed.unwrap_err()).contains("invalid Rex metadata"));
    // A JSON string that makes the whole sequence this long.
    let sized = |len: usize| {
        let mut bytes = b"\x1b_rex1;42;\"".to_vec();
        bytes.resize(len - 3, b'x');
        bytes.extend_from_slice(b"\"\x1b\\");
        ApcParser::new(42).feed(&bytes)
    };
    let whole = sized(MAX_APC_BYTES).unwrap();
    assert!(matches!(whole[..], [Part::Metadata(Value::String(_))]));
    assert!(sized(MAX_APC_BYTES + 1).is_err());
    // Another stream's is no metadata, and is passed on however long it is.
    let mut foreign = b"\x1b_rex1;9;".to_vec();
    foreign.resize(MAX_APC_BYTES + 1, b'x');
    assert_eq!(
        ApcParser::new(42).feed(&foreign).unwrap(),
        [Part::Output(foreign)]
    );
}

#[test]
fn frames_reuse_unchanged_rows_and_copy_whole_graphemes() {
    let mut terminal = terminal(20, 3);
    terminal.vt_write("界 e\u{301}\r\nsecond\r\nthird".as_bytes());
    let mut renderer = Renderer::new().unwrap();
    let first = renderer.frame(&terminal).unwrap().unwrap();
    assert_eq!(first.selected_text(1, 5), "界 e\u{301}");
    assert_eq!(first.selected_text(0, 25), "界 e\u{301}\nsecond");
    assert!(renderer.frame(&terminal).unwrap().is_none());
    terminal.vt_write(b"\x1b[3;1HX");
    let second = renderer.frame(&terminal).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first.lines[0], &second.lines[0]));
    assert!(!Arc::ptr_eq(&first.lines[2], &second.lines[2]));
}

#[test]
fn a_row_that_only_moved_is_kept_and_one_that_also_changed_is_not() {
    let mut terminal = terminal(20, 4);
    terminal.vt_write(b"one\r\n\x1b[31mtwo\x1b[m\r\nthree\r\nfour");
    let mut renderer = Renderer::new().unwrap();
    let first = renderer.frame(&terminal).unwrap().unwrap();
    // Output scrolls the screen by a row, and in the same write changes another.
    terminal.vt_write(b"\r\nfive\x1b[2;1Hthr33");
    let second = renderer.frame(&terminal).unwrap().unwrap();
    assert_eq!(text(&second), "two\nthr33\nfour\nfive");
    assert!(Arc::ptr_eq(&first.lines[1], &second.lines[0]));
    assert!(!Arc::ptr_eq(&first.lines[2], &second.lines[1]));
    assert!(Arc::ptr_eq(&first.lines[3], &second.lines[2]));
    // The user scrolls back a row.
    terminal.scroll_viewport(ScrollViewport::Delta(-1));
    let back = renderer.frame(&terminal).unwrap().unwrap();
    assert_eq!(text(&back), "one\ntwo\nthr33\nfour");
    for y in 1..4 {
        assert!(Arc::ptr_eq(&back.lines[y], &second.lines[y - 1]));
    }
    // The same text in the same cells is another row in another colour.
    terminal.scroll_viewport(ScrollViewport::Bottom);
    terminal.vt_write(b"\x1b[2J\x1b[H\x1b[34mtwo");
    let blue = renderer.frame(&terminal).unwrap().unwrap();
    assert_eq!(text(&blue), "two\n\n\n");
    assert!(!Arc::ptr_eq(&blue.lines[0], &second.lines[0]));
    assert_eq!(blue.lines[0].look(&blue.lines[0].cells[0]).fg, 0x81a2be);
}

#[test]
fn a_cell_holds_the_whole_of_a_cluster_and_a_wide_glyph_has_its_spare_half() {
    let mut terminal = terminal(14, 2);
    // Characters that belong together share a cell where a program asks for it.
    terminal.vt_write("\x1b[?2027h🇯🇵👨\u{200d}👩\u{200d}👧e\u{301}e\u{302}aa界界y界".as_bytes());
    let frame = render(&terminal);
    let row = &frame.lines[0];
    let shown: Vec<&str> = (0..14).map(|x| row.text(x, x + 1)).collect();
    let flag = "🇯🇵";
    let family = "👨\u{200d}👩\u{200d}👧";
    // Two clusters side by side that begin alike, and two cells that are alike.
    let wanted = [
        flag, "", family, "", "e\u{301}", "e\u{302}", "a", "a", "界", "", "界", "", "y", "",
    ];
    assert_eq!(shown, wanted);
    let spare = |x: usize| (row.cells[x].spacer_tail, row.cells[x].spacer_head);
    assert_eq!(
        [spare(0), spare(1), spare(9), spare(12)],
        [(false, false), (true, false), (true, false), (false, false)]
    );
    // The last column is too narrow for a wide glyph, which goes to the next row.
    assert_eq!((spare(13), row.wrapped), ((false, true), true));
    assert_eq!(frame.lines[1].text(0, 2), "界");
    let all = format!("{flag}{family}e\u{301}e\u{302}aa界界y界");
    assert_eq!(text(&frame), all);
}

#[test]
fn looks_are_reversed_hidden_and_laid_on_a_colour_of_their_own() {
    let mut terminal = terminal(12, 2);
    terminal.vt_write(b"\x1b[31;44mab\x1b[7mcd\x1b[27;8mef\x1b[m\x1b[42m\x1b[K\r\n");
    // Two cells erased to one colour, two to another, two left alone.
    terminal.vt_write(b"\x1b[48;2;1;2;3m\x1b[2X\x1b[2C\x1b[42m\x1b[2X\x1b[m\x1b[4C");
    terminal.vt_write(b"\x1b[4;58;5;1mu");
    let frame = render(&terminal);
    let look = |x: usize, y: usize| *frame.lines[y].look(&frame.lines[y].cells[x]);
    let colours = |x, y| (look(x, y).fg, look(x, y).bg);
    let (red, blue, green) = (0xcc6666, 0x81a2be, 0xb5bd68);
    assert_eq!([colours(0, 0), colours(2, 0)], [(red, blue), (blue, red)]);
    // Hidden text is not there to draw or to copy, and its cells still are.
    assert_eq!(
        (colours(4, 0), frame.lines[0].text(4, 6)),
        ((red, blue), "")
    );
    // A cell a program erased holds the colour it was erased to, and no style.
    let erased = [colours(6, 0), colours(11, 0), colours(1, 1), colours(2, 1)];
    let ground = |color| (frame.fg, color);
    assert_eq!(erased, [green, green, 0x010203, green].map(ground));
    assert_eq!(colours(4, 1), (frame.fg, frame.bg));
    let line = (look(6, 1).style.underline, look(6, 1).underline_color);
    assert_eq!(line, (libghostty_vt::style::Underline::Single, Some(red)));
    assert_eq!(text(&frame), "abcd\n      u");
}

#[test]
fn a_link_covers_the_cells_it_was_written_over_and_no_others() {
    let mut terminal = terminal(20, 1);
    let link = |target: &str| format!("\x1b]8;;{target}\x1b\\");
    let (first, second) = ("https://a.example/", "https://b.example/");
    // Cells that are alike in all but where they lead.
    let bytes = format!("xx{}xx{}xx{}xx", link(first), link(second), link(""));
    terminal.vt_write(bytes.as_bytes());
    let frame = render(&terminal);
    let links = frame.lines[0].links.iter();
    let links: Vec<_> = links
        .map(|(start, end, target)| (*start, *end, &**target))
        .collect();
    assert_eq!(links, [(2, 4, first), (4, 6, second)]);
    assert_eq!(text(&frame), "xxxxxxxx");
}

#[test]
fn a_cursor_that_only_moved_and_colours_that_changed_are_drawn() {
    let mut terminal = terminal(20, 3);
    terminal.vt_write(b"\x1b[31mred\x1b[m plain");
    let mut renderer = Renderer::new().unwrap();
    let first = renderer.frame(&terminal).unwrap().unwrap();
    assert_eq!(first.cursor, Some((9, 0)));
    assert!(renderer.frame(&terminal).unwrap().is_none());
    let mut drawn = |terminal: &mut Terminal<'static, 'static>, bytes: &[u8]| {
        terminal.vt_write(bytes);
        let frame = renderer.frame(terminal).unwrap().expect("a change to draw");
        assert!(renderer.frame(terminal).unwrap().is_none());
        frame
    };
    // Moved along its row, hidden, reshaped: none of it writes a cell, and every
    // row is kept.
    let moved = drawn(&mut terminal, b"\x1b[1;2H");
    assert_eq!(moved.cursor, Some((1, 0)));
    assert!(Arc::ptr_eq(&first.lines[0], &moved.lines[0]));
    assert_eq!(drawn(&mut terminal, b"\x1b[?25l").cursor, None);
    let shaped = drawn(&mut terminal, b"\x1b[?25h\x1b[6 q");
    let cursor = (shaped.cursor, shaped.cursor_shape);
    assert_eq!(cursor, (Some((1, 0)), CursorShape::Bar));
    assert!(Arc::ptr_eq(&first.lines[0], &shaped.lines[0]));
    let colored = drawn(&mut terminal, b"\x1b]12;#0a0b0c\x1b\\");
    assert_eq!(colored.cursor_color, 0x0a0b0c);
    // The colours cells fall back on are held by every look.
    let ground = drawn(&mut terminal, b"\x1b]10;#f0f0f0\x1b\\\x1b]11;#102030\x1b\\");
    assert_eq!((ground.fg, ground.bg), (0xf0f0f0, 0x102030));
    let plain = ground.lines[0].look(&ground.lines[0].cells[5]);
    assert_eq!((plain.fg, plain.bg), (0xf0f0f0, 0x102030));
    let reversed = drawn(&mut terminal, b"\x1b[?5h");
    assert_eq!((reversed.fg, reversed.bg), (0x102030, 0xf0f0f0));
}

#[test]
fn a_theme_keeps_none_of_the_colours_a_program_set() {
    let mut terminal = terminal(20, 2);
    terminal.vt_write(b"\x1b]4;1;#aabbcc\x1b\\");
    let theme = serde_json::json!({"type": "theme", "palette": ["2=#010203"]});
    apply_metadata(&mut terminal, &theme).unwrap();
    let default = terminal.default_color_palette().unwrap().0;
    assert_eq!(rgb(default[2]), 0x010203);
    assert_ne!(rgb(default[1]), 0xaabbcc);
    // The program's colour still shows, until the program gives it up.
    assert_eq!(rgb(terminal.color_palette().unwrap().0[1]), 0xaabbcc);
    terminal.vt_write(b"\x1b]104;1\x1b\\");
    assert_ne!(rgb(terminal.color_palette().unwrap().0[1]), 0xaabbcc);
}

#[test]
fn copying_soft_wrapped_lines_does_not_insert_newlines() {
    let mut terminal = terminal(5, 3);
    terminal.vt_write(b"abcdefgh\r\nlast");
    let frame = render(&terminal);
    assert_eq!(frame.selected_text(0, 14), "abcdefgh\nlast");
}

#[test]
fn worker_wakes_for_output_and_sleeps_when_idle() {
    let mut server = terminal(20, 2);
    server.vt_write(b"hello");
    let (handle, wakes) = worker(encode(&mut server));
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(top(&handle), "hello");
    assert!(wakes.recv_timeout(QUIET).is_err());
    write(&handle, b" world");
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(top(&handle), "hello world");
    drop(handle);
    assert!(matches!(
        wakes.recv_timeout(WAIT),
        Err(flume::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn what_stops_a_worker_is_told_by_its_handle() {
    let snapshot = encode(&mut terminal(20, 2));
    let malformed = [snapshot.as_slice(), b"\x1b_rex1;7;{\x1b\\"].concat();
    for (bytes, cause) in [
        (
            b"no snapshot begins so".to_vec(),
            "restoring terminal snapshot",
        ),
        (malformed, "invalid Rex metadata"),
    ] {
        let (handle, wakes) = worker(bytes);
        let status = loop {
            wakes.recv_timeout(WAIT).unwrap();
            if let Some(status) = handle.status() {
                break status;
            }
        };
        assert!(status.contains(cause), "{status}");
        assert!(handle.scroll(1).is_err());
    }
}

#[test]
fn a_held_frame_is_drawn_whole_as_the_next_hold_begins() {
    let (handle, wakes) = worker(encode(&mut terminal(20, 2)));
    wakes.recv_timeout(WAIT).unwrap();
    // One frame held and finished, and the next begun and left half drawn, all in
    // one write.
    write(&handle, b"\x1b[?2026hone\x1b[?2026l\x1b[?2026h\rtw");
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(top(&handle), "one");
    assert!(wakes.recv_timeout(QUIET).is_err());
    write(&handle, b"o\x1b[?2026l");
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(top(&handle), "two");
}

#[test]
fn holds_without_number_in_one_write_are_not_each_drawn() {
    let (handle, wakes) = worker(encode(&mut terminal(20, 2)));
    wakes.recv_timeout(WAIT).unwrap();
    thread::sleep(FRAME_INTERVAL * 2);
    let mut frames = Vec::new();
    for frame in 0..500 {
        frames.extend_from_slice(format!("\x1b[?2026h\r{frame}\x1b[?2026l").as_bytes());
    }
    write(&handle, &frames);
    let mut drawn = 0;
    while wakes.recv_timeout(QUIET).is_ok() {
        drawn += 1;
    }
    // The first to come when one was due, and the last, once all was written.
    assert!((1..=3).contains(&drawn), "{drawn}");
    assert_eq!(top(&handle), "499");
}

#[test]
fn a_hold_that_never_ends_is_given_up_after_its_limit() {
    let (handle, wakes) = worker(encode(&mut terminal(20, 2)));
    wakes.recv_timeout(WAIT).unwrap();
    let began = Instant::now();
    write(&handle, b"\x1b[?2026hstuck");
    let drawn = wakes.recv_timeout(WAIT).unwrap();
    assert!(drawn.duration_since(began) >= RENDER_HOLD_LIMIT);
    assert_eq!(top(&handle), "stuck");
    // It is over for good: what follows is drawn as it comes.
    write(&handle, b"!");
    let next = wakes.recv_timeout(WAIT).unwrap();
    assert!(next.duration_since(drawn) < RENDER_HOLD_LIMIT);
    assert_eq!(top(&handle), "stuck!");
}

#[test]
fn the_first_frame_after_a_restore_is_drawn_inside_a_hold() {
    let mut server = terminal(20, 2);
    server.vt_write(b"shown\x1b[?2026h");
    let began = Instant::now();
    let (handle, wakes) = worker(encode(&mut server));
    let drawn = wakes.recv_timeout(WAIT).unwrap();
    assert!(drawn.duration_since(began) < RENDER_HOLD_LIMIT);
    assert_eq!(top(&handle), "shown");
    // The hold it was taken in stands until its program ends it.
    write(&handle, b"\rlater");
    assert!(wakes.recv_timeout(QUIET).is_err());
    write(&handle, b"\x1b[?2026l");
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(top(&handle), "later");
}

#[test]
fn scrolling_draws_through_a_hold_and_staying_put_does_not() {
    let mut server = terminal(20, 2);
    server.vt_write(b"1\r\n2\r\n3\r\n4");
    let (handle, wakes) = worker(encode(&mut server));
    wakes.recv_timeout(WAIT).unwrap();
    let began = Instant::now();
    write(&handle, b"\x1b[?2026h\rheld");
    // Every key press asks for the bottom, where the view already is.
    handle.scroll_to_bottom();
    assert!(wakes.recv_timeout(QUIET).is_err());
    handle.scroll(-1).unwrap();
    let drawn = wakes.recv_timeout(WAIT).unwrap();
    assert!(drawn.duration_since(began) < RENDER_HOLD_LIMIT);
    assert_eq!(handle.latest().unwrap().scroll_offset, 1);
    assert_eq!(top(&handle), "2");
}

#[test]
fn the_view_moves_by_pages_and_by_shell_prompts() {
    // Prompts on rows 0, 3, 6 and 9 of ten, the last two of them on screen.
    let mut server = terminal(20, 2);
    for prompt in 1..=4 {
        server.vt_write(format!("\x1b]133;P;k=i\x07p{prompt}").as_bytes());
        if prompt < 4 {
            server.vt_write(b"\r\n\x1b]133;B\x07cmd\r\n\x1b]133;C\x07out\r\n");
        }
    }
    let (handle, wakes) = worker(encode(&mut server));
    wakes.recv_timeout(WAIT).unwrap();
    let after = |asked: Result<()>| {
        asked.unwrap();
        wakes.recv_timeout(WAIT).unwrap();
        (handle.latest().unwrap().scroll_offset, top(&handle))
    };
    assert_eq!(after(handle.scroll_prompts(-1)), (2, "p3".into()));
    assert_eq!(after(handle.scroll_pages(-1)), (4, "cmd".into()));
    assert_eq!(after(handle.scroll_prompts(-1)), (5, "p2".into()));
    assert_eq!(after(handle.scroll_prompts(1)), (2, "p3".into()));
    handle.scroll_to_bottom();
    wakes.recv_timeout(WAIT).unwrap();
    assert_eq!(
        (handle.latest().unwrap().scroll_offset, top(&handle)),
        (0, "out".into())
    );
}

#[test]
fn frames_tell_of_the_alternate_screen_and_its_scrolling() {
    let mut terminal = terminal(20, 2);
    let mut renderer = Renderer::new().unwrap();
    let primary = renderer.frame(&terminal).unwrap().unwrap();
    // Ghostty has the wheel send arrows there unless a program says otherwise.
    assert!(!primary.alternate_screen && primary.alternate_scroll);
    terminal.vt_write(b"\x1b[?1049h");
    assert!(renderer.frame(&terminal).unwrap().unwrap().alternate_screen);
    // A mode dirties nothing, and is a new frame all the same.
    terminal.vt_write(b"\x1b[?1007l");
    assert!(!renderer.frame(&terminal).unwrap().unwrap().alternate_scroll);
    assert!(renderer.frame(&terminal).unwrap().is_none());
}

#[test]
fn output_under_a_view_scrolled_back_is_drawn_only_where_it_shows() {
    let mut terminal = terminal(20, 3);
    // As little history as Ghostty will keep, so that it is soon cut away.
    terminal.set_scrollback_max_bytes(Some(1)).unwrap();
    let mut written = 0;
    let mut lines = |terminal: &mut Terminal, count| {
        for _ in 0..count {
            written += 1;
            terminal.vt_write(format!("line {written}\r\n").as_bytes());
        }
    };
    let mut renderer = Renderer::new().unwrap();
    let mut shown = None;
    // Draw, and say whether that made a frame. Either way the last one made is
    // what a renderer that remembers nothing would draw now.
    let mut draw = |terminal: &Terminal<'static, 'static>| {
        let drawn = renderer.frame(terminal).unwrap();
        let new = drawn.is_some();
        shown = drawn.or(shown.take());
        let (shown, afresh) = (shown.as_ref().unwrap(), render(terminal));
        assert_eq!(text(shown), text(&afresh));
        assert_eq!(shown.cursor, afresh.cursor);
        assert_eq!(shown.scroll_offset == 0, afresh.scroll_offset == 0);
        new
    };
    lines(&mut terminal, 100);
    assert!(draw(&terminal));

    // Wholly in history: output below changes nothing in view.
    terminal.scroll_viewport(ScrollViewport::Delta(-10));
    assert!(draw(&terminal));
    lines(&mut terminal, 5);
    terminal.vt_write(b"\x1b[?25l\x1b[?25h");
    assert!(!draw(&terminal));
    // One row short of the bottom: the first two rows of the screen are in view.
    terminal.scroll_viewport(ScrollViewport::Delta(14));
    assert!(draw(&terminal));
    terminal.vt_write(b"\x1b[3;1Hunseen");
    assert!(!draw(&terminal));
    terminal.vt_write(b"\x1b[1;1Hseen\x1b[3;1H\r\n");
    assert!(draw(&terminal));

    // History cut away from under the view, at its top and then within it.
    for down in [0, 2] {
        terminal.scroll_viewport(ScrollViewport::Top);
        terminal.scroll_viewport(ScrollViewport::Delta(down));
        assert!(draw(&terminal));
        let (mut kept, mut cut) = (0, 0);
        while cut == 0 {
            let before = terminal.total_rows().unwrap();
            lines(&mut terminal, 500);
            assert!(before < 1_000_000, "history is never cut");
            let trimmed = terminal.total_rows().unwrap() < before + 500;
            match (draw(&terminal), trimmed) {
                (false, false) => kept += 1,
                (true, true) => cut += 1,
                other => panic!("drawn and trimmed: {other:?}"),
            }
        }
        assert!(kept > 0);
    }

    // A clear of the history, and a resize.
    terminal.scroll_viewport(ScrollViewport::Bottom);
    terminal.scroll_viewport(ScrollViewport::Delta(-10));
    assert!(draw(&terminal));
    terminal.vt_write(b"\x1b[3J");
    assert!(draw(&terminal));
    lines(&mut terminal, 20);
    terminal.scroll_viewport(ScrollViewport::Delta(-10));
    assert!(draw(&terminal));
    terminal.resize(30, 3, 0, 0).unwrap();
    assert!(draw(&terminal));
}

#[test]
fn links_are_those_marked_and_those_written_out() {
    let mut terminal = terminal(20, 4);
    terminal.vt_write(b"a \x1b]8;;https://rex.example/docs\x1b\\docs\x1b]8;;\x1b\\ b\r\n");
    // An address that wraps, in brackets, at the end of a sentence.
    terminal.vt_write(b"see (https://example.com/a_(b)).\r\n");
    terminal.vt_write(b"\x1b]8;;file:///etc/passwd\x1b\\x\x1b]8;;\x1b\\ ftp.example.com");
    let frame = render(&terminal);
    let link = |x, y| crate::link::at(&frame, x, y).map(|link| (link.target, link.spans));
    // A marked link leads where it says, not where its text reads.
    let marked = Some(("https://rex.example/docs".to_owned(), vec![(0, 2, 6)]));
    assert_eq!(link(2, 0), marked);
    assert_eq!(link(5, 0), marked);
    assert_eq!((link(1, 0), link(6, 0)), (None, None));
    let written = "https://example.com/a_(b)".to_owned();
    let written = Some((written, vec![(1, 5, 20), (2, 0, 10)]));
    assert_eq!(link(5, 1), written);
    assert_eq!(link(9, 2), written);
    // Neither the bracket before it nor what closes the sentence after it.
    assert_eq!((link(4, 1), link(10, 2), link(11, 2)), (None, None, None));
    // A file is never followed, and a name without its kind is no address.
    assert_eq!((link(0, 3), link(5, 3)), (None, None));
}

/// Counts what each thread asks of the allocator, for the benchmarks.
struct Counting;

thread_local! {
    /// How many times this thread has asked for memory, and for how many bytes.
    static ASKED: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ASKED.set((ASKED.get().0 + 1, ASKED.get().1 + layout.size() as u64));
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, memory: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(memory, layout) }
    }

    unsafe fn realloc(&self, memory: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        ASKED.set((ASKED.get().0 + 1, ASKED.get().1 + size as u64));
        unsafe { std::alloc::System.realloc(memory, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The time and the memory that the measured part of a benchmark's frames took.
#[derive(Default)]
pub(crate) struct Meter {
    spent: Duration,
    asked: (u64, u64),
}

impl Meter {
    pub(crate) fn time<T>(&mut self, work: impl FnOnce() -> T) -> T {
        let (before, began) = (ASKED.get(), Instant::now());
        let done = work();
        self.spent += began.elapsed();
        let after = ASKED.get();
        self.asked.0 += after.0 - before.0;
        self.asked.1 += after.1 - before.1;
        done
    }
}

/// Run rounds of frames and print what one frame costs. The first round is not counted.
pub(crate) fn measure(name: &str, mut frame: impl FnMut(&mut Meter)) {
    const FRAMES: u32 = 200;
    let mut round = || {
        let mut meter = Meter::default();
        (0..FRAMES).for_each(|_| frame(&mut meter));
        meter
    };
    round();
    let mut rounds: Vec<Meter> = (0..5).map(|_| round()).collect();
    rounds.sort_by_key(|meter| meter.spent);
    let micros = |meter: &Meter| meter.spent.as_secs_f64() * 1e6 / f64::from(FRAMES);
    let (count, bytes) = rounds[2].asked;
    println!(
        "{name:<42}{:8.1} µs a frame at best, {:8.1} at the median; {:6} allocations, {:5} KB",
        micros(&rounds[0]),
        micros(&rounds[2]),
        count / u64::from(FRAMES),
        bytes / 1024 / u64::from(FRAMES),
    );
}

/// A row of coloured words, 200 columns of them, unlike any other line's.
fn coloured(line: usize) -> String {
    use std::fmt::Write as _;
    let mut row = String::new();
    for word in line * 20..line * 20 + 20 {
        let look = ["", "", ";1", "", ";4", ";48;5;236", "", ";3"][word % 8];
        let ink = word % 200 + 20;
        let text = word * 2_654_435_761 % 0xf_ffff_ffff;
        write!(row, "\x1b[0;38;5;{ink}{look}m{text:<9x} ").unwrap();
    }
    row
}

/// A row of wide glyphs and of cells that hold several characters.
fn clustered(line: usize) -> String {
    const WORDS: [&str; 8] = [
        "界面テスト ",
        "👨\u{200d}👩\u{200d}👧 ",
        "e\u{301}a\u{308}o\u{302} ",
        "한국어 ",
        "🇯🇵🇫🇷 ",
        "🙂👍🏽 ",
        "plain ",
        "日本語の文章 ",
    ];
    // Short of the last column, so that no glyph wraps.
    (line..line + 22).map(|word| WORDS[word % 8]).collect()
}

/// What a program writes for the nth frame of a screen.
pub(crate) type Program = fn(usize) -> String;

/// The screens the benchmarks draw, 200 columns by 60 rows, each with its name.
pub(crate) const SCREENS: [(&str, Program); 4] = [
    ("coloured text, all redrawn", |frame| {
        let rows: Vec<String> = (frame * 60..frame * 60 + 60).map(coloured).collect();
        format!("\x1b[H{}", rows.join("\r\n"))
    }),
    ("coloured text, one line scrolls", |frame| {
        format!("\r\n{}", coloured(frame))
    }),
    ("wide glyphs and clusters", |frame| {
        let rows: Vec<String> = (frame..frame + 60).map(clustered).collect();
        format!("\x1b[H\x1b[J{}", rows.join("\r\n"))
    }),
    ("a shell's short lines, all redrawn", |frame| {
        let prompt = "\x1b[32muser@host\x1b[m:\x1b[34m~/src\x1b[m$";
        let line = |line: usize| format!("{prompt} make {:x}\x1b[K", line * 2_654_435_761);
        let rows: Vec<String> = (frame * 60..frame * 60 + 60).map(line).collect();
        format!("\x1b[H{}", rows.join("\r\n"))
    }),
];

/// One of those screens, a frame at a time.
pub(crate) struct Bench {
    terminal: Terminal<'static, 'static>,
    renderer: Renderer,
    write: Program,
    frame: usize,
}

impl Bench {
    pub(crate) fn new(write: Program) -> Self {
        let mut terminal = terminal(200, 60);
        configure(&mut terminal).unwrap();
        // Characters that belong together share a cell, as a shell asks of Rex.
        terminal.vt_write(b"\x1b[?2027h");
        let renderer = Renderer::new().unwrap();
        let mut screen = Self {
            terminal,
            renderer,
            write,
            frame: 0,
        };
        // Enough frames that a screen which scrolls is full.
        (0..60).for_each(|_| drop(screen.next(&mut Meter::default())));
        screen
    }

    /// Have the program write its next frame, and draw it. Only the drawing is timed.
    pub(crate) fn next(&mut self, meter: &mut Meter) -> Arc<Frame> {
        self.terminal.vt_write((self.write)(self.frame).as_bytes());
        self.frame += 1;
        let drawn = meter.time(|| self.renderer.frame(&self.terminal));
        drawn.unwrap().expect("a frame unlike the last")
    }
}

#[test]
#[ignore = "a benchmark: run it alone, with --release and --nocapture"]
fn bench_frames() {
    for (name, write) in SCREENS {
        let mut screen = Bench::new(write);
        let mut last = screen.next(&mut Meter::default());
        let (mut frames, mut new) = (0, 0);
        measure(name, |meter| {
            let frame = screen.next(meter);
            let old = |row: &&Arc<Row>| last.lines.iter().any(|old| Arc::ptr_eq(old, row));
            new += frame.lines.len() - frame.lines.iter().filter(old).count();
            frames += 1;
            last = frame;
        });
        println!(
            "{:42}{:8.1} rows a frame are new",
            "",
            new as f64 / frames as f64
        );
    }
}
