use super::*;
use crate::terminal::{
    Request, select,
    tests::{encode, lined, long, render, row, terminal, until, worker, write},
};

/// What is selected, which a match found is.
fn selected(terminal: &Terminal) -> Option<String> {
    select::text(terminal).unwrap()
}

/// Look until there is no further to look.
fn find(finding: &mut Finding, terminal: &mut Terminal<'static, '_>, text: &str) {
    finding.look(terminal, text).unwrap();
    while finding.work(terminal, false).unwrap() {}
}

#[test]
fn text_is_found_through_all_of_history_and_its_matches_walked() {
    let mut terminal = long();
    let mut finding = Finding::default();
    assert_eq!(finding.found().unwrap(), None);
    // The row of the match selected, which is brought into view.
    let shown = |terminal: &Terminal<'static, '_>| {
        let frame = render(terminal);
        let y = frame.selected.iter().position(Option::is_some).unwrap();
        (row(&frame, y), frame.selected[y])
    };
    // Letters match in either case.
    find(&mut finding, &mut terminal, "LINE 4242");
    assert_eq!(finding.found().unwrap(), Some((1, 1)));
    assert_eq!(selected(&terminal).as_deref(), Some("line 4242"));
    assert_eq!(shown(&terminal), ("line 4242".into(), Some((0, 9))));
    // Lines 99, 990 to 999 and 9900 to 9999: the last of them comes first.
    find(&mut finding, &mut terminal, "line 99");
    assert_eq!(finding.found().unwrap(), Some((1, 111)));
    assert_eq!(shown(&terminal).0, "line 9999");
    finding.step(&mut terminal, true).unwrap();
    assert_eq!(finding.found().unwrap(), Some((2, 111)));
    assert_eq!(shown(&terminal).0, "line 9998");
    // And round from the newest to the oldest.
    finding.step(&mut terminal, false).unwrap();
    finding.step(&mut terminal, false).unwrap();
    assert_eq!(finding.found().unwrap(), Some((111, 111)));
    assert_eq!(shown(&terminal), ("line 99".into(), Some((0, 7))));
    // Text that is nowhere leaves nothing selected.
    find(&mut finding, &mut terminal, "no such text");
    assert_eq!(finding.found().unwrap(), Some((0, 0)));
    assert_eq!(selected(&terminal), None);
    // With nothing looked for, the last match stays selected.
    find(&mut finding, &mut terminal, "Line 0");
    find(&mut finding, &mut terminal, "");
    assert_eq!(finding.found().unwrap(), None);
    assert_eq!(selected(&terminal).as_deref(), Some("line 0"));
}

#[test]
fn a_match_is_shown_clear_of_the_field_that_looks_for_it() {
    let mut terminal = lined(12, 100);
    let mut finding = Finding::default();
    let shown = |terminal: &Terminal<'static, '_>| {
        render(terminal).selected.iter().position(Option::is_some)
    };
    find(&mut finding, &mut terminal, "line 50");
    assert_eq!(shown(&terminal), Some(4));
    // With no history above it to show, it stays where it is.
    find(&mut finding, &mut terminal, "line 0");
    assert_eq!(shown(&terminal), Some(0));
}

#[test]
fn a_worker_finds_as_asked_and_says_how_many_in_its_frames() {
    let mut server = terminal(20, 3);
    for line in 0..500 {
        server.vt_write(format!("line {line}\r\n").as_bytes());
    }
    let (handle, wakes) = worker(encode(&mut server));
    let found = || handle.latest().and_then(|frame| frame.found);
    handle.ask(Request::Find("line 7".into()));
    // Lines 7, 70 to 79 and 700 on, of which there are none.
    until(&wakes, || found() == Some((1, 11)));
    handle.ask(Request::Step { older: true });
    until(&wakes, || found() == Some((2, 11)));
    let frame = handle.latest().unwrap();
    assert!(frame.scroll_offset > 0 && frame.text().contains("line 78"));
    // New output is looked through as it comes.
    write(&handle, b"line 7 again");
    until(&wakes, || found() == Some((3, 12)));
    handle.ask(Request::Find(String::new()));
    until(&wakes, || found().is_none());
}
