use super::*;

#[test]
fn a_note_says_where_it_is_from_in_text_fit_for_the_desktop() {
    let (home, notes) = Herald::new();
    home.say("p1", "w1", "claude", "rex-lite", "Needs approval: Run it?");
    let note = notes.try_recv().unwrap();
    assert_eq!((note.pane.as_str(), note.tab.as_str()), ("p1", "w1"));
    assert_eq!(note.summary, "claude — rex-lite");
    assert_eq!(note.body, "Needs approval: Run it?");
    // Another server's notes come out at the same place, and say whose they are.
    home.of("beta").say("p2", "w2", "far", "remote", "Done");
    assert_eq!(notes.try_recv().unwrap().summary, "far — remote on beta");
    // A program's text is kept to a line, the way round it was written, and short.
    assert_eq!(plain("one\ntwo\u{1b}[0m"), "one two [0m");
    assert_eq!(plain("\u{202e}txt.exe\u{2066}"), "txt.exe");
    let long = plain(&"é".repeat(1000));
    assert_eq!(long.chars().count(), MAX_TEXT + 1);
    assert!(long.ends_with('…'));
}
