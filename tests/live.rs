//! Run only against a disposable server, never the default personal socket.
use rex_lite::{
    backend::Client,
    model::{Action, Activity, Direction, Edit, Size, View, Window},
    terminal::Frame,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn wait(client: &Client, label: &str, ready: impl Fn(&View) -> bool) -> View {
    let start = Instant::now();
    loop {
        let view = client.snapshot();
        if ready(&view) {
            return view;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{label}: {} ({:?})",
            view.status,
            view.panes().map(|p| &p.status).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn frame<'a>(view: &'a View, pane: &str) -> Option<&'a Arc<Frame>> {
    view.pane(pane)?.frame.as_ref()
}

fn screen(view: &View, pane: &str) -> String {
    frame(view, pane)
        .map(|frame| frame.selected_text(0, usize::MAX))
        .unwrap_or_default()
}

fn find<'a>(view: &'a View, id: &str) -> Option<&'a Window> {
    view.windows().iter().find(|window| window.id == id)
}

fn send(client: &Client, action: Action) {
    client.try_send(action).unwrap();
}

fn enter(client: &Client, pane: &str) {
    for pressed in [true, false] {
        send(
            client,
            Action::Key {
                pane: pane.into(),
                key: "\n".into(),
                code: "enter".into(),
                modifiers: 0,
                pressed,
            },
        );
    }
}

#[test]
#[ignore = "requires REX_LITE_TEST_SERVER pointing to a disposable Rex server"]
fn live_input_resize_splits_tabs_sessions_and_reconnect() {
    let endpoint = std::env::var("REX_LITE_TEST_SERVER").expect("explicit test endpoint required");
    assert!(
        endpoint.starts_with("unix://")
            && endpoint.contains("rex-lite")
            && endpoint.contains("/work/"),
        "refusing to mutate a non-test Rex server"
    );
    let client = Client::spawn(vec![endpoint.clone()], false, Arc::new(|| {}));
    let initial = wait(&client, "initial Ghostty snapshot", |v| {
        v.connected && v.panes().any(|p| p.frame.is_some())
    });
    let session = initial.selected_session.clone().unwrap();
    let window = initial.selected_window.clone().unwrap();
    let pane = initial.current_window().unwrap().panes[0].id.clone();
    send(&client, Action::FocusPane { pane: pane.clone() });
    let size = Size {
        cols: 100,
        rows: 32,
        cell_width: 9,
        cell_height: 19,
    };
    let resize = Action::Resize {
        pane: pane.clone(),
        size,
    };
    send(&client, resize);
    wait(&client, "server resize metadata", |v| {
        frame(v, &pane).is_some_and(|f| f.cols == 100 && f.rows == 32)
    });

    // Type a command and run it.
    let run = |command: &str| {
        let (id, text) = (pane.clone(), command.into());
        send(&client, Action::Text { pane: id, text });
        enter(&client, &pane);
    };
    // Split the marker in the command so seeing the typed command cannot satisfy the assertion.
    run("printf 'rex-lite-%s\\n' verified");
    wait(&client, "keyboard input and live output", |v| {
        screen(v, &pane).contains("rex-lite-verified")
    });

    let paste = Action::Paste {
        pane: pane.clone(),
        text: "printf 'paste-%s\\n' verified".into(),
    };
    send(&client, paste);
    enter(&client, &pane);
    wait(&client, "paste and live output", |v| {
        screen(v, &pane).contains("paste-verified")
    });

    let old_frame = frame(&client.snapshot(), &pane).unwrap().clone();
    send(&client, Action::Reconnect);
    wait(&client, "fresh reconnect snapshot", |v| {
        v.connected
            && frame(v, &pane).is_some_and(|f| !Arc::ptr_eq(f, &old_frame))
            && screen(v, &pane).contains("rex-lite-verified")
    });
    let split = Action::Split {
        pane: pane.clone(),
        direction: Direction::Right,
    };
    send(&client, split);
    let split = wait(&client, "split layout and streams", |v| {
        find(v, &window)
            .is_some_and(|w| w.panes.len() == 2 && w.panes.iter().all(|p| p.frame.is_some()))
    });
    let mut others = find(&split, &window).unwrap().panes.iter();
    let other = others.find(|p| p.id != pane).unwrap().id.clone();
    // A pane fills its tab in place of the rest, and gives it back to them.
    for (label, zoomed, panes) in [("a pane zoomed", true, 1), ("a pane restored", false, 2)] {
        send(&client, Action::Zoom { pane: pane.clone() });
        wait(&client, label, |v| {
            let shown = |w: &Window| w.panes.len() == panes && w.panes[0].id == pane;
            find(v, &window).is_some_and(|w| w.zoomed == zoomed && shown(w))
        });
    }
    send(&client, Action::CreateWindow(session.clone()));
    let next = wait(&client, "new tab", |v| {
        v.windows().len() == initial.windows().len() + 1
            && v.selected_window.as_ref() != Some(&window)
            && v.current_window()
                .is_some_and(|w| w.panes.iter().all(|p| p.frame.is_some()))
    });
    assert_eq!(next.selected_session.as_ref(), Some(&session));
    let show = |window: &str| Action::SelectWindow {
        session: session.clone(),
        window: window.into(),
    };
    send(&client, show(&window));
    wait(&client, "return to existing tab", |v| {
        v.selected_window.as_ref() == Some(&window) && screen(v, &pane).contains("paste-verified")
    });

    // A tab goes by its name if it has one, and otherwise by its terminal's title.
    let tab = next.selected_window.clone().unwrap();
    let edit = |session: &str, window: Option<&str>, edit| Action::Edit {
        session: session.into(),
        window: window.map(str::to_owned),
        edit,
    };
    let rename = |name: &str| edit(&session, Some(&window), Edit::Rename(name.into()));
    send(&client, rename("Named"));
    wait(&client, "a tab renamed", |v| {
        find(v, &window).is_some_and(|w| w.name() == "Named")
    });
    send(&client, rename(""));
    // Of its panes, the one that typing goes to.
    send(&client, Action::FocusPane { pane: pane.clone() });
    run("printf '\\033]2;%s\\007' Titled");
    wait(&client, "a tab going by its terminal's title", |v| {
        find(v, &window).is_some_and(|w| w.name() == "Titled")
    });
    // Programs say what they are doing, and a tab shows the most pressing of it.
    for (state, activity) in [("blocked", Activity::Blocked), ("idle", Activity::Idle)] {
        run(&format!("printf '\\033]7501;state=%s\\007' {state}"));
        wait(&client, "a program's status", |v| {
            find(v, &window).is_some_and(|w| w.activity() == activity)
        });
    }
    // A tab has the icon of the program in its foreground, or of the one that
    // reports from it.
    let program = |label: &str, icon: &str| {
        wait(&client, label, |v| {
            find(v, &window).is_some_and(|w| w.program().name() == icon)
        });
    };
    program("a shell", "shell");
    run("sleep 1");
    program("a program in the foreground", "shell-sleep");
    program("the shell again", "shell");
    run("printf '\\033]7501;state=working:app=claude-code\\007'");
    program("an agent, by its report", "claude-code");
    run("printf '\\033]7501;state=clear\\007'");
    program("the shell once more", "shell");
    // What a program copies reaches the client, short or long.
    let copy = |text: &str| format!("printf '\\033]52;c;%s\\007' \"$({text} | base64 -w0)\"");
    run(&copy("printf copied-text"));
    wait(&client, "a program's copy", |v| {
        v.copied
            .as_ref()
            .is_some_and(|c| c.0 == pane && c.1 == "copied-text")
    });
    run(&copy("head -c 1500000 /dev/zero | tr '\\0' x"));
    wait(&client, "a long copy", |v| {
        let long = |text: &str| text.len() == 1_500_000 && text.bytes().all(|b| b == b'x');
        v.copied.as_ref().is_some_and(|c| c.0 == pane && long(&c.1))
    });
    // A pane closed leaves the tab to the others.
    send(&client, Action::ClosePane { pane: other });
    wait(&client, "a pane closed", |v| {
        find(v, &window).is_some_and(|w| !w.split() && w.panes[0].id == pane)
    });
    let order = |v: &View| -> Vec<String> { v.windows().iter().map(|w| w.id.clone()).collect() };
    let first = Edit::MoveBefore(Some(window.clone()));
    send(&client, edit(&session, Some(&tab), first));
    wait(&client, "a tab moved first", |v| {
        order(v).first() == Some(&tab)
    });
    send(&client, edit(&session, Some(&tab), Edit::MoveBefore(None)));
    wait(&client, "a tab moved last", |v| {
        order(v).last() == Some(&tab)
    });

    // A session made with a name is shown at once, and ending it shows another.
    send(&client, Action::CreateSession(Some("Second".into())));
    let second = wait(&client, "a named session", |v| {
        let shown = v.session().is_some_and(|s| s.name == "Second");
        shown && v.panes().any(|p| p.frame.is_some())
    });
    let second = second.selected_session.unwrap();
    // The tabs of the session left behind are still listed.
    let behind = client.snapshot();
    let behind = behind.sessions.iter().find(|s| s.id == session).unwrap();
    assert_eq!(behind.windows.len(), initial.windows().len() + 1);
    send(&client, edit(&second, None, Edit::Rename("Renamed".into())));
    wait(&client, "a session renamed", |v| {
        v.session().is_some_and(|s| s.name == "Renamed")
    });
    let first = Edit::MoveBefore(Some(session.clone()));
    send(&client, edit(&second, None, first));
    wait(&client, "a session moved first", |v| {
        v.sessions[0].id == second
    });
    // Its last tab closed, a session remains, empty.
    let only = client.snapshot().windows()[0].id.clone();
    send(&client, edit(&second, Some(&only), Edit::Close));
    wait(&client, "a session without tabs", |v| {
        v.selected_session.as_ref() == Some(&second) && v.windows().is_empty()
    });
    send(&client, edit(&second, None, Edit::Close));
    wait(&client, "a session ended", |v| {
        v.sessions.iter().all(|s| s.id != second) && v.selected_session.as_ref() == Some(&session)
    });
    // A tab of a session shown again is shown with it.
    send(&client, show(&tab));
    wait(&client, "a tab chosen", |v| {
        v.selected_window.as_ref() == Some(&tab)
    });
    send(&client, edit(&session, Some(&tab), Edit::Close));
    wait(&client, "a tab closed", |v| {
        v.windows().len() == initial.windows().len() && v.selected_window.as_ref() == Some(&window)
    });

    // A client for a server the UI is not showing lists sessions and selects none.
    let other = Client::spawn(vec![endpoint], true, Arc::new(|| {}));
    let listed = wait(&other, "sessions without a selection", |v| {
        v.connected && !v.sessions.is_empty()
    });
    assert!(listed.selected_session.is_none() && listed.windows().is_empty());
    // It lists their tabs all the same.
    assert!(listed.sessions.iter().any(|s| !s.windows.is_empty()));
    send(&other, Action::SelectSession(session.clone()));
    wait(&other, "a session shown on request", |v| {
        v.panes().any(|p| p.frame.is_some())
    });
    other.leave();
    assert!(other.snapshot().windows().is_empty());
    let stays_left = || {
        std::thread::sleep(Duration::from_millis(400));
        let left = other.snapshot();
        assert!(left.connected && left.selected_session.is_none() && left.windows().is_empty());
    };
    // It stays left when the server next reports a change.
    send(&client, Action::CreateWindow(session.clone()));
    send(&client, show(&window));
    stays_left();
    // And when it left before the server had done what it was last asked.
    send(&other, Action::SelectSession(session.clone()));
    other.leave();
    stays_left();
    send(&other, Action::CreateSession(Some("Third".into())));
    other.leave();
    let third = wait(&client, "a session made by a client that left", |v| {
        v.sessions.iter().any(|s| s.name == "Third")
    });
    stays_left();
    let third = third.sessions.iter().find(|s| s.name == "Third").unwrap();
    send(&client, edit(&third.id, None, Edit::Close));
    wait(&client, "that session ended", |v| {
        v.sessions.iter().all(|s| s.name != "Third")
    });
}
