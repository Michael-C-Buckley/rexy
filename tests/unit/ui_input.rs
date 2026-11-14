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

/// Every key the server knows by name.
const SERVER_KEYS: &str = "unidentified backquote backslash bracket_left bracket_right comma \
    digit_0 digit_1 digit_2 digit_3 digit_4 digit_5 digit_6 digit_7 digit_8 digit_9 equal \
    intl_backslash intl_ro intl_yen key_a key_b key_c key_d key_e key_f key_g key_h key_i key_j \
    key_k key_l key_m key_n key_o key_p key_q key_r key_s key_t key_u key_v key_w key_x key_y \
    key_z minus period quote semicolon slash alt_left alt_right backspace caps_lock context_menu \
    control_left control_right enter meta_left meta_right shift_left shift_right space tab \
    convert kana_mode non_convert delete end help home insert page_down page_up arrow_down \
    arrow_left arrow_right arrow_up num_lock numpad_0 numpad_1 numpad_2 numpad_3 numpad_4 \
    numpad_5 numpad_6 numpad_7 numpad_8 numpad_9 numpad_add numpad_backspace numpad_clear \
    numpad_clear_entry numpad_comma numpad_decimal numpad_divide numpad_enter numpad_equal \
    numpad_memory_add numpad_memory_clear numpad_memory_recall numpad_memory_store \
    numpad_memory_subtract numpad_multiply numpad_paren_left numpad_paren_right numpad_subtract \
    numpad_separator numpad_up numpad_down numpad_right numpad_left numpad_begin numpad_home \
    numpad_end numpad_insert numpad_delete numpad_page_up numpad_page_down escape f1 f2 f3 f4 f5 \
    f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 f21 f22 f23 f24 f25 fn fn_lock \
    print_screen scroll_lock pause browser_back browser_favorites browser_forward browser_home \
    browser_refresh browser_search browser_stop eject launch_app_1 launch_app_2 launch_mail \
    media_play_pause media_select media_stop media_track_next media_track_previous power sleep \
    audio_volume_down audio_volume_mute audio_volume_up wake_up copy cut paste";

#[test]
fn every_key_is_named_as_the_server_names_it() {
    let known: HashSet<&str> = SERVER_KEYS.split_whitespace().collect();
    let named = Key::ALL
        .iter()
        .filter_map(|key| Some((key, key_code(*key)?)));
    for (key, code) in named.clone() {
        assert!(known.contains(code.as_str()), "{key:?} is sent as {code}");
    }
    assert!(named.count() > 80, "most keys have a name");
    assert_eq!(key_code(Key::SuperLeft).as_deref(), Some("meta_left"));
    assert_eq!(
        (key_code(Key::F25).as_deref(), key_code(Key::F26)),
        (Some("f25"), None)
    );
    assert!(model::is_modifier("meta_right") && !model::is_modifier("key_a"));
}

#[test]
fn super_is_held_from_its_press_to_its_release() {
    let mut keyboard = Keyboard::default();
    let none = Modifiers::NONE;
    let press = [key(Key::SuperLeft, true, none)];
    keyboard.watch(&press);
    assert_eq!(keyboard.bits(Modifiers::SHIFT), model::SHIFT | model::SUPER);
    // With it held a key types nothing, like one pressed with Ctrl.
    let out = keyboard.translate("p", &[key(Key::A, true, none), Event::Text("a".into())]);
    assert_eq!(out.len(), 1);
    assert_eq!(key_of(&out[0]), ("a", model::SUPER, true));
    keyboard.watch(&[key(Key::SuperLeft, false, none)]);
    assert_eq!(keyboard.bits(none), 0);
    // A window that loses the keyboard never hears of the release.
    keyboard.watch(&press);
    keyboard.watch(&[Event::WindowFocused(false)]);
    assert_eq!(keyboard.bits(none), 0);
}

#[test]
fn a_key_held_down_repeats() {
    let mut keyboard = Keyboard::default();
    let held = Event::Key {
        key: Key::ArrowUp,
        physical_key: Some(Key::ArrowUp),
        pressed: true,
        modifiers: Modifiers::NONE,
        repeat: true,
    };
    let events = [key(Key::ArrowUp, true, Modifiers::NONE), held];
    let out = keyboard.translate("p", &events);
    let repeats = |action: &Action| matches!(action, Action::Key { repeat: true, .. });
    assert_eq!(out.len(), 2);
    assert!(!repeats(&out[0]) && repeats(&out[1]));
    // However long it was held, it is let go once.
    let out = keyboard.translate("p", &[key(Key::ArrowUp, false, Modifiers::NONE)]);
    assert_eq!(out.len(), 1);
    assert!(!repeats(&out[0]));
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

#[test]
fn a_key_is_spelled_as_rex_spells_it() {
    let chord = model::CTRL | model::SHIFT;
    let spelled = |key, physical, modifiers| spellings(key, Some(physical), modifiers).unwrap();
    // By what it types and by its place, modifiers in Rex's order.
    let split = spelled(Key::D, Key::D, model::SHIFT | model::CTRL | model::ALT);
    assert_eq!(split, ["ctrl+alt+shift+d", "ctrl+alt+shift+[KeyD]"]);
    // A layout's own letter, wherever its key is.
    assert_eq!(spelled(Key::E, Key::D, model::CTRL)[0], "ctrl+e");
    // Shift is left written, and the symbol it types is not: a plus is the
    // equals key's, and an exclamation mark the 1's.
    assert_eq!(spelled(Key::Plus, Key::Equals, chord)[0], "ctrl+shift+=");
    assert_eq!(
        spelled(Key::Exclamationmark, Key::Num1, model::SHIFT)[0],
        "shift+1"
    );
    // A plus typed without Shift, as on a number pad, is itself.
    assert_eq!(spelled(Key::Plus, Key::Plus, model::CTRL)[0], "ctrl++");
    let named = [
        (Key::PageUp, "shift+pageup", "shift+[PageUp]"),
        (Key::ArrowLeft, "shift+left", "shift+[ArrowLeft]"),
        (Key::Tab, "shift+tab", "shift+[Tab]"),
        (Key::Enter, "shift+enter", "shift+[Enter]"),
        (Key::Space, "shift+space", "shift+[Space]"),
        (Key::F5, "shift+f5", "shift+[F5]"),
        (Key::Copy, "shift+copy", "shift+[Copy]"),
    ];
    for (key, name, place) in named {
        assert_eq!(spelled(key, key, model::SHIFT), [name, place]);
    }
    assert_eq!(spellings(Key::Escape, None, 0).unwrap()[0], "escape");
    assert_eq!(spelled(Key::K, Key::K, model::SUPER)[0], "cmd+k");
    // A modifier alone is no chord.
    assert_eq!(
        spellings(Key::ShiftLeft, Some(Key::ShiftLeft), model::SHIFT),
        None
    );
}

#[test]
fn a_key_rex_spells_is_pressed_as_itself() {
    let pressed = |chord| match press("p", chord) {
        Some(Action::Key {
            key,
            code,
            modifiers,
            pressed: true,
            ..
        }) => Some((key, code, modifiers)),
        _ => None,
    };
    let key = |key: &str, code: &str, modifiers| Some((key.to_owned(), code.to_owned(), modifiers));
    assert_eq!(pressed("ctrl+b"), key("b", "key_b", model::CTRL));
    assert_eq!(
        pressed("alt+shift+left"),
        key("", "arrow_left", model::ALT | model::SHIFT)
    );
    assert_eq!(pressed("[KeyB]"), key("b", "key_b", 0));
    assert_eq!(pressed("enter"), key("\n", "enter", 0));
    assert_eq!((pressed("hyper+b"), pressed("ctrl+nothing")), (None, None));
}

#[test]
fn a_key_taken_takes_what_it_typed() {
    let none = Modifiers::NONE;
    let text = |text: &str| Event::Text(text.into());
    let mut events = vec![
        key(Key::A, true, none),
        text("a"),
        key(Key::B, true, none),
        text("b"),
        key(Key::B, false, none),
        text("c"),
    ];
    remove(&mut events, &[2]);
    let left = [
        key(Key::A, true, none),
        text("a"),
        key(Key::B, false, none),
        text("c"),
    ];
    assert_eq!(events, left);
}

#[test]
fn the_key_that_ends_a_sequence_goes_with_the_keys_before_it() {
    let prefix = Some((Key::S, vec!["ctrl+x".into()]));
    let mut keyboard = Keyboard {
        prefix,
        ..Default::default()
    };
    // Only that key: one pressed ahead of it goes as it is.
    let events = [
        key(Key::ArrowUp, true, Modifiers::NONE),
        key(Key::S, true, Modifiers::CTRL),
    ];
    let out = keyboard.translate("p", &events);
    assert!(matches!(&out[0], Action::Key { .. }));
    let Action::After { prefix, key } = &out[1] else {
        panic!("expected a key after others, got {:?}", out[1]);
    };
    assert_eq!(
        (prefix.as_slice(), key_of(key).1),
        (&["ctrl+x".to_owned()][..], model::CTRL)
    );
    assert_eq!(keyboard.prefix, None);
}
