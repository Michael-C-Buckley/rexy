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
        .map(|frame| frame.text())
        .unwrap_or_default()
}

fn find<'a>(view: &'a View, id: &str) -> Option<&'a Window> {
    view.windows().iter().find(|window| window.id == id)
}

fn send(client: &Client, action: Action) {
    client.try_send(action).unwrap();
}

/// A key going down in a pane or coming up, alone.
fn key(pane: &str, key: &str, code: &str, modifiers: u8, pressed: bool) -> Action {
    Action::Key {
        pane: pane.into(),
        key: key.into(),
        code: code.into(),
        modifiers,
        pressed,
        repeat: false,
        prefix: Vec::new(),
    }
}

fn enter(client: &Client, pane: &str) {
    for pressed in [true, false] {
        send(client, key(pane, "\n", "enter", 0, pressed));
    }
}

/// A client of the server, with the keys a window has.
fn open(endpoint: &str, herald: Herald) -> Client {
    let keymap = Some(rex_lite::ui::commands::request());
    Client::spawn(vec![endpoint.to_owned()], Arc::new(|| {}), herald, keymap)
}

/// A terminal has been drawn.
fn drawn(view: &View) -> bool {
    view.panes().any(|pane| pane.frame.is_some())
}

/// The server to test against, which must be one made for it.
fn endpoint() -> String {
    let endpoint = std::env::var("REX_LITE_TEST_SERVER").expect("explicit test endpoint required");
    assert!(
        endpoint.starts_with("unix://")
            && endpoint.contains("rex-lite")
            && endpoint.contains("/work/"),
        "refusing to mutate a non-test Rex server"
    );
    endpoint
}

/// Run the `rex` command against that server and no other: what it printed, or
/// what it said had gone wrong.
fn rex(endpoint: &str, args: &[&str]) -> Result<String, String> {
    let mut rex = std::process::Command::new("rex");
    let rex = rex.args(["-S", endpoint, "--autostart=false"]).args(args);
    let rex = rex.output().unwrap();
    match rex.status.success() {
        true => Ok(String::from_utf8(rex.stdout).unwrap()),
        false => Err(String::from_utf8_lossy(&rex.stderr).into_owned()),
    }
}

/// Ask that server something as the command line does.
fn call(endpoint: &str, method: &str, payload: Value) -> Result<Value, String> {
    let reply = rex(endpoint, &["api", "call", method, &payload.to_string()])?;
    Ok(serde_json::from_str(&reply).unwrap())
}

#[test]
#[ignore = "requires REX_LITE_TEST_SERVER pointing to a disposable Rex server"]
fn live_input_resize_splits_tabs_sessions_and_reconnect() {
    let endpoint = endpoint();
    let (herald, notes) = Herald::new();
    let client = open(&endpoint, herald);
    // A connection comes with every session's tabs, and with no session shown
    // unasked. A window asks for the first, of the server it shows.
    let listed = wait(&client, "a connection", |v| v.connected);
    assert!(listed.sessions.iter().all(|s| !s.windows.is_empty()));
    assert!(listed.selected_session.is_none() && listed.windows().is_empty());
    send(&client, listed.unshown().unwrap());
    let initial = wait(&client, "initial Ghostty snapshot", drawn);
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
    for (text, code) in [("\t", "tab"), ("\n", "enter")] {
        send(&client, key(&pane, text, code, model::SHIFT, true));
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
    let divide = |ratio: f32| Action::Divide(vec![border.moved(ratio)]);
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
    let rex = |args: &[&str]| rex(&endpoint, args).unwrap();
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
    let call = |method: &str, payload: Value| call(&endpoint, method, payload).unwrap();
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

    // A tab shown before is kept fed while it is out of sight. Going back to
    // one finds it drawn, with nothing to wait for.
    let tab = next.selected_window.clone().unwrap();
    send(&client, show(&tab));
    assert!(client.snapshot().panes().all(|pane| pane.frame.is_some()));
    send(&client, show(&window));
    assert!(screen(&client.snapshot(), &pane).contains("paste-verified"));

    // A tab goes by its name if it has one, and otherwise by its terminal's title.
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
        shown && drawn(v)
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
    let again = open(&endpoint, Herald::new().0);
    let opened = wait(&again, "a window opened again", |v| v.connected);
    send(&again, opened.unshown().unwrap());
    send(&again, show(&window));
    wait(&again, "a window back where it was", |v| {
        v.selected_window.as_ref() == Some(&window) && drawn(v)
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
        v.selected_session.as_ref() == Some(&session) && drawn(v)
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
    let other = open(&endpoint, Herald::new().0);
    wait(&other, "another connection", |v| v.connected);
    send(&other, Action::SelectSession(session.clone()));
    wait(&other, "a session shown on request", drawn);
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

/// The keys follow the server's keymap, the server's own actions can be done, and
/// what other clients ask of this one reaches it and is answered.
#[test]
#[ignore = "requires REX_LITE_TEST_SERVER pointing to a disposable Rex server"]
fn live_keymap_actions_and_requests() {
    let endpoint = endpoint();
    let call = |method: &str, payload: Value| call(&endpoint, method, payload);
    let client = open(&endpoint, Herald::new().0);
    let keymap = |label: &str, ready: &dyn Fn(&Value) -> bool| {
        let view = wait(&client, label, |v| v.keymap.as_deref().is_some_and(ready));
        view.keymap.unwrap()
    };
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let bound = |map: &Value, keys: &Value, action: &str| {
        let is = |b: &Value| &b["keys"] == keys && b["action"] == action;
        list(&map["bindings"]).iter().any(is)
    };
    // With nothing in the configuration, this client's own keys come back as
    // they were sent: keys are matched by how they are spelled.
    let first = keymap("the server's keymap", &|_| true);
    let asked = rex_lite::ui::commands::request();
    let own = list(&asked["defaults"][0]["bindings"]);
    assert!(own.len() > 30 && list(&first["diagnostics"]).is_empty());
    for binding in &own {
        let action = binding["action"].as_str().unwrap();
        assert!(bound(&first, &binding["keys"], action), "{binding}");
    }
    // Written another way, each comes back in this spelling: the server reads
    // them all, but for the keys that copy and paste, which it keeps as they came.
    let mut other = asked.clone();
    for binding in other["defaults"][0]["bindings"].as_array_mut().unwrap() {
        let key = binding["keys"][0].as_str().unwrap();
        let (held, name) = key.split_at(key[..key.len() - 1].rfind('+').map_or(0, |at| at + 1));
        let mut held: Vec<String> = held.split_terminator('+').map(str::to_uppercase).collect();
        held.reverse();
        held.push(name.to_uppercase());
        binding["keys"] = json!([held.join("+")]);
    }
    let read = call("client.keymap", other).unwrap();
    for binding in &own {
        let action = binding["action"].as_str().unwrap();
        let read = bound(&read, &binding["keys"], action);
        let kept = ["[Copy]", "[Paste]"]
            .map(Value::from)
            .contains(&binding["keys"][0]);
        assert!(read != kept, "{binding}");
    }

    // What another client asks of this one reaches it, and the answer reaches
    // the other: that it was done, or that it was not and why.
    send(&client, client.snapshot().unshown().unwrap());
    let shown = wait(&client, "a terminal", drawn);
    let session = shown.selected_session.clone().unwrap();
    let pane = shown.current_window().unwrap().panes[0].id.clone();
    let clients = call("client.list", json!({})).unwrap();
    let me = list(&clients["clients"]).into_iter();
    let me = me
        .filter(|c| c["info"]["name"] == "Rex Lite")
        .map(|c| c["client_id"].clone());
    let me: Vec<Value> = me.collect();
    assert_eq!(me.len(), 1);
    let perform = |action: &str, error: Option<&'static str>| {
        let request =
            json!({"client_id": me[0], "action": action, "origin": "cli", "block_id": pane});
        std::thread::scope(|scope| {
            let asking = scope.spawn(|| call("client.perform", request));
            let mut asked = wait(&client, "another client's asking", |v| !v.asked.is_empty()).asked;
            let asked = asked.remove(0);
            assert_eq!(
                (asked.name.as_str(), asked.pane.as_ref()),
                (action, Some(&pane))
            );
            let call = asked.call.unwrap();
            send(&client, Action::Reply { call, error });
            asking.join().unwrap()
        })
    };
    assert_eq!(perform("pane.zoom", None).unwrap()["performed_by"], me[0]);
    let refused = perform("pane.zoom", Some("refused")).unwrap_err();
    assert!(refused.contains("refused"), "{refused}");
    // A window that takes nothing, as one out of sight does not, refuses in time.
    let untaken = json!({"client_id": me[0], "action": "pane.zoom"});
    let untaken = call("client.perform", untaken).unwrap_err();
    assert!(untaken.contains("refused"), "{untaken}");
    // What is not among this client's actions, the server refuses for it.
    let unknown = json!({"client_id": me[0], "action": "client.settings"});
    let unknown = call("client.perform", unknown).unwrap_err();
    assert!(unknown.contains("not_supported"), "{unknown}");
    assert!(client.snapshot().asked.is_empty());

    // The account's configuration changes the keys, and the server says when: a
    // key rebound, a key freed, a sequence a function answers, an action and a mode.
    let path = first["config"]["path"].as_str().unwrap();
    // Only that of a server made for testing, as `tests/smoke.py` makes one.
    let disposable = ["/work/rex-lite-live-", "/work/rex-lite-smoke-"];
    assert!(
        disposable.iter().any(|made| path.contains(made)),
        "refusing to write {path}"
    );
    let config = r#"
        rex.bind("ctrl+shift+d", "pane.zoom")
        rex.unbind("ctrl+tab")
        rex.bind("ctrl+x>ctrl+s", function(ctx, ev)
          rex.session.set_label{ label = "after " .. ev.prefix[1] }
          return true
        end)
        rex.action{
          name = "focus_mode", title = "Focus Mode", repeats = true,
          run = function(ctx, args) rex.client.queue("pane.zoom") end,
        }
        rex.mode("only", { exclusive = true })
        rex.bind("only/z", "client.mode.exit")
    "#;
    std::fs::write(path, config).unwrap();
    rex(&endpoint, &["config", "reload"]).unwrap();
    let zoom = json!(["ctrl+shift+d"]);
    let changed = keymap("a keymap read again", &|map| bound(map, &zoom, "pane.zoom"));
    let bindings = list(&changed["bindings"]);
    let at = |action: &str| {
        bindings
            .iter()
            .position(|b| b["keys"] == zoom && b["action"] == action)
    };
    // The key's own binding is still there, to be tried after the new one.
    assert!(at("pane.zoom") < at("pane.split.right") && at("pane.zoom").is_some());
    assert!(!bindings.iter().any(|b| b["keys"] == json!(["ctrl+tab"])));
    assert!(bound(
        &changed,
        &json!(["ctrl+shift+tab"]),
        "client.tab.previous"
    ));
    assert!(bound(
        &changed,
        &json!(["ctrl+x", "ctrl+s"]),
        "client.key.forward"
    ));
    let mode = &changed["modes"][0];
    assert_eq!(
        (&mode["name"], &mode["exclusive"]),
        (&json!("only"), &json!(true))
    );
    let in_mode = |b: &Value| b["mode"] == "only" && b["keys"] == json!(["z"]);
    assert!(bindings.iter().any(in_mode));
    let action = list(&changed["actions"]).into_iter();
    let action: Vec<_> = action.filter(|a| a["name"] == "focus_mode").collect();
    assert_eq!(
        (&action[0]["title"], &action[0]["repeats"]),
        (&json!("Focus Mode"), &json!(true))
    );
    assert!(
        list(&changed["diagnostics"]).is_empty(),
        "{:#}",
        changed["diagnostics"]
    );

    // The last key of a sequence goes to the function bound to it with the keys
    // before it, and without them to the terminal.
    let mut last = key(&pane, "s", "key_s", model::CTRL, true);
    if let Action::Key { prefix, .. } = &mut last {
        *prefix = vec!["ctrl+x".into()];
    }
    send(&client, last);
    wait(&client, "a sequence answered by a function", |v| {
        v.session().is_some_and(|s| s.name == "after ctrl+x")
    });
    let typed = |v: &View| screen(v, &pane).matches('s').count();
    let before = typed(&client.snapshot());
    send(&client, key(&pane, "s", "key_s", 0, true));
    wait(&client, "a key typed", |v| typed(v) > before);
    enter(&client, &pane);

    // An action of the configuration's is done by the server, and what it asks
    // of this client in turn comes back to be done here.
    let perform = |name: &str| Action::Perform {
        name: name.into(),
        args: Value::Null,
        origin: "key",
        pane: Some(pane.clone()),
    };
    send(&client, perform("focus_mode"));
    let queued = wait(&client, "an action asked in turn", |v| !v.asked.is_empty()).asked;
    assert_eq!(
        (queued[0].name.as_str(), &queued[0].call),
        ("pane.zoom", &None)
    );
    // So is a terminal's own: this one clears what the terminal shows.
    let typed = |v: &View| screen(v, &pane).contains("rex-lite-typed");
    let text = "echo rex-lite-typed".into();
    send(
        &client,
        Action::Text {
            pane: pane.clone(),
            text,
        },
    );
    wait(&client, "something typed", typed);
    enter(&client, &pane);
    wait(&client, "a second line of it", |v| {
        screen(v, &pane).matches("rex-lite-typed").count() == 2
    });
    send(&client, perform("com.superlogical.terminal.clear"));
    wait(&client, "a terminal cleared", |v| !typed(v));
    // One there is not is said to have failed.
    send(&client, perform("no_such_action"));
    let failed = wait(&client, "an action that failed", |v| v.error.is_some());
    assert!(failed.error.unwrap().contains("not_found"));
    assert_eq!(session, client.snapshot().selected_session.unwrap());
    // The configuration is left as it was found.
    std::fs::write(path, "").unwrap();
    rex(&endpoint, &["config", "reload"]).unwrap();
}
