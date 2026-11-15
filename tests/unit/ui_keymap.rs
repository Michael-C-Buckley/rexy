use super::*;
use serde_json::json;

/// A server's answer, in the order to try its bindings.
fn map() -> Keymap {
    Keymap::parse(&json!({
        "bindings": [
            {"keys": ["ctrl+b", "c"], "action": "client.tab.new", "source": "init.lua:1"},
            {"keys": ["ctrl+b", "x"], "action": FORWARD},
            {"keys": ["ctrl+l"], "action": FORWARD},
            {"keys": ["ctrl+l"], "action": "pane.zoom"},
            {"keys": ["ctrl+m"], "action": "client.mac.only"},
            {"keys": ["ctrl+m"], "action": "pane.zoom"},
            {"keys": ["ctrl+[KeyT]"], "action": "client.tab.new"},
            {"keys": ["ctrl+d"], "action": "pane.split", "args": {"direction": "right"}},
            {"keys": ["ctrl+r"], "action": "dev_layout", "args": {"file": "app.log"}},
            {"keys": ["left"], "action": "pane.focus.left", "mode": "move"},
            {"keys": ["g", "g"], "action": "client.scroll.top", "mode": "move"},
            {"keys": ["ctrl+b"], "action": "pane.close", "source": "default"},
        ],
        "modes": [
            {"name": "move", "exclusive": false, "blocked": [["ctrl+d"]]},
            {"name": "only", "exclusive": true},
        ],
        "actions": [
            {"name": "dev_layout", "kind": "lua", "title": "Open Dev Layout", "repeats": true},
            {"name": "com.superlogical.terminal.write", "kind": "block"},
        ],
    }))
}

fn pressed(keys: &[&str]) -> Vec<Vec<String>> {
    keys.iter().map(|key| vec![(*key).to_owned()]).collect()
}

/// What a press comes to, by the name of what it runs.
fn step<'a>(map: &'a Keymap, modes: &[&str], keys: &[&str]) -> Result<&'a str, Step<'a>> {
    let modes: Vec<_> = modes
        .iter()
        .map(|mode| ((*mode).to_owned(), false))
        .collect();
    let can = |bound: &Binding| bound.name != "client.mac.only";
    match resolve(map, &modes, &pressed(keys), can) {
        Step::Run(bound) => Ok(&bound.name),
        step => Err(step),
    }
}

#[test]
fn an_answer_is_read_with_each_action_under_its_name_in_the_table() {
    let map = map();
    assert_eq!(map.bindings.len(), 12);
    let split = map.bindings.iter().find(|b| b.keys == ["ctrl+d"]).unwrap();
    assert_eq!(
        (split.name.as_str(), &split.args["direction"]),
        ("pane.split.right", &json!("right"))
    );
    assert_eq!(map.bindings[9].mode.as_deref(), Some("move"));
    assert!(map.has_mode("only") && !map.has_mode("resize"));
    assert_eq!(map.served.len(), 2);
    // The table says which of this client's actions a held key does again, and
    // the server which of its own.
    assert!(map.repeats("client.tab.next") && !map.repeats("client.tab.new"));
    assert!(map.repeats("dev_layout") && !map.repeats("com.superlogical.terminal.write"));
}

#[test]
fn a_key_does_the_first_thing_bound_to_it_that_can_be_done() {
    let map = map();
    assert_eq!(step(&map, &[], &["ctrl+m"]), Ok("pane.zoom"));
    // One that a function on the server answers hides what is beneath it.
    assert_eq!(step(&map, &[], &["ctrl+l"]), Ok(FORWARD));
    assert_eq!(step(&map, &[], &["ctrl+q"]), Err(Step::Pass));
    // By its place on the keyboard as by what it types.
    let place = [vec!["ctrl+e".to_owned(), "ctrl+[KeyT]".to_owned()]];
    assert_eq!(
        resolve(&map, &[], &place, |_| true),
        Step::Run(&map.bindings[6])
    );
}

#[test]
fn a_sequence_waits_for_its_keys_and_hides_the_key_it_starts_with() {
    let map = map();
    assert_eq!(step(&map, &[], &["ctrl+b"]), Err(Step::Wait));
    assert_eq!(step(&map, &[], &["ctrl+b", "c"]), Ok("client.tab.new"));
    assert_eq!(step(&map, &[], &["ctrl+b", "x"]), Ok(FORWARD));
    // A key that goes on no sequence is the terminal's, and Escape ends one.
    assert_eq!(step(&map, &[], &["ctrl+b", "q"]), Err(Step::Pass));
    assert_eq!(step(&map, &[], &["ctrl+b", "esc"]), Err(Step::Void));
    // With nothing the sequence could come to, its first key is free.
    let none = |bound: &Binding| bound.keys.len() == 1;
    assert_eq!(
        resolve(&map, &[], &pressed(&["ctrl+b"]), none),
        Step::Run(&map.bindings[11])
    );
}

#[test]
fn a_mode_has_its_own_keys_and_leaves_the_rest_unless_it_says_otherwise() {
    let map = map();
    assert_eq!(step(&map, &[], &["left"]), Err(Step::Pass));
    assert_eq!(step(&map, &["move"], &["left"]), Ok("pane.focus.left"));
    assert_eq!(step(&map, &["move"], &["g"]), Err(Step::Wait));
    assert_eq!(step(&map, &["move"], &["g", "g"]), Ok("client.scroll.top"));
    // What it does not bind is as it was, but for a key it blocks.
    assert_eq!(step(&map, &["move"], &["ctrl+m"]), Ok("pane.zoom"));
    assert_eq!(step(&map, &["move"], &["ctrl+d"]), Err(Step::Void));
    assert_eq!(step(&map, &["move"], &["esc"]), Err(Step::Leave));
    // One that is all there is ignores every other key. The last entered is asked first.
    assert_eq!(step(&map, &["move", "only"], &["left"]), Err(Step::Void));
    assert_eq!(
        step(&map, &["only", "move"], &["left"]),
        Ok("pane.focus.left")
    );
    assert_eq!(step(&map, &["only", "move"], &["ctrl+m"]), Err(Step::Void));
}

#[test]
fn keys_are_shown_as_they_are_read_on_a_keyboard() {
    let map = map();
    assert_eq!(map.shortcut("client.tab.new"), "Ctrl+B, C");
    // Not a key that is sent on to the server before it comes to this.
    assert_eq!(map.shortcut("pane.zoom"), "Ctrl+M");
    assert_eq!(map.tip("Split", "pane.split.right"), "Split (Ctrl+D)");
    // Those of a mode are not its keys the rest of the time.
    assert_eq!(map.tip("Top", "client.scroll.top"), "Top");
    let own = Keymap::own();
    // Of several, the last in the table.
    assert_eq!(own.shortcut("client.font.increase"), "Ctrl+=");
    assert_eq!(own.shortcut("client.tab.previous"), "Ctrl+Shift+Tab");
    assert_eq!(own.shortcut("client.scroll.page_up"), "Shift+PageUp");
    assert_eq!(own.shortcut("client.tab.goto"), "");
}

#[test]
fn modes_and_a_sequence_end_with_the_keymap_they_began_in() {
    let mut keys = Keys::default();
    keys.modes.push(("move".into(), false));
    keys.pending = pressed(&["ctrl+b"]);
    keys.follow(None);
    assert_eq!((keys.modes.len(), keys.pending.len()), (1, 1));
    let reply = Arc::new(json!({"bindings": []}));
    keys.follow(Some(&reply));
    assert!(keys.modes.is_empty() && keys.pending.is_empty() && keys.map.bindings.is_empty());
    keys.modes.push(("move".into(), true));
    keys.follow(Some(&reply));
    assert_eq!(keys.modes.len(), 1);
    keys.follow(None);
    assert!(keys.modes.is_empty() && !keys.map.bindings.is_empty());
}

#[test]
fn what_can_be_pressed_next_is_said_with_what_it_does() {
    let map = map();
    let next = |modes: &[&str], keys: &[&str]| {
        let modes = modes.iter().map(|mode| ((*mode).to_owned(), false));
        let modes: Vec<_> = modes.collect();
        let next = map.next(&modes, &pressed(keys));
        let said = |(key, title): &(&str, String)| format!("{key}: {title}");
        next.iter().map(said).collect::<Vec<_>>()
    };
    // After the first key of a sequence, the keys that go on from it.
    assert_eq!(
        next(&[], &["ctrl+b"]),
        ["c: New Tab", "x: Send to Terminal"]
    );
    // Nothing goes on from a key that is whole.
    assert!(next(&[], &["ctrl+l"]).is_empty());
    // In a mode, what the mode binds, and that more keys follow one of them.
    let moving = ["left: Focus Pane Left", "g: Scroll to Top…"];
    assert_eq!(next(&["move"], &[]), moving);
    assert_eq!(next(&["move"], &["g"]), ["g: Scroll to Top"]);
    // A sequence begun in a mode goes on as it would outside it.
    assert_eq!(next(&["move"], &["ctrl+b"]).len(), 2);
    assert_eq!(shown("ctrl+shift+pageup"), "Ctrl+Shift+PageUp");
    // A server's own action goes by the title the server gives it.
    let bound = map.bindings.iter().find(|bound| bound.name == "dev_layout");
    assert_eq!(map.title(bound.unwrap()), "Open Dev Layout");
}
