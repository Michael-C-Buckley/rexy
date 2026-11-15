use super::*;
use crate::model::{Pane, Session};
use std::collections::HashSet;

fn pane(id: &str) -> Pane {
    Pane {
        id: id.into(),
        title: id.to_uppercase(),
        idle: id == "idle",
        ..Pane::default()
    }
}

fn tab(id: &str, panes: &[&str]) -> Window {
    Window {
        id: id.into(),
        panes: panes.iter().map(|id| pane(id)).collect(),
        ..Window::default()
    }
}

/// One session on show, with three tabs: the second on show, and split.
fn view() -> View {
    let session = |id: &str, windows| Session {
        id: id.into(),
        name: id.to_uppercase(),
        active: None,
        windows,
    };
    let tabs = vec![
        tab("t1", &["a"]),
        tab("t2", &["b", "idle", "c"]),
        tab("t3", &["d"]),
    ];
    View {
        connected: true,
        sessions: vec![session("s1", tabs), session("s2", Vec::new())],
        selected_session: Some("s1".into()),
        selected_window: Some("t2".into()),
        ..Default::default()
    }
}

/// What a command comes to with the keyboard in a pane, in words.
fn plan(view: &View, focused: &str, name: &str, args: Value) -> Result<String, &'static str> {
    let keys = Keys::default();
    let standing = Standing {
        view,
        focused: Some(focused),
        font_size: 15.0,
        sidebar: false,
        keys: &keys,
    };
    Ok(match standing.plan(name, &args, None)? {
        Deed::Ask(Action::SelectWindow { window, .. }) => format!("show {window}"),
        Deed::Ask(Action::SelectSession(session)) => format!("show {session}"),
        Deed::Ask(Action::Edit { edit, .. }) => format!("{edit:?}"),
        Deed::Ask(Action::Split { pane, direction }) => format!("split {pane} {direction:?}"),
        Deed::Ask(Action::Zoom { pane }) => format!("zoom {pane}"),
        Deed::Ask(Action::Key { code, .. }) => format!("press {code}"),
        Deed::Focus(pane) => format!("focus {pane}"),
        Deed::Close(_, spared, _) => format!("close, spared: {spared}"),
        Deed::CloseTab(_, tab, spared, _) => format!("close {tab}, spared: {spared}"),
        Deed::Rename(_, tab, name) => format!("rename {tab:?} from {name}"),
        Deed::Font(size) => format!("font {size}"),
        _ => "other".into(),
    })
}

#[test]
fn the_table_names_each_command_once_and_spells_its_keys_as_rex_does() {
    let names: HashSet<_> = COMMANDS.iter().map(|command| command.0).collect();
    assert_eq!(names.len(), COMMANDS.len());
    let mut seen = HashSet::new();
    for (key, name) in KEYS {
        assert!(seen.insert(key), "{key} is bound twice");
        assert!(
            names.contains(name),
            "{key} is bound to {name}, which is no command"
        );
        // What a keyboard cannot be heard to press would never be matched. A
        // plus is the one key whose name is also what joins a chord.
        let heard = *key == "ctrl++" || input::press("p", key).is_some();
        assert!(heard, "{key} is not a key as Rex spells it");
    }
    // A server is told of them all, and of the names that take a direction.
    let request = request();
    let count = |list: &Value| list.as_array().unwrap().len();
    assert_eq!(count(&request["actions"]), COMMANDS.len() + DIRECTED.len());
    assert_eq!(count(&request["defaults"][0]["bindings"]), KEYS.len());
}

#[test]
fn an_action_with_a_direction_goes_by_a_name_of_its_own() {
    let right = json!({"direction": "right"});
    assert_eq!(named("pane.split", &right), "pane.split.right");
    assert_eq!(
        named("client.tab.move", &json!({"direction": "forward"})),
        "client.tab.move.forward"
    );
    assert_eq!(named("pane.split.right", &right), "pane.split.right");
    assert_eq!(named("pane.zoom", &right), "pane.zoom");
    let view = view();
    let split = plan(&view, "b", "pane.split", right);
    assert_eq!(split.as_deref(), Ok("split b Right"));
    // One this client lacks is not among its actions.
    let left = json!({"direction": "left"});
    assert_eq!(plan(&view, "b", "pane.split", left), Err("not_supported"));
}

#[test]
fn a_command_is_planned_for_what_is_on_show() {
    let view = view();
    let plan = |name, args| plan(&view, "b", name, args);
    let none = Value::Null;
    assert_eq!(plan("client.settings", none.clone()), Err("not_supported"));
    assert_eq!(
        plan("client.font.increase", none.clone()).as_deref(),
        Ok("font 16")
    );
    // Tabs go round and round, and by number from either end.
    assert_eq!(
        plan("client.tab.next", none.clone()).as_deref(),
        Ok("show t3")
    );
    assert_eq!(
        plan("client.tab.previous", none.clone()).as_deref(),
        Ok("show t1")
    );
    let index = |index: i64| plan("client.tab.goto", json!({"index": index}));
    assert_eq!(index(1).as_deref(), Ok("show t1"));
    assert_eq!(index(-1).as_deref(), Ok("show t3"));
    assert_eq!(
        (index(0), index(4), index(-4)),
        (Err("refused"), Err("refused"), Err("refused"))
    );
    assert_eq!(plan("client.tab.goto", none.clone()), Err("refused"));
    // A tab moves to before the one before it, or to after the one after it.
    let moved = |name| plan(name, Value::Null);
    assert_eq!(
        moved("client.tab.move.backward").as_deref(),
        Ok("MoveBefore(Some(\"t1\"))")
    );
    assert_eq!(
        moved("client.tab.move.forward").as_deref(),
        Ok("MoveBefore(None)")
    );
    assert_eq!(plan("session.next", none.clone()).as_deref(), Ok("show s2"));
    let select = |id: &str| plan("session.select", json!({"session_id": id}));
    assert_eq!(
        (select("s2").as_deref(), select("s9")),
        (Ok("show s2"), Err("refused"))
    );
    // A session's name is there to be changed only in the sidebar.
    assert_eq!(plan("session.rename", none.clone()), Err("refused"));
    assert_eq!(
        plan("client.tab.rename", none.clone()).as_deref(),
        Ok("rename Some(\"t2\") from B")
    );
    assert_eq!(
        plan("client.tab.close", none.clone()).as_deref(),
        Ok("close t2, spared: false")
    );
    // Panes are those of the tab on show, round and round.
    assert_eq!(
        plan("pane.focus_next", none.clone()).as_deref(),
        Ok("focus idle")
    );
    assert_eq!(
        plan("pane.focus_previous", none.clone()).as_deref(),
        Ok("focus c")
    );
    assert_eq!(plan("pane.zoom", none.clone()).as_deref(), Ok("zoom b"));
    assert_eq!(
        plan("pane.close", none.clone()).as_deref(),
        Ok("close, spared: false")
    );
    assert_eq!(
        plan("pane.send_key", json!({"key": "ctrl+b"})).as_deref(),
        Ok("press key_b")
    );
    assert_eq!(plan("pane.send_key", none.clone()), Err("refused"));
    // No history is kept of a terminal that has yet to show anything.
    assert_eq!(plan("client.scroll.page_up", none.clone()), Err("refused"));
    assert_eq!(plan("client.mode.exit", none), Err("refused"));
}

#[test]
fn a_command_is_refused_where_it_has_nothing_to_act_on() {
    let mut view = view();
    // A pane another client names is one on show, or none.
    let keys = Keys::default();
    let standing = |view| Standing {
        view,
        focused: Some("b"),
        font_size: 15.0,
        sidebar: true,
        keys: &keys,
    };
    let named = |pane| standing(&view).plan("pane.close", &Value::Null, Some(pane));
    assert!(matches!(named("idle"), Ok(Deed::Close(_, true, _))));
    assert!(matches!(named("a"), Err("refused")));
    let rename = standing(&view).plan("session.rename", &Value::Null, None);
    assert!(matches!(rename, Ok(Deed::Rename(..))));
    // What is pasted goes to the pane named. Where nothing is selected there is
    // nothing to copy.
    let paste = standing(&view).plan("pane.paste", &Value::Null, Some("c"));
    assert!(matches!(paste, Ok(Deed::Paste(pane)) if pane == "c"));
    let copy = standing(&view).plan("pane.copy", &Value::Null, None);
    assert!(matches!(copy, Err("refused")));
    // A key bound to what cannot be done is still the window's, unless a
    // program is to have it: one that moves through history where there is none.
    let keeps = |name| standing(&view).keeps(name, &Value::Null);
    assert!(keeps("client.mode.exit") && keeps("client.tab.goto") && keeps("pane.zoom"));
    assert!(keeps("pane.copy"));
    assert!(!keeps("client.scroll.page_up") && !keeps("client.settings"));
    // A tab of one pane has nothing to zoom, and a first tab nowhere to move back to.
    view.selected_window = Some("t1".into());
    assert_eq!(plan(&view, "a", "pane.zoom", Value::Null), Err("refused"));
    assert_eq!(
        plan(&view, "a", "pane.focus_next", Value::Null),
        Err("refused")
    );
    assert_eq!(
        plan(&view, "a", "client.tab.move.backward", Value::Null),
        Err("refused")
    );
    assert_eq!(
        plan(&view, "b", "pane.split.right", Value::Null),
        Err("refused")
    );
    view.selected_window = Some("t3".into());
    assert_eq!(
        plan(&view, "d", "client.tab.move.forward", Value::Null),
        Err("refused")
    );
    // Without a connection there is the window itself to change, and the connection.
    view.connected = false;
    assert_eq!(
        plan(&view, "d", "client.tab.new", Value::Null),
        Err("refused")
    );
    assert_eq!(
        plan(&view, "d", "client.font.reset", Value::Null).as_deref(),
        Ok("font 15")
    );
    assert!(plan(&view, "d", "session.refresh", Value::Null).is_ok());
}
