use std::sync::Arc;

use libghostty_vt::terminal::ScrollViewport;

use super::*;
use crate::terminal::{
    Copied, Renderer, Request, TerminalHandle,
    tests::{QUIET, encode, render, terminal, until, worker, write},
};

/// Press in the middle of a cell and let go, as often as `times`.
fn click(selecting: &mut Selecting, terminal: &Terminal, (x, y): (u16, u16), times: usize) {
    for _ in 0..times {
        let at = [f32::from(x) + 0.5, f32::from(y) + 0.5];
        selecting.press(terminal, at, false).unwrap();
        selecting.release(terminal).unwrap();
    }
}

/// Press at the left of one cell and drag to the right of another.
fn drag(selecting: &mut Selecting, terminal: &Terminal, from: (u16, u16), to: (u16, u16)) {
    let at = |(x, y): (u16, u16), within: f32| [f32::from(x) + within, f32::from(y) + 0.5];
    selecting.press(terminal, at(from, 0.1), false).unwrap();
    selecting.drag(terminal, at(to, 0.9), false).unwrap();
}

/// Ask a worker for a press at one place and a drag to another, in cells.
fn drag_asked(handle: &TerminalHandle, from: [f32; 2], to: [f32; 2]) {
    let (extend, block) = (false, false);
    handle.ask(Request::Press { at: from, extend });
    handle.ask(Request::Drag { at: to, block });
}

/// The selected text a worker hands back next.
fn handed(handle: &TerminalHandle, wakes: &flume::Receiver<Instant>) -> Copied {
    let mut copied = Vec::new();
    until(wakes, || {
        copied.extend(handle.copied());
        !copied.is_empty()
    });
    copied.remove(0)
}

/// What copying would copy.
fn copied(terminal: &Terminal) -> String {
    text(terminal).unwrap().unwrap_or_default()
}

#[test]
fn a_selection_stays_with_its_text_through_output_and_scrolling() {
    let mut terminal = terminal(10, 3);
    terminal.vt_write(b"one\r\ntwo\r\nthree");
    let mut renderer = Renderer::new().unwrap();
    let mut selecting = Selecting::new().unwrap();
    let mut frame = |terminal: &Terminal<'static, '_>| renderer.frame(terminal).unwrap().unwrap();
    let plain = frame(&terminal);
    assert!(plain.selected.is_empty());
    drag(&mut selecting, &terminal, (0, 1), (2, 1));
    let made = frame(&terminal);
    assert_eq!(made.selected, [None, Some((0, 3)), None]);
    // No row is another row for being selected.
    assert!((0..3).all(|y| Arc::ptr_eq(&plain.lines[y], &made.lines[y])));
    // Output moves it up the screen, and then off it.
    terminal.vt_write(b"\r\nfour");
    assert_eq!(frame(&terminal).selected, [Some((0, 3)), None, None]);
    terminal.vt_write(b"\r\nfive\r\nsix");
    assert_eq!(frame(&terminal).selected, [None; 3]);
    assert_eq!(copied(&terminal), "two");
    terminal.scroll_viewport(ScrollViewport::Top);
    assert_eq!(frame(&terminal).selected, [None, Some((0, 3)), None]);
    // A press that is no drag lets go of it.
    click(&mut selecting, &terminal, (5, 2), 1);
    assert!(frame(&terminal).selected.is_empty());
    assert_eq!(text(&terminal).unwrap(), None);
}

#[test]
fn two_presses_select_a_word_and_three_a_line_though_either_wraps() {
    let mut terminal = terminal(5, 4);
    terminal.vt_write(b"ab cdefgh ij\r\nlast");
    let mut selecting = Selecting::new().unwrap();
    click(&mut selecting, &terminal, (1, 1), 2);
    assert_eq!(copied(&terminal), "cdefgh");
    let word = [Some((3, 5)), Some((0, 4)), None, None];
    assert_eq!(render(&terminal).selected, word);
    click(&mut selecting, &terminal, (1, 1), 1);
    assert_eq!(copied(&terminal), "ab cdefgh ij");
    let line = [Some((0, 5)), Some((0, 5)), Some((0, 2)), None];
    assert_eq!(render(&terminal).selected, line);
    // A press elsewhere begins again at one.
    click(&mut selecting, &terminal, (1, 3), 1);
    assert_eq!(text(&terminal).unwrap(), None);

    // Only blanks part words, so an address is one.
    let mut terminal = self::terminal(40, 2);
    terminal.vt_write(b"see https://host:8080/a?b=(c),d now");
    click(&mut selecting, &terminal, (12, 0), 2);
    assert_eq!(copied(&terminal), "https://host:8080/a?b=(c),d");
}

#[test]
fn wide_glyphs_and_clusters_at_a_selections_edges_are_copied_whole_and_once() {
    let mut terminal = terminal(20, 2);
    // Characters that belong together share a cell, as a shell asks of Rex.
    terminal.vt_write("\x1b[?2027h界 e\u{301} 👍🏽x".as_bytes());
    let mut selecting = Selecting::new().unwrap();
    // From the spare half of one wide glyph to the first half of another.
    drag(&mut selecting, &terminal, (1, 0), (5, 0));
    assert_eq!(copied(&terminal), "界 e\u{301} 👍🏽");
    assert_eq!(render(&terminal).selected, [Some((0, 7)), None]);
    // And back the other way, between the two.
    selecting.press(&terminal, [4.9, 0.5], false).unwrap();
    selecting.drag(&terminal, [2.1, 0.5], false).unwrap();
    // The blanks that end a line are not copied.
    assert_eq!(copied(&terminal), " e\u{301}");
    // With Shift a press reaches from where the last began.
    selecting.release(&terminal).unwrap();
    selecting.press(&terminal, [7.9, 0.5], true).unwrap();
    assert_eq!(copied(&terminal), "👍🏽x");
}

#[test]
fn a_block_is_selected_by_columns() {
    let mut terminal = terminal(10, 2);
    terminal.vt_write(b"abcd\r\nefgh\r\nijkl");
    let mut selecting = Selecting::new().unwrap();
    selecting.press(&terminal, [1.1, 0.5], false).unwrap();
    selecting.drag(&terminal, [2.9, 1.5], true).unwrap();
    assert_eq!(copied(&terminal), "fg\njk");
}

#[test]
fn a_worker_selects_as_asked_and_hands_back_the_text() {
    let mut server = terminal(10, 3);
    server.vt_write(b"one\r\ntwo\r\nthree");
    let (handle, wakes) = worker(encode(&mut server));
    let selected = || handle.latest().map(|frame| frame.selected.clone());
    drag_asked(&handle, [0.1, 1.5], [2.9, 1.5]);
    until(&wakes, || selected() == Some(vec![None, Some((0, 3)), None]));
    assert_eq!(handle.copied(), []);
    // Let go, it is the desktop's selection. Copying is asked for.
    handle.ask(Request::Release);
    assert_eq!(handed(&handle, &wakes), Copied::Selection("two".into()));
    handle.ask(Request::Copy);
    assert_eq!(handed(&handle, &wakes), Copied::Clipboard("two".into()));
    // The pane goes on while something is selected in it.
    write(&handle, b"\r\nfour");
    until(&wakes, || selected() == Some(vec![Some((0, 3)), None, None]));
    assert_eq!(handle.latest().unwrap().text(), "two\nthree\nfour");
    handle.ask(Request::Deselect);
    until(&wakes, || selected() == Some(Vec::new()));
    // Nothing selected is nothing to copy. All of it is the desktop's selection too.
    handle.ask(Request::Copy);
    handle.ask(Request::SelectAll);
    let all = Copied::Selection("one\ntwo\nthree\nfour".into());
    assert_eq!(handed(&handle, &wakes), all);
}

#[test]
fn a_drag_held_past_the_top_scrolls_back_and_goes_on_selecting() {
    let mut server = terminal(10, 3);
    for line in 0..40 {
        server.vt_write(format!("\r\nline {line}").as_bytes());
    }
    let (handle, wakes) = worker(encode(&mut server));
    drag_asked(&handle, [3.9, 1.5], [0.0, -2.0]);
    let offset = || handle.latest().map_or(0, |frame| frame.scroll_offset);
    until(&wakes, || offset() >= 5);
    handle.ask(Request::Release);
    // Every row it passed is selected, down to where it began.
    let Copied::Selection(text) = handed(&handle, &wakes) else {
        panic!("not the desktop's selection");
    };
    assert!(text.lines().count() > 5 && text.ends_with("line 37\nline"), "{text}");
    // Let go, the view comes to rest: a frame may be on its way yet, and no
    // more after it.
    assert!((0..3).any(|_| wakes.recv_timeout(QUIET).is_err()));
}
