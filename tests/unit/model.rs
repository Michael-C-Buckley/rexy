use super::*;

fn pane(id: &str, [x, y, width, height]: [f32; 4]) -> Pane {
    Pane { id: id.into(), x, y, width, height, ..Pane::default() }
}

#[test]
fn the_pane_beside_another_is_the_nearest_alongside_it() {
    // One pane down the left, and three on the right: one above two side by side.
    let window = Window {
        id: "w".into(),
        panes: vec![
            pane("left", [0.0, 0.0, 0.6, 1.0]),
            pane("top", [0.6, 0.0, 0.4, 0.25]),
            pane("low", [0.6, 0.25, 0.2, 0.75]),
            pane("corner", [0.8, 0.25, 0.2, 0.75]),
        ],
        ..Window::default()
    };
    let beside = |pane, toward| window.beside(pane, toward).map(|pane| pane.id.as_str());
    let [left, right, up, down] = [[-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]];
    assert_eq!(beside("left", right), Some("top"));
    assert_eq!(beside("top", down), Some("low"));
    assert_eq!(beside("low", right), Some("corner"));
    assert_eq!(beside("corner", up), Some("top"));
    // Past the nearer pane, not the one it touches only across that pane.
    assert_eq!(beside("corner", left), Some("low"));
    assert_eq!(beside("low", left), Some("left"));
    for (pane, toward) in [("left", left), ("top", up), ("corner", right), ("low", down)] {
        assert_eq!(beside(pane, toward), None);
    }
    assert_eq!(beside("gone", right), None);
}

#[test]
fn a_tab_says_the_most_pressing_thing_any_of_its_panes_has_to() {
    let says = |id: &str, activity, message: &str| Pane {
        activity,
        message: message.into(),
        ..pane(id, [0.0; 4])
    };
    let tab =
        |panes, hidden| Window { id: "w".into(), zoomed: true, panes, hidden, ..Window::default() };
    // One out of sight is heard too, and of two as pressing, the first.
    let hidden = vec![says("b", Activity::Done, "Built"), says("c", Activity::Done, "Later")];
    let busy = tab(vec![says("a", Activity::Working, "Thinking")], hidden);
    assert_eq!(busy.says(), (Activity::Done, "Built"));
    let quiet = tab(vec![pane("d", [0.0; 4])], Vec::new());
    assert_eq!(quiet.says(), (Activity::Idle, ""));
    assert_eq!(tab(Vec::new(), Vec::new()).says(), (Activity::Idle, ""));
    let session =
        Session { id: "s".into(), name: "s".into(), active: None, windows: vec![quiet, busy] };
    assert_eq!(session.activity(), Activity::Done);
    // It is told as one line, of whichever parts there are.
    assert_eq!(line("Done", "Built"), "Done: Built");
    assert_eq!(line("Done", ""), "Done");
    assert_eq!(line("", "Thinking"), "Thinking");
    assert_eq!(line("", ""), "");
}

fn tab(id: &str) -> Window {
    Window { id: id.into(), ..Window::default() }
}

fn session(id: &str, active: Option<&str>, tabs: &[&str]) -> Session {
    Session {
        id: id.into(),
        name: id.into(),
        active: active.map(str::to_owned),
        windows: tabs.iter().map(|id| tab(id)).collect(),
    }
}

#[test]
fn the_tab_on_show_stays_while_it_is_there_whatever_the_server_shows() {
    let mut view = View {
        sessions: vec![session("s", Some("b"), &["a", "b", "c"])],
        selected_session: Some("s".into()),
        ..Default::default()
    };
    let shown = |view: &mut View, asked: Option<&str>| {
        view.selected_window = asked.map(str::to_owned);
        view.settle();
        view.selected_window.clone()
    };
    // With none shown yet, the tab the server shows.
    assert_eq!(shown(&mut view, None).as_deref(), Some("b"));
    // One that is shown stays, whatever the server comes to show: another
    // client's choice of tab is not this window's.
    assert_eq!(shown(&mut view, Some("c")).as_deref(), Some("c"));
    view.sessions[0].active = Some("a".into());
    view.settle();
    assert_eq!(view.selected_window.as_deref(), Some("c"));
    // One that has closed gives way to the server's.
    assert_eq!(shown(&mut view, Some("gone")).as_deref(), Some("a"));
    // And where the view held is yet to have the server's, to the first.
    view.sessions[0].active = Some("gone".into());
    assert_eq!(shown(&mut view, None).as_deref(), Some("a"));
    view.sessions[0].windows.clear();
    assert_eq!(shown(&mut view, Some("a")), None);
    // A session that has ended is on show no longer, with its tab.
    view.sessions.clear();
    view.selected_window = Some("a".into());
    view.settle();
    assert_eq!((view.selected_session, view.selected_window), (None, None));
}

#[test]
fn only_a_server_showing_nothing_is_asked_for_its_first_session() {
    let mut view = View {
        connected: true,
        sessions: vec![session("s", None, &[]), session("t", None, &[])],
        ..Default::default()
    };
    let first = |view: &View| match view.unshown() {
        Some(Action::SelectSession(session)) => Some(session),
        _ => None,
    };
    assert_eq!(first(&view).as_deref(), Some("s"));
    view.selected_session = Some("t".into());
    assert_eq!(first(&view), None);
    view.selected_session = None;
    view.connected = false;
    assert_eq!(first(&view), None);
    view.connected = true;
    view.sessions.clear();
    assert_eq!(first(&view), None);
}

#[test]
fn a_key_moves_the_border_of_the_split_a_pane_is_in_and_balancing_evens_them() {
    // One pane beside two, one above the other.
    let border = |before: &str, after: &str, upright, room, at| Border {
        before: before.into(),
        after: after.into(),
        upright,
        room,
        at,
    };
    let tab = |split: f32| Window {
        panes: vec![
            pane("a", [0.0, 0.0, split, 1.0]),
            pane("b", [split, 0.0, 1.0 - split, 0.5]),
            pane("c", [split, 0.5, 1.0 - split, 0.5]),
        ],
        borders: vec![
            border("a", "b", true, [0.0, 0.0, 1.0, 1.0], split),
            border("b", "c", false, [split, 0.0, 1.0 - split, 1.0], 0.5),
        ],
        ..Default::default()
    };
    let half = tab(0.5);
    // The border by the pane before it, and the share that side is to have.
    let nudged = |pane: &str, toward| {
        let asked = half.nudged(pane, toward)?;
        Some(format!("{} {:.2}", asked.before, asked.ratio))
    };
    assert_eq!(nudged("a", [1.0, 0.0]).as_deref(), Some("a 0.55"));
    assert_eq!(nudged("b", [-1.0, 0.0]).as_deref(), Some("a 0.45"));
    // The innermost split that lies that way.
    assert_eq!(nudged("b", [0.0, 1.0]).as_deref(), Some("b 0.55"));
    assert_eq!(nudged("c", [0.0, -1.0]).as_deref(), Some("b 0.45"));
    // No split of its own lies up and down of the one beside the two.
    assert_eq!(nudged("a", [0.0, 1.0]), None);
    // A border goes no further than leaves each side a little.
    let last = tab(0.93).nudged("a", [1.0, 0.0]).unwrap();
    assert_eq!(last, half.borders[0].moved(Border::MOST));
    assert_eq!(tab(Border::MOST).nudged("a", [1.0, 0.0]), None);
    // Balanced, the one is as wide as the two, which share their half.
    assert_eq!(tab(0.3).balanced(), [half.borders[0].moved(0.5)]);
    assert!(half.balanced().is_empty());
    // Three in a row have a third each: the first, and the others half the rest.
    let mut row = tab(0.5);
    row.panes[1] = pane("b", [0.5, 0.0, 0.3, 1.0]);
    row.panes[2] = pane("c", [0.8, 0.0, 0.2, 1.0]);
    row.borders[1] = border("b", "c", true, [0.5, 0.0, 0.5, 1.0], 0.8);
    let [first, rest] = &row.balanced()[..] else {
        panic!("both borders move");
    };
    assert!((first.ratio - 1.0 / 3.0).abs() < 0.001);
    assert_eq!(*rest, row.borders[1].moved(0.5));
}

#[test]
fn a_directory_is_named_shortly_for_a_title() {
    assert_eq!(folder("/home/ada"), "~");
    assert_eq!(folder("/home/ada/code"), "~/code");
    assert_eq!(folder("/home/ada/code/x"), "~/code/x");
    // Many levels down, the last two say where.
    let deep = "/home/ada/work/projects/rex-lite";
    assert_eq!(folder(deep), "…/projects/rex-lite");
    assert_eq!(folder("/var/log/nginx"), "…/log/nginx");
    // A shell may give it as an address, with its machine's name.
    assert_eq!(folder("file://studio/home/ada/code"), "~/code");
    assert_eq!(folder("/Users/a/b"), "~/b");
    assert_eq!(folder("/root"), "~");
    assert_eq!(folder("/rootfs/x"), "/rootfs/x");
    assert_eq!(folder("/"), "/");
    assert_eq!(folder("/etc"), "/etc");
}

#[test]
fn a_pane_says_how_far_along_it_is_only_while_it_works() {
    let pane = |activity, progress| Pane {
        activity,
        progress,
        task: "Refactor auth".into(),
        message: "running tests".into(),
        ..Pane::default()
    };
    assert_eq!(pane(Activity::Working, Some(60)).word().as_deref(), Some("60%"));
    assert_eq!(pane(Activity::Working, None).word(), None);
    assert_eq!(pane(Activity::Done, Some(60)).word().as_deref(), Some("Done"));
    assert_eq!(pane(Activity::Idle, Some(60)).word(), None);
    let saying = pane(Activity::Working, None).saying();
    assert_eq!(saying, "Refactor auth: running tests");
}
