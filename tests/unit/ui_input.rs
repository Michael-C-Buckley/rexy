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
fn clipboard_interception_preserves_terminal_control_keys() {
    let mut keyboard = Keyboard::default();
    let out = keyboard.translate(
        "p",
        &[
            Event::ModifiersChanged(Modifiers::CTRL),
            Event::Copy,
            key(Key::C, false, Modifiers::CTRL),
            Event::Paste("must not paste".into()),
            key(Key::V, false, Modifiers::CTRL),
            Event::ModifiersChanged(Modifiers::CTRL | Modifiers::SHIFT),
            Event::Paste("hello\n".into()),
        ],
    );
    assert_eq!(out.len(), 5);
    assert_eq!(key_of(&out[0]), ("c", model::CTRL, true));
    assert_eq!(key_of(&out[1]), ("c", model::CTRL, false));
    assert_eq!(key_of(&out[2]), ("v", model::CTRL, true));
    assert!(matches!(&out[4], Action::Paste { text, .. } if text == "hello\n"));
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
fn fast_control_c_is_not_removed_by_clipboard_filter() {
    let mut keyboard = Keyboard::default();
    let mut events = vec![
        Event::ModifiersChanged(Modifiers::CTRL),
        Event::Copy,
        key(Key::C, false, Modifiers::CTRL),
        Event::ModifiersChanged(Modifiers::NONE),
    ];
    assert!(!keyboard.take_copy(&mut events));
    let actions = keyboard.translate("p", &events);
    assert_eq!(actions.len(), 2);
    assert_eq!(key_of(&actions[0]), ("c", model::CTRL, true));
    assert_eq!(key_of(&actions[1]), ("c", model::CTRL, false));
}

#[test]
fn fast_shift_clipboard_chords_use_event_time_modifiers() {
    let mut keyboard = Keyboard::default();
    let chord = Modifiers::CTRL | Modifiers::SHIFT;
    let mut events = vec![
        Event::ModifiersChanged(chord),
        Event::Copy,
        key(Key::C, false, chord),
        Event::ModifiersChanged(Modifiers::NONE),
        Event::ModifiersChanged(chord),
        Event::Paste("pasted once".into()),
        key(Key::V, false, chord),
        Event::ModifiersChanged(Modifiers::NONE),
    ];
    assert!(keyboard.take_copy(&mut events));
    assert!(!events.iter().any(|event| matches!(event, Event::Copy)));
    let actions = keyboard.translate("p", &events);
    assert_eq!(actions.len(), 1);
    assert!(matches!(&actions[0], Action::Paste { text, .. } if text == "pasted once"));
}

#[test]
fn empty_clipboard_recovers_only_an_unambiguous_control_v() {
    let mut keyboard = Keyboard::default();
    let actions = keyboard.translate(
        "p",
        &[
            Event::ModifiersChanged(Modifiers::CTRL),
            key(Key::V, false, Modifiers::CTRL),
            Event::ModifiersChanged(Modifiers::NONE),
        ],
    );
    assert_eq!(actions.len(), 2);
    assert_eq!(key_of(&actions[0]), ("v", model::CTRL, true));
    assert_eq!(key_of(&actions[1]), ("v", model::CTRL, false));

    // Shift released before V must not turn a paste into terminal Ctrl+V.
    let actions = keyboard.translate(
        "p",
        &[
            Event::ModifiersChanged(Modifiers::CTRL | Modifiers::SHIFT),
            Event::Paste("text".into()),
            Event::ModifiersChanged(Modifiers::CTRL),
            key(Key::V, false, Modifiers::CTRL),
            Event::ModifiersChanged(Modifiers::NONE),
        ],
    );
    assert_eq!(actions.len(), 1);
    assert!(matches!(&actions[0], Action::Paste { text, .. } if text == "text"));

    let actions = keyboard.translate(
        "p",
        &[
            Event::ModifiersChanged(Modifiers::CTRL | Modifiers::SHIFT),
            Event::ModifiersChanged(Modifiers::CTRL),
            key(Key::V, false, Modifiers::CTRL),
            Event::ModifiersChanged(Modifiers::NONE),
        ],
    );
    assert!(
        actions.is_empty(),
        "an empty Shift+paste is not terminal input"
    );
}

#[test]
fn copy_modifiers_and_releases_keep_selection_for_local_escape() {
    let mut keyboard = Keyboard::default();
    let ctrl = Modifiers::CTRL;
    let chord = ctrl | Modifiers::SHIFT;
    let mut events = vec![
        key(Key::ControlLeft, true, ctrl),
        Event::ModifiersChanged(ctrl),
        key(Key::ShiftLeft, true, chord),
        Event::ModifiersChanged(chord),
        Event::Copy,
        key(Key::C, false, chord),
        key(Key::ShiftLeft, false, ctrl),
        Event::ModifiersChanged(ctrl),
        key(Key::ControlLeft, false, Modifiers::NONE),
        Event::ModifiersChanged(Modifiers::NONE),
    ];
    assert!(keyboard.take_copy(&mut events));
    let actions = keyboard.translate("p", &events);
    assert!(!actions.is_empty(), "physical modifiers still reach Rex");
    assert!(
        !actions.iter().any(clears_selection),
        "copy must preserve selection so Escape is consumed locally, not as a shell Meta prefix"
    );

    let actions = keyboard.translate(
        "p",
        &[
            key(Key::C, true, Modifiers::NONE),
            Event::Text("c".into()),
            key(Key::C, false, Modifiers::NONE),
        ],
    );
    assert!(actions.iter().any(clears_selection));
    assert_eq!(key_of(&actions[0]), ("c", 0, true));
    assert!(
        !clears_selection(&actions[1]),
        "a release never resumes a selection"
    );
}
