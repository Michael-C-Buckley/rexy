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
    assert_eq!(step(&map, &[], &["ctrl+b", "escape"]), Err(Step::Void));
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
    assert_eq!(step(&map, &["move"], &["escape"]), Err(Step::Leave));
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
fn a_key_held_down_does_again_only_what_is_the_better_for_it() {
    let mut keys = Keys::default();
    let next = keys
        .map
        .bindings
        .iter()
        .find(|bound| bound.name == "client.tab.next");
    let next = next.unwrap().clone();
    let again = |name: &str| keys.map.repeats(name).then(|| next.clone());
    // Holding the keys for the next tab goes on through the tabs. Holding those
    // for a new one makes one, and its repeats go nowhere.
    keys.held = Some(("ctrl+tab".into(), again("client.tab.next")));
    assert_eq!(keys.again("ctrl+tab"), Some(Some(&next)));
    // With Ctrl let go, the key is no longer the one that was taken.
    assert_eq!(keys.again("tab"), None);
    keys.held = Some(("ctrl+shift+t".into(), again("client.tab.new")));
    assert_eq!(keys.again("ctrl+shift+t"), Some(None));
}
