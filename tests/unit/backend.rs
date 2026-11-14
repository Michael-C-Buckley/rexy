use super::*;

#[test]
fn nested_split_geometry_respects_server_ratios() {
    let layout = json!({"split": {
        "direction": "horizontal",
        "ratio": 0.6,
        "before": {"block_id": "a"},
        "after": {"split": {
            "direction": "vertical",
            "ratio": 0.25,
            "before": {"block_id": "b"},
            "after": {"block_id": "c"},
        }},
    }});
    let view = json!({"windows": [{"window_id": "w", "layers": [{"layout": layout}]}]});
    let windows = parse_windows(&view, "me");
    let panes = &windows[0].panes;
    assert_eq!(panes.len(), 3);
    assert!((panes[1].x - 0.6).abs() < 0.001);
    assert!((panes[2].height - 0.75).abs() < 0.001);
    assert!((panes.iter().map(|p| p.width * p.height).sum::<f32>() - 1.).abs() < 0.001);
}

#[test]
fn the_most_pressing_of_what_programs_report_speaks_for_their_terminal() {
    let none = (Activity::Idle, String::new(), None);
    assert_eq!(reported(&json!([])), none);
    assert_eq!(reported(&Value::Null), none);
    assert_eq!(reported(&json!([{"state": "unheard-of"}])), none);
    let records = json!([
        {"state": "working", "effective_app": "claude", "msg": "Thinking"},
        {"state": "blocked", "kind": "permission", "msg": "Run it?"},
        {"state": "blocked", "kind": "permission", "msg": "And this?"},
        {"state": "done", "effective_app": "cargo"},
    ]);
    let (activity, message, app) = reported(&records);
    assert_eq!(activity, Activity::Blocked(Wait::Approval));
    // Of two as pressing, the first.
    assert_eq!(message, "Run it?");
    // One that has finished lends its icon no longer.
    assert_eq!(app, Program::named(["claude"]));
    let waits = [
        ("question", Wait::Question),
        ("auth", Wait::SignIn),
        ("unheard-of", Wait::Input),
    ];
    for (kind, wait) in waits {
        let records = json!([{"state": "blocked", "kind": kind}]);
        assert_eq!(reported(&records).0, Activity::Blocked(wait));
    }
    // What a program left behind is still to be looked at, failure before success.
    let left = json!([{"state": "done", "msg": "Built"}, {"state": "error", "msg": "Broke"}]);
    assert_eq!(reported(&left), (Activity::Error, "Broke".into(), None));
}

#[test]
fn zoom_and_minimized_layers_are_respected() {
    let view = json!({"windows": [{"window_id": "w", "zoomed_block_id": "b", "layers": [
        {
            "layout": {"block_id": "b"},
            "bounds": [0.2, 0.2, 0.5, 0.5],
            "blocks": [{"block_id": "b", "owner": "me"}],
        },
        {"state": "minimized", "layout": {"block_id": "hidden"}},
    ]}]});
    let windows = parse_windows(&view, "me");
    assert_eq!(windows[0].panes.len(), 1);
    assert_eq!(windows[0].panes[0].width, 1.);
    assert!(windows[0].panes[0].owner);
    // The panes of a layer that was put away are kept track of, out of sight, as
    // are those behind a zoomed one.
    let ids = |panes: &[Pane]| panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&windows[0].hidden), ["hidden"]);
    let layout = json!({"split": {
        "direction": "horizontal",
        "ratio": 0.5,
        "before": {"block_id": "a"},
        "after": {"block_id": "b"},
    }});
    let tab = json!({"window_id": "w", "zoomed_block_id": "b", "layers": [{"layout": layout}]});
    let windows = parse_windows(&json!({"windows": [tab]}), "me");
    assert_eq!(ids(&windows[0].panes), ["b"]);
    assert_eq!(ids(&windows[0].hidden), ["a"]);
    assert!(windows[0].zoomed && windows[0].split());
}

#[test]
fn control_and_release_keys_never_duplicate_text() {
    assert_eq!(key_payload("c", "key_c", model::CTRL, "press")["text"], "");
    assert_eq!(
        key_payload("A", "key_a", model::SHIFT, "press")["text"],
        "A"
    );
    assert_eq!(key_payload("a", "key_a", 0, "release")["text"], "");
    assert_eq!(key_payload("a", "key_a", 0, "repeat")["text"], "a");
    assert_eq!(key_payload("a", "key_a", 0, "repeat")["action"], "repeat");
}

#[test]
fn shift_is_used_up_only_in_typing_something_else() {
    let shift = model::SHIFT;
    let consumed = |key, code| key_payload(key, code, shift, "press")["consumed_mods"].clone();
    assert_eq!(consumed("A", "key_a"), "shift");
    assert_eq!(consumed("!", "digit_1"), "shift");
    // Or a program could not tell these from Tab, Enter and Space.
    assert_eq!(consumed("\t", "tab"), "");
    assert_eq!(consumed("\n", "enter"), "");
    assert_eq!(consumed(" ", "space"), "");
    assert_eq!(key_payload("\t", "tab", shift, "press")["mods"], "shift");
    assert_eq!(key_payload("a", "key_a", 0, "press")["consumed_mods"], "");
}

#[test]
fn a_key_is_known_by_what_it_types_without_shift() {
    let (shift, ctrl) = (model::SHIFT, model::CTRL);
    let unshifted = |key, code, bits| {
        let key = key_payload(key, code, bits, "press");
        key["unshifted_codepoint"]
            .as_u64()
            .and_then(|c| char::from_u32(c as u32))
    };
    assert_eq!(unshifted("n", "key_n", 0), Some('n'));
    assert_eq!(unshifted("N", "key_n", shift), Some('n'));
    assert_eq!(unshifted("!", "digit_1", shift), Some('1'));
    assert_eq!(unshifted("?", "slash", shift | ctrl), Some('/'));
    assert_eq!(unshifted("\t", "tab", shift), Some('\t'));
    // A letter follows the keyboard's layout, with Shift or without.
    assert_eq!(unshifted("a", "key_q", ctrl), Some('a'));
    assert_eq!(unshifted("É", "key_e", shift), Some('é'));
    assert_eq!(unshifted("é", "unidentified", 0), Some('é'));
    // A key that types nothing, and text that came from no one key.
    assert_eq!(unshifted("", "arrow_up", 0), Some('\0'));
    assert_eq!(unshifted("日本", "unidentified", 0), Some('\0'));
}

#[test]
fn notifications_coalesce_until_snapshot_acknowledges() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    let wake = Wake {
        pending: AtomicBool::new(false),
        callback: Arc::new(move || {
            seen.fetch_add(1, Ordering::Relaxed);
        }),
    };
    for _ in 0..100 {
        wake.notify();
    }
    assert_eq!(count.load(Ordering::Relaxed), 1);
    wake.pending.store(false, Ordering::Release);
    wake.notify();
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

fn client() -> (Client, mpsc::Receiver<Action>) {
    let (actions, queue) = mpsc::channel(256);
    let shared = Shared {
        state: Arc::default(),
        wake: Arc::new(Wake {
            pending: AtomicBool::new(false),
            callback: Arc::new(|| {}),
        }),
        herald: Herald::new().0,
    };
    let done = flume::bounded(0).1;
    let client = Client {
        actions,
        shared,
        done,
    };
    (client, queue)
}

#[test]
fn input_is_accepted_only_while_connected() {
    let (client, mut queue) = client();
    assert!(client.try_send(Action::CreateSession(None)).is_err());
    client.try_send(Action::Reconnect).unwrap();
    client.shared.status("Connected", true);
    client.try_send(Action::CreateSession(None)).unwrap();
    assert!(matches!(queue.try_recv(), Ok(Action::Reconnect)));
    assert!(matches!(queue.try_recv(), Ok(Action::CreateSession(None))));
    assert!(queue.try_recv().is_err());
}

#[test]
fn input_racing_a_disconnect_never_outlives_it() {
    let (client, mut queue) = client();
    let client = Arc::new(client);
    let stop = Arc::new(AtomicBool::new(false));
    let typing = {
        let (client, stop) = (client.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = client.try_send(Action::CreateSession(None));
            }
        })
    };
    for _ in 0..20_000 {
        client.shared.status("Connected", true);
        std::thread::yield_now();
        // What `run` does when a connection fails.
        client.shared.status("Disconnected", false);
        while queue.try_recv().is_ok() {}
        // Input accepted before the failure is gone, and none may follow it.
        std::thread::yield_now();
        assert!(queue.try_recv().is_err(), "input outlived its connection");
    }
    stop.store(true, Ordering::Relaxed);
    typing.join().unwrap();
}
