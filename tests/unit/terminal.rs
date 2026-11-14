use super::*;

fn terminal(cols: u16, rows: u16) -> Terminal {
    Terminal::new(ghostty_vt::TerminalOptions {
        cols,
        rows,
        ..Default::default()
    })
    .unwrap()
}

fn render(terminal: &Terminal) -> Arc<Frame> {
    Renderer::new().unwrap().frame(terminal).unwrap().unwrap()
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
    let mut bytes = snapshot::encode(&original).unwrap().to_vec();
    let snapshot_len = bytes.len();
    bytes.extend_from_slice(b"1;2;3mX");
    let mut input = io::Cursor::new(bytes);
    let mut source = SnapshotSource::new(Fragmented(&mut input));
    let mut restored = snapshot::Decoder::new(&mut source)
        .unwrap()
        .decode()
        .unwrap();
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
    let mut damaged = snapshot::encode(&terminal(20, 2)).unwrap().to_vec();
    damaged[16] ^= 1;
    assert!(
        snapshot::Decoder::new(SnapshotSource::new(damaged.as_slice()))
            .unwrap()
            .decode()
            .is_err()
    );
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
    assert!(original.kitty_graphics().unwrap().image(42).is_some());
    let bytes = snapshot::encode(&original).unwrap();
    let mut restored = snapshot::Decoder::new(SnapshotSource::new(bytes.as_ref()))
        .unwrap()
        .decode()
        .unwrap();
    assert!(restored.kitty_graphics().unwrap().image(42).is_some());
    configure(&mut restored).unwrap();
    assert_eq!(restored.kitty_image_storage_limit().unwrap(), 0);
    assert!(!restored.is_kitty_image_from_file_allowed().unwrap());
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
        .send(Command::Bytes(snapshot::encode(&server).unwrap().to_vec()))
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
