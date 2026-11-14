use super::*;

fn terminal(cols: u16, rows: u16) -> Terminal<'static, 'static> {
    Terminal::new(cols, rows).unwrap()
}

fn render(terminal: &Terminal<'static, '_>) -> Arc<Frame> {
    Renderer::new().unwrap().frame(terminal).unwrap().unwrap()
}

fn encode(terminal: &mut Terminal) -> Vec<u8> {
    let mut bytes = Vec::new();
    terminal.encode_snapshot(&mut bytes).unwrap();
    bytes
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
    assert_eq!(frame.lines[2].cells[4].text, "X");
    assert_eq!(frame.lines[2].cells[4].fg, 0x010203);
}

#[test]
fn snapshot_rejects_crc_damage_and_oversized_records() {
    let mut damaged = encode(&mut terminal(20, 2));
    damaged[16] ^= 1;
    assert!(restore(damaged.as_slice()).is_err());
    let mut oversized = b"GHOSTSNP\x01\x00".to_vec();
    oversized.extend_from_slice(&1u16.to_le_bytes());
    oversized.extend_from_slice(&((MAX_RECORD_BYTES + 1) as u32).to_le_bytes());
    oversized.extend_from_slice(&[0; 4]);
    let mut output = Vec::new();
    assert!(
        SnapshotSource::new(oversized.as_slice())
            .read_to_end(&mut output)
            .is_err()
    );
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
    // The colours cells fall back on are held by every cell.
    let ground = drawn(&mut terminal, b"\x1b]10;#f0f0f0\x1b\\\x1b]11;#102030\x1b\\");
    assert_eq!((ground.fg, ground.bg), (0xf0f0f0, 0x102030));
    let plain = &ground.lines[0].cells[5];
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
    let (wake, wakes) = flume::unbounded();
    let handle = TerminalHandle::new(
        7,
        Arc::new(move || {
            let _ = wake.send(());
        }),
    )
    .unwrap();
    let mut server = terminal(20, 2);
    server.vt_write(b"hello");
    handle
        .input
        .send(Command::Bytes(encode(&mut server)))
        .unwrap();
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(handle.latest().unwrap().selected_text(0, 4), "hello");
    assert!(wakes.recv_timeout(Duration::from_millis(80)).is_err());
    handle
        .input
        .send(Command::Bytes(b" world".to_vec()))
        .unwrap();
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(handle.latest().unwrap().selected_text(0, 10), "hello world");
    drop(handle);
    assert!(matches!(
        wakes.recv_timeout(Duration::from_secs(3)),
        Err(flume::RecvTimeoutError::Disconnected)
    ));
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
