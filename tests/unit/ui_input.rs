use super::*;

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        modifiers,
        repeat: false,
    }
}

/// The text, modifiers and direction of a key action.
fn key_of(action: &Action) -> (&str, u8, bool) {
    match action {
        Action::Key {
            key,
            modifiers,
            pressed,
            ..
        } => (key, *modifiers, *pressed),
        other => panic!("expected a key, got {other:?}"),
    }
}

#[test]
fn physical_text_is_sent_once_and_releases_match() {
    let mut keyboard = Keyboard::default();
    let out = keyboard.translate(
        "p",
        &[
            key(Key::A, true, Modifiers::SHIFT),
            Event::Text("A".into()),
            key(Key::A, false, Modifiers::NONE),
        ],
    );
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], Action::Key { code, .. } if code == "key_a"));
    assert_eq!(key_of(&out[0]), ("A", model::SHIFT, true));
    assert_eq!(key_of(&out[1]), ("A", 0, false));
}

#[test]
fn control_chords_reach_the_terminal_as_keys() {
    let mut keyboard = Keyboard::default();
    let ctrl = Modifiers::CTRL;
    let out = keyboard.translate(
        "p",
        &[
            key(Key::ControlLeft, true, ctrl),
            key(Key::C, true, ctrl),
            key(Key::C, false, ctrl),
            key(Key::V, true, ctrl),
            key(Key::V, false, ctrl),
            key(Key::ControlLeft, false, Modifiers::NONE),
        ],
    );
    assert_eq!(out.len(), 6);
    assert_eq!(key_of(&out[1]), ("c", model::CTRL, true));
    assert_eq!(key_of(&out[2]), ("c", model::CTRL, false));
    assert_eq!(key_of(&out[3]), ("v", model::CTRL, true));
    assert_eq!(key_of(&out[4]), ("v", model::CTRL, false));
}

#[test]
fn a_paste_is_sent_once_as_a_paste() {
    let mut keyboard = Keyboard::default();
    let chord = Modifiers::CTRL | Modifiers::SHIFT;
    // The shell consumes the V press and asks for the clipboard; only its release remains.
    let out = keyboard.translate(
        "p",
        &[Event::Paste("hello\n".into()), key(Key::V, false, chord)],
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Action::Paste { text, .. } if text == "hello\n"));
}

#[test]
fn ime_commit_has_no_extra_printable_press() {
    let mut keyboard = Keyboard::default();
    let out = keyboard.translate(
        "p",
        &[
            key(Key::E, true, Modifiers::NONE),
            Event::Ime(ImeEvent::Commit("é".into())),
        ],
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Action::Text { text, .. } if text == "é"));
}

#[test]
fn losing_the_window_releases_held_keys() {
    let mut keyboard = Keyboard::default();
    keyboard.translate("p", &[key(Key::ArrowUp, true, Modifiers::NONE)]);
    let out = keyboard.translate("p", &[Event::WindowFocused(false)]);
    assert_eq!(out.len(), 1);
    assert_eq!(key_of(&out[0]), ("", 0, false));
    assert!(
        keyboard
            .translate("p", &[key(Key::ArrowUp, false, Modifiers::NONE)])
            .is_empty()
    );
}

#[test]
fn only_real_input_clears_a_selection() {
    let mut keyboard = Keyboard::default();
    let ctrl = Modifiers::CTRL;
    let chord = ctrl | Modifiers::SHIFT;
    // A copy chord as the pane sees it: the shell has consumed the C press.
    let actions = keyboard.translate(
        "p",
        &[
            key(Key::ControlLeft, true, ctrl),
            key(Key::ShiftLeft, true, chord),
            key(Key::C, false, chord),
            key(Key::ShiftLeft, false, ctrl),
            key(Key::ControlLeft, false, Modifiers::NONE),
        ],
    );
    assert_eq!(actions.len(), 4, "physical modifiers still reach Rex");
    assert!(!actions.iter().any(clears_selection));

    let actions = keyboard.translate(
        "p",
        &[
            key(Key::C, true, Modifiers::NONE),
            Event::Text("c".into()),
            key(Key::C, false, Modifiers::NONE),
        ],
    );
    assert_eq!(key_of(&actions[0]), ("c", 0, true));
    assert!(clears_selection(&actions[0]));
    assert!(!clears_selection(&actions[1]), "a release never does");
}

#[test]
fn news_of_nothing_composed_is_kept_only_when_something_was() {
    let preedit = |text: &str| {
        let (text, active_range_chars) = (text.to_owned(), None);
        Event::Ime(ImeEvent::Preedit {
            text,
            active_range_chars,
        })
    };
    let commit = || Event::Ime(ImeEvent::Commit("に".into()));
    let mut keyboard = Keyboard::default();
    let mut settled = |mut events: Vec<Event>| {
        keyboard.settle(&mut events);
        events
    };
    // What Wayland says when told where the cursor is: nothing, over and over.
    assert_eq!(settled(vec![preedit("")]), []);
    assert_eq!(settled(vec![preedit(""), Event::Text("a".into())]).len(), 1);
    // A composition is heard through to the end of it, and no further.
    assert_eq!(settled(vec![preedit("n"), preedit("に")]).len(), 2);
    assert_eq!(settled(vec![preedit(""), commit()]).len(), 2);
    assert_eq!(settled(vec![preedit("")]), []);
}
