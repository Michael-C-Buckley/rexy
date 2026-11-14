use super::*;

/// A layout node as the server sends one: where it sits, and what is in it.
fn leaf(rect: [f32; 4], block: &str) -> Value {
    json!({"rect": rect, "block_id": block})
}

fn split(rect: [f32; 4], before: Value, after: Value) -> Value {
    json!({"rect": rect, "split": {"before": before, "after": after}})
}

fn windows(view: &Value) -> Vec<Window> {
    parse_windows(view, "me", &mut Vec::new())
}

#[test]
fn panes_sit_where_the_server_puts_them() {
    let right = split(
        [0.6, 0., 0.4, 1.],
        leaf([0.6, 0., 0.4, 0.25], "b"),
        leaf([0.6, 0.25, 0.4, 0.75], "c"),
    );
    let layout = split(FULL, leaf([0., 0., 0.6, 1.], "a"), right);
    let view = json!({"windows": [{"window_id": "w", "layers": [{"layout": layout}]}]});
    let window = &windows(&view)[0];
    let placed = |pane: &Pane| (pane.id.clone(), [pane.x, pane.y, pane.width, pane.height]);
    let panes: Vec<_> = window.panes.iter().map(placed).collect();
    let at = |id: &str, rect| (id.to_owned(), rect);
    assert_eq!(
        panes,
        [
            at("a", [0., 0., 0.6, 1.]),
            at("b", [0.6, 0., 0.4, 0.25]),
            at("c", [0.6, 0.25, 0.4, 0.75])
        ]
    );
}

#[test]
fn what_is_not_a_terminal_keeps_its_place_and_says_what_it_is() {
    let blocks = json!([
        {"block_id": "t", "creator_name": TERMINAL, "label": "shell"},
        {"block_id": "o", "creator_name": "com.superlogical.tunnel"},
    ]);
    let layout = split(
        FULL,
        leaf([0., 0., 0.5, 1.], "t"),
        leaf([0.5, 0., 0.5, 1.], "o"),
    );
    let layers = json!([{"layout": layout, "blocks": blocks}]);
    let view = json!({"windows": [{"window_id": "w", "layers": layers}]});
    let mut terminals = Vec::new();
    let tabs = parse_windows(&view, "me", &mut terminals);
    let [terminal, other] = &tabs[0].panes[..] else {
        panic!("a pane was left out");
    };
    assert_eq!(terminals, ["t"]);
    assert_eq!((terminal.title.as_str(), other.x), ("shell", 0.5));
    assert_eq!(other.title, "com.superlogical.tunnel");
    assert_eq!(other.status, "com.superlogical.tunnel: not a terminal");
}

#[test]
fn the_most_pressing_of_what_programs_report_speaks_for_their_terminal() {
    let none = (Activity::Idle, "", None);
    fn reported(records: &Value) -> (Activity, &str, Option<Program>) {
        super::reported(items(records))
    }
    let unheard = json!([{"state": "unheard-of"}]);
    assert_eq!(reported(&json!([])), none);
    assert_eq!(reported(&Value::Null), none);
    assert_eq!(reported(&unheard), none);
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
    assert_eq!(reported(&left), (Activity::Error, "Broke", None));
}

#[test]
fn zoom_and_minimized_layers_are_respected() {
    let view = json!({"windows": [{"window_id": "w", "zoomed_block_id": "b", "layers": [
        {
            "layout": leaf([0.2, 0.2, 0.5, 0.5], "b"),
            "blocks": [{"block_id": "b", "owner": "me"}],
        },
        {"state": "minimized", "layout": leaf(FULL, "hidden")},
    ]}]});
    let windows = windows(&view);
    assert_eq!(windows[0].panes.len(), 1);
    assert_eq!(windows[0].panes[0].width, 1.);
    assert!(windows[0].panes[0].owner);
    // The panes of a layer that was put away are kept track of, out of sight, as
    // are those behind a zoomed one.
    let ids = |panes: &[Pane]| panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&windows[0].hidden), ["hidden"]);
    let layout = split(
        FULL,
        leaf([0., 0., 0.5, 1.], "a"),
        leaf([0.5, 0., 0.5, 1.], "b"),
    );
    let tab = json!({"window_id": "w", "zoomed_block_id": "b", "layers": [{"layout": layout}]});
    let windows = self::windows(&json!({"windows": [tab]}));
    assert_eq!(ids(&windows[0].panes), ["b"]);
    assert_eq!(ids(&windows[0].hidden), ["a"]);
    assert!(windows[0].zoomed && windows[0].split());
}

#[test]
fn a_view_is_asked_for_only_when_a_newer_one_is_heard_of() {
    let mut held = Held::default();
    // With none held, one is asked for, and once only while it is on its way.
    assert!(held.want(0));
    assert!(!held.want(0) && !held.want(3));
    // The one that comes is older than the news that came meanwhile: ask again.
    assert!(held.take(2));
    held.asking = false;
    assert!(!held.current() && held.want(0));
    assert!(held.take(3));
    held.asking = false;
    // Held, so neither old news nor an answer naming it asks again. Newer does.
    assert!(held.current());
    assert!(!held.want(2) && !held.want(3));
    assert!(held.want(4));
    // Views come in any order, and an older one is not taken over a newer.
    assert!(held.take(5));
    assert!(!held.take(4) && !held.take(5));
    assert_eq!(held.revision, Some(5));
}

#[test]
fn the_list_of_sessions_follows_the_news_of_it() {
    let mut sessions = Vec::new();
    let mut told = |event: Event| {
        // Told twice, as when news of a change comes after an answer that had it.
        list(&mut sessions, &event);
        list(&mut sessions, &event);
        let ids = sessions.iter().map(|session| session.id.clone());
        ids.collect::<Vec<_>>().join(" ")
    };
    let created = |session: &str| Event::Created {
        session: session.into(),
        name: String::new(),
    };
    let moved = |session: &str, before: Option<&str>| Event::Moved {
        session: session.into(),
        before: before.map(str::to_owned),
    };
    told(created("a"));
    told(created("b"));
    assert_eq!(told(created("c")), "a b c");
    assert_eq!(told(moved("c", Some("a"))), "c a b");
    assert_eq!(told(moved("c", None)), "a b c");
    assert_eq!(told(moved("a", Some("c"))), "b a c");
    // News of what is not there changes nothing.
    assert_eq!(told(moved("gone", Some("b"))), "b a c");
    assert_eq!(told(moved("b", Some("gone"))), "a c b");
    let session = "c".to_owned();
    assert_eq!(told(Event::Destroyed { session }), "a b");
    assert_eq!(sessions[0].name, "Untitled");
}

#[test]
fn a_terminal_is_what_the_news_of_it_last_said() {
    let mut block = Block::default();
    block.note(News::Title("vim".into()));
    let (program, pid) = (Program::named(["claude"]), Some(7));
    block.note(News::Program { program, pid });
    assert_eq!((block.title.as_str(), block.foreground), ("vim", Some(7)));
    let report = |block: &mut Block, id: &str, record: Option<Value>| {
        let before = block.says().0;
        block.note(News::Status(vec![(id.into(), record)]));
        block.news(before)
    };
    // What a program comes to is news once, and its parts speak after it.
    let working = json!({"state": "working", "effective_app": "cargo"});
    assert_eq!(report(&mut block, "", Some(working.clone())), None);
    let asks = json!({"state": "blocked", "kind": "question", "msg": "Which?"});
    let asked = Some("Question: Which?".to_owned());
    assert_eq!(report(&mut block, "build/test", Some(asks.clone())), asked);
    assert_eq!(report(&mut block, "build/test", Some(asks)), None);
    assert_eq!(block.says().2, Program::named(["cargo"]));
    // A record replaces the one of its name, and one cleared says nothing more.
    let done = json!({"state": "done"});
    assert_eq!(report(&mut block, "", Some(done)), None);
    assert_eq!(report(&mut block, "build/test", None), Some("Done".into()));
    assert_eq!(report(&mut block, "", None), None);
    assert_eq!(block.says(), (Activity::Idle, "", None));
}

#[test]
fn a_pane_is_taken_only_from_an_owner_that_has_left_the_server() {
    let owned = json!([
        {"block_id": "mine", "owner": "client:me"},
        {"block_id": "theirs", "owner": "client:other"},
        {"block_id": "left", "owner": "client:gone"},
        {"block_id": "nobody's"},
    ]);
    let present = json!([{"client_id": "client:me"}, {"client_id": "client:other"}]);
    assert_eq!(abandoned(&owned, &present), ["left"]);
    // Nothing is taken on a guess, where the server would not say who is there.
    assert!(abandoned(&owned, &Value::Null).is_empty());
    assert!(abandoned(&Value::Null, &present).is_empty());
}

#[test]
fn what_keeps_failing_is_tried_again_less_often_or_not_at_all() {
    let lost = anyhow::Error::from(Ended::Dropped);
    let secs = |failures, near| retry(&lost, failures, near).map(|wait| wait.as_secs());
    // A server on this machine, and a terminal of a session joined, for ever.
    let near: Vec<_> = (0..5).map(|failures| secs(failures, true)).collect();
    assert_eq!(near, [2, 15, 15, 15, 15].map(Some));
    // Another machine three times, and then when the user says.
    let far: Vec<_> = (0..4).map(|failures| secs(failures, false)).collect();
    assert_eq!(far, [Some(2), Some(15), None, None]);
    // A server that this client cannot talk to is not asked again, however near,
    // nor a terminal that has closed.
    let refused = anyhow::Error::from(Ended::Incompatible("update the server".into()));
    assert_eq!(retry(&refused.context("Connecting"), 0, true), None);
    assert_eq!(retry(&Ended::BlockClosed.into(), 0, true), None);
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

fn client() -> (Client, mpsc::Receiver<(Action, u64)>) {
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
    assert!(matches!(queue.try_recv(), Ok((Action::Reconnect, 0))));
    // Asking for something to be shown is numbered, to tell its answer by.
    let made = queue.try_recv();
    assert!(matches!(made, Ok((Action::CreateSession(None), 1))));
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

#[test]
fn what_is_asked_to_be_shown_is_shown_at_once_and_numbered() {
    let (client, mut queue) = client();
    let tab = |id: &str| Window {
        id: id.into(),
        label: None,
        focus: None,
        zoomed: false,
        panes: Vec::new(),
        hidden: Vec::new(),
    };
    let session = |id: &str, active: &str| Session {
        id: id.into(),
        name: id.into(),
        active: Some(active.into()),
        windows: vec![tab("a"), tab("b")],
    };
    client.shared.change(|s| {
        s.view.connected = true;
        s.view.sessions = vec![session("s", "a"), session("t", "b")];
    });
    let shown = || {
        let view = client.snapshot();
        (view.selected_session, view.selected_window)
    };
    let both = |session: &str, tab: &str| (Some(session.to_owned()), Some(tab.to_owned()));
    let show = |session: &str, window: &str| Action::SelectWindow {
        session: session.into(),
        window: window.into(),
    };
    // A session comes with the tab the server shows of it.
    client.try_send(Action::SelectSession("s".into())).unwrap();
    assert_eq!(shown(), both("s", "a"));
    client.try_send(show("s", "b")).unwrap();
    assert_eq!(shown(), both("s", "b"));
    // A new tab is yet to be, and the one on show stays until it is.
    client.try_send(Action::CreateWindow("s".into())).unwrap();
    assert_eq!(shown(), both("s", "b"));
    client.try_send(Action::CreateWindow("t".into())).unwrap();
    assert_eq!(shown(), both("t", "b"));
    // What is not there is asked for all the same, and nothing is shown for it.
    client.try_send(show("gone", "a")).unwrap();
    assert_eq!(shown(), both("t", "b"));
    // Input asks for nothing to be shown, and has no number.
    let pane = "p".to_owned();
    client.try_send(Action::FocusPane { pane }).unwrap();
    client.leave();
    assert_eq!(shown(), (None, None));
    let numbers = std::iter::from_fn(|| queue.try_recv().ok()).map(|(_, pick)| pick);
    assert_eq!(numbers.collect::<Vec<_>>(), [1, 2, 3, 4, 5, 0, 6]);
}
