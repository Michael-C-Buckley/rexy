//! Run only against a disposable server, never the default personal socket.
use rex_lite::{
    backend::Client,
    model::{self, Action, Activity, Direction, Edit, Size, View, Wait, Window},
    notify::Herald,
    terminal::Frame,
};
use serde_json::{Value, json};
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
                repeat: false,
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
    let (herald, notes) = Herald::new();
    let client = Client::spawn(vec![endpoint.clone()], Arc::new(|| {}), herald);
    // A connection comes with every session's tabs, and with no session shown
    // unasked. A window asks for the first, of the server it shows.
    let listed = wait(&client, "a connection", |v| v.connected);
    assert!(listed.sessions.iter().all(|s| !s.windows.is_empty()));
    assert!(listed.selected_session.is_none() && listed.windows().is_empty());
    send(&client, listed.unshown().unwrap());
    let initial = wait(&client, "initial Ghostty snapshot", |v| {
        v.panes().any(|p| p.frame.is_some())
    });
    let session = initial.selected_session.clone().unwrap();
    let window = initial.selected_window.clone().unwrap();
    let pane = initial.current_window().unwrap().panes[0].id.clone();
    // The pane was made by a client that has left the server. Shown here, it
    // becomes this client's without being touched.
    let owned = |v: &View| v.pane(&pane).is_some_and(|p| p.owner);
    wait(&client, "a pane whose owner has gone", owned);
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

    // A program that asks is told where the mouse moves and of each step of the
    // wheel, and can tell Shift+Tab and Shift+Enter from Tab and Enter. It reads
    // one line, which the terminal shows as it comes.
    let tracking = |v: &View| frame(v, &pane).is_some_and(|f| f.mouse_tracking);
    run(
        "printf '\\033[?1003h\\033[?1006h\\033[>1u'; head -n 1 | cat -v; \
        printf '\\033[<u\\033[?1003l\\033[?1006l'",
    );
    wait(&client, "a program tracking the mouse", tracking);
    let (id, x, y) = (pane.clone(), 3, 4);
    let motion = Action::Mouse {
        pane: pane.clone(),
        kind: "motion",
        x,
        y,
        button: None,
        modifiers: 0,
    };
    send(&client, motion);
    for _ in 0..3 {
        let (pane, lines) = (id.clone(), -1);
        send(&client, Action::Wheel { pane, x, y, lines });
    }
    for (key, code) in [("\t", "tab"), ("\n", "enter")] {
        let key = Action::Key {
            pane: pane.clone(),
            key: key.into(),
            code: code.into(),
            modifiers: model::SHIFT,
            pressed: true,
            repeat: false,
        };
        send(&client, key);
    }
    wait(&client, "the mouse and shifted keys reported", |v| {
        let screen = screen(v, &pane);
        let wheel = screen.matches("[<64;4;5M").count();
        let keys = screen.contains("[9;2u") && screen.contains("[13;2u");
        screen.contains("[<35;4;5M") && wheel == 3 && keys
    });
    enter(&client, &pane);
    wait(&client, "the mouse let go", |v| !tracking(v));

    let old_frame = frame(&client.snapshot(), &pane).unwrap().clone();
    send(&client, Action::Reconnect);
    // The server knows the client again, with what it owned: nothing else here
    // would give the pane back to it untouched while the server still held its place.
    wait(&client, "fresh reconnect snapshot", |v| {
        v.connected
            && owned(v)
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
    // The border between the two moves where it is asked to, and the server gives
    // each side its share. Asked faster than the server answers, as a drag asks,
    // it comes to rest where it was last asked to go.
    let [border] = &find(&split, &window).unwrap().borders[..] else {
        panic!("a split of two has one border");
    };
    let sides = (&border.before, &border.after, border.upright);
    assert_eq!(sides, (&pane, &other, true));
    let divide = |ratio: f32| Action::Divide {
        before: pane.clone(),
        after: other.clone(),
        upright: true,
        ratio,
    };
    let divided = |ratio: f32| {
        wait(&client, "a border moved", |v| {
            let near = |at: f32| (at - ratio).abs() < 0.01;
            find(v, &window).is_some_and(|w| near(w.borders[0].at) && near(w.panes[0].width))
        });
    };
    send(&client, divide(0.3));
    divided(0.3);
    for step in 0..=20 {
        send(&client, divide(0.5 + 0.01 * step as f32));
    }
    divided(0.7);
    send(&client, divide(0.5));
    divided(0.5);
    // The server is told some things from outside: this window types only into
    // what it shows, and makes no floating layers.
    let rex = |args: &[&str]| {
        let rex = std::process::Command::new("rex")
            .args(["-S", &endpoint, "--autostart=false"])
            .args(args)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&rex.stderr);
        assert!(rex.status.success(), "{error}");
        String::from_utf8(rex.stdout).unwrap()
    };
    // A pane fills its tab in place of the rest, and gives it back to them.
    for (label, zoomed, panes) in [("a pane zoomed", true, 1), ("a pane restored", false, 2)] {
        send(&client, Action::Zoom { pane: pane.clone() });
        wait(&client, label, |v| {
            // One pane alone has no border to drag.
            let shown = |w: &Window| {
                w.panes.len() == panes && w.panes[0].id == pane && w.borders.len() == panes - 1
            };
            find(v, &window).is_some_and(|w| w.zoomed == zoomed && shown(w))
        });
        if !zoomed {
            continue;
        }
        // The pane behind it runs on, and what it has to say is still heard.
        let report = "printf '\\033]7501;state=done\\007'\n";
        rex(&["send", "-s", &session, "-b", &other, report]);
        wait(&client, "a pane out of sight heard", |v| {
            let behind = find(v, &window).and_then(|w| w.hidden.first());
            behind.is_some_and(|p| p.id == other && p.activity == Activity::Done)
        });
        let note = notes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            (note.pane.as_str(), note.body.as_str()),
            (other.as_str(), "Done")
        );
    }
    // A pane of a floating layer that has been put away runs on out of sight too,
    // and comes back, with the keyboard, when it is revealed.
    let call = |method: &str, payload: Value| -> Value {
        let reply = rex(&["api", "call", method, &payload.to_string()]);
        serde_json::from_str(&reply).unwrap()
    };
    let layout = json!({"block": {"flavor": "com.superlogical.terminal.shell"}});
    let bounds = [0.2, 0.2, 0.5, 0.5];
    let layer =
        json!({"session_id": session, "window_id": window, "bounds": bounds, "layout": layout});
    let layer = call("session.new_layer", layer);
    let floating = layer["block_ids"][0].as_str().unwrap().to_owned();
    let away = json!({"session_id": session, "layer_id": layer["layer_id"], "state": "minimized"});
    call("session.set_layer_state", away);
    let tab = |v: &View, shown: usize, hidden: &[&str]| {
        let hides = |w: &Window| {
            w.hidden
                .iter()
                .map(|p| p.id.as_str())
                .eq(hidden.iter().copied())
        };
        find(v, &window).is_some_and(|w| w.panes.len() == shown && hides(w))
    };
    wait(&client, "a pane put away", |v| tab(v, 2, &[&floating]));
    let reveal = Action::Reveal {
        pane: floating.clone(),
    };
    send(&client, reveal);
    // Nor is there one to drag under a pane that floats over it.
    let borders = |v: &View| find(v, &window).unwrap().borders.len();
    let back = wait(&client, "a pane brought back", |v| {
        v.made.as_ref() == Some(&floating) && tab(v, 3, &[])
    });
    assert_eq!(borders(&back), 0);
    send(&client, Action::ClosePane { pane: floating });
    let closed = wait(&client, "a floating pane closed", |v| tab(v, 2, &[]));
    assert_eq!(borders(&closed), 1);
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
    // Programs say what they are doing, and what they wait for or end with is
    // passed on, for whoever is not looking. The message is "Run it?" in base64.
    let said = |report: &str, activity, note: Option<&str>| {
        run(&format!("printf '\\033]7501;{report}\\007'"));
        wait(&client, "a program's status", |v| {
            v.pane(&pane).is_some_and(|p| p.activity == activity)
        });
        let heard = notes.recv_timeout(Duration::from_millis(300)).ok();
        assert_eq!(heard.as_ref().map(|note| note.body.as_str()), note);
        assert!(
            heard.is_none_or(|note| note.tab == window && note.summary.starts_with("Titled — "))
        );
    };
    let asked = Activity::Blocked(Wait::Approval);
    let ask = "state=blocked:kind=permission:msg=UnVuIGl0Pw";
    said(ask, asked, Some("Needs approval: Run it?"));
    assert_eq!(client.snapshot().pane(&pane).unwrap().message, "Run it?");
    said("state=working", Activity::Working, None);
    // What a program comes to while the connection is lost is news when it is back.
    run("sleep 1; printf '\\033]7501;state=%s\\007' error");
    wait(&client, "a report to come", |v| {
        screen(v, &pane).contains("sleep 1")
    });
    let old_frame = frame(&client.snapshot(), &pane).unwrap().clone();
    send(&client, Action::Reconnect);
    wait(&client, "a status that changed out of hearing", |v| {
        let fresh = frame(v, &pane).is_some_and(|f| !Arc::ptr_eq(f, &old_frame));
        v.connected && fresh && v.pane(&pane).is_some_and(|p| p.activity == Activity::Error)
    });
    let late = notes.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(late.body, "Error");
    said("state=done", Activity::Done, Some("Done"));
    // Running something else does not take back what a program ended with.
    run("true");
    said("state=error", Activity::Error, Some("Error"));
    said("state=idle", Activity::Idle, None);
    // A bell is news to the window as well, and a notice is for the desktop alone.
    run("printf '\\007'");
    wait(&client, "a bell", |v| v.rang.contains(&pane));
    let bell = notes.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(bell.body, "Bell");
    run("printf '\\033]777;notify;Build;All green\\007'");
    let notice = notes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(notice.body, "Build: All green");
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
    // A window that opens is shown the server's first session, and asks at once
    // to be where it last was instead. That is where it comes to be.
    let again = Client::spawn(vec![endpoint.clone()], Arc::new(|| {}), Herald::new().0);
    let opened = wait(&again, "a window opened again", |v| v.connected);
    send(&again, opened.unshown().unwrap());
    send(&again, show(&window));
    wait(&again, "a window back where it was", |v| {
        v.selected_window.as_ref() == Some(&window) && v.panes().any(|p| p.frame.is_some())
    });
    drop(again);
    // Its last tab closed, a session remains, empty.
    let only = client.snapshot().windows()[0].id.clone();
    send(&client, edit(&second, Some(&only), Edit::Close));
    wait(&client, "a session without tabs", |v| {
        v.selected_session.as_ref() == Some(&second) && v.windows().is_empty()
    });
    send(&client, edit(&second, None, Edit::Close));
    let ended = wait(&client, "a session ended", |v| {
        v.sessions.iter().all(|s| s.id != second) && v.selected_session.is_none()
    });
    send(&client, ended.unshown().unwrap());
    wait(&client, "the first session shown in its place", |v| {
        v.selected_session.as_ref() == Some(&session) && v.panes().any(|p| p.frame.is_some())
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

    // A second window on the same session takes nothing from the first by showing it.
    let other = Client::spawn(vec![endpoint.clone()], Arc::new(|| {}), Herald::new().0);
    wait(&other, "another connection", |v| v.connected);
    send(&other, Action::SelectSession(session.clone()));
    wait(&other, "a session shown on request", |v| {
        v.panes().any(|p| p.frame.is_some())
    });
    // Each goes by this machine's name, which tells it from one on another.
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
    let clients = || {
        let listed = call("session.list_clients", json!({"session_id": session}));
        let clients = listed["clients"].as_array().unwrap().iter();
        let ours = clients.filter(|c| c["info"]["name"] == "Rex Lite");
        ours.filter(|c| c["info"]["label"] == hostname.trim())
            .count()
    };
    assert_eq!(clients(), 2);
    std::thread::sleep(Duration::from_millis(300));
    assert!(owned(&client.snapshot()) && !owned(&other.snapshot()));
    // What it takes by using it comes back when it goes.
    send(&other, Action::FocusPane { pane: pane.clone() });
    wait(&other, "a pane taken", owned);
    wait(&client, "a pane lost to another window", |v| !owned(v));
    // A connection made again finds the client still in the session it had joined,
    // and leaves it if that is no longer on show.
    send(&other, Action::Reconnect);
    other.leave();
    wait(&client, "a pane given back", owned);
    wait(&other, "another window connected again", |v| v.connected);
    assert_eq!(clients(), 1);
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
    // The probe shows the first session as a window would, and sees its terminal.
    let probed = rex_lite::backend::probe(endpoint, Duration::from_secs(12)).unwrap();
    assert!(probed.contains("1 sessions") && !probed.contains(" 0 decoded"));
}
