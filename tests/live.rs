//! Run only against a disposable server, never the default personal socket.
use rex_lite::{
    backend::Client,
    model::{Action, Direction, Size, View},
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
fn live_input_resize_splits_tabs_and_reconnect() {
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

    // Split the marker in the command so seeing the typed command cannot satisfy the assertion.
    let text = Action::Text {
        pane: pane.clone(),
        text: "printf 'rex-lite-%s\\n' verified".into(),
    };
    send(&client, text);
    enter(&client, &pane);
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
    wait(&client, "split layout and streams", |v| {
        v.windows
            .iter()
            .find(|w| w.id == window)
            .is_some_and(|w| w.panes.len() == 2 && w.panes.iter().all(|p| p.frame.is_some()))
    });
    send(&client, Action::CreateWindow);
    let next = wait(&client, "new tab", |v| {
        v.windows.len() == initial.windows.len() + 1
            && v.selected_window.as_ref() != Some(&window)
            && v.current_window()
                .is_some_and(|w| w.panes.iter().all(|p| p.frame.is_some()))
    });
    assert_eq!(next.selected_session.as_ref(), Some(&session));
    send(&client, Action::SelectWindow(window.clone()));
    wait(&client, "return to existing tab", |v| {
        v.selected_window.as_ref() == Some(&window) && screen(v, &pane).contains("paste-verified")
    });

    // A client for a server the UI is not showing lists sessions and selects none.
    let other = Client::spawn(vec![endpoint], true, Arc::new(|| {}));
    let listed = wait(&other, "sessions without a selection", |v| {
        v.connected && !v.sessions.is_empty()
    });
    assert!(listed.selected_session.is_none() && listed.windows.is_empty());
    send(&other, Action::SelectSession(session.clone()));
    wait(&other, "a session shown on request", |v| {
        v.panes().any(|p| p.frame.is_some())
    });
    other.leave();
    assert!(other.snapshot().windows.is_empty());
    // It stays left when the server next reports a change.
    send(&client, Action::SelectWindow(next.selected_window.unwrap()));
    send(&client, Action::SelectWindow(window.clone()));
    std::thread::sleep(Duration::from_millis(400));
    let left = other.snapshot();
    assert!(left.connected && left.selected_session.is_none() && left.windows.is_empty());
}
