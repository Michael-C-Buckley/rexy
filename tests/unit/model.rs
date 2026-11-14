use super::*;

fn pane(id: &str, [x, y, width, height]: [f32; 4]) -> Pane {
    Pane {
        id: id.into(),
        title: String::new(),
        program: Program::default(),
        activity: Activity::Idle,
        message: String::new(),
        x,
        y,
        width,
        height,
        frame: None,
        status: String::new(),
        owner: false,
        idle: false,
    }
}

#[test]
fn the_pane_beside_another_is_the_nearest_alongside_it() {
    // One pane down the left, and three on the right: one above two side by side.
    let window = Window {
        id: "w".into(),
        label: None,
        focus: None,
        zoomed: false,
        hidden: Vec::new(),
        borders: Vec::new(),
        panes: vec![
            pane("left", [0.0, 0.0, 0.6, 1.0]),
            pane("top", [0.6, 0.0, 0.4, 0.25]),
            pane("low", [0.6, 0.25, 0.2, 0.75]),
            pane("corner", [0.8, 0.25, 0.2, 0.75]),
        ],
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
    for (pane, toward) in [
        ("left", left),
        ("top", up),
        ("corner", right),
        ("low", down),
    ] {
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
    let tab = |panes, hidden| Window {
        id: "w".into(),
        label: None,
        focus: None,
        zoomed: true,
        panes,
        hidden,
        borders: Vec::new(),
    };
    // One out of sight is heard too, and of two as pressing, the first.
    let hidden = vec![
        says("b", Activity::Done, "Built"),
        says("c", Activity::Done, "Later"),
    ];
    let busy = tab(vec![says("a", Activity::Working, "Thinking")], hidden);
    assert_eq!(busy.says(), (Activity::Done, "Built"));
    let quiet = tab(vec![pane("d", [0.0; 4])], Vec::new());
    assert_eq!(quiet.says(), (Activity::Idle, ""));
    assert_eq!(tab(Vec::new(), Vec::new()).says(), (Activity::Idle, ""));
    let session = Session {
        id: "s".into(),
        name: "s".into(),
        active: None,
        windows: vec![quiet, busy],
    };
    assert_eq!(session.activity(), Activity::Done);
    // It is told as one line, of whichever parts there are.
    assert_eq!(line("Done", "Built"), "Done: Built");
    assert_eq!(line("Done", ""), "Done");
    assert_eq!(line("", "Thinking"), "Thinking");
    assert_eq!(line("", ""), "");
}

fn tab(id: &str) -> Window {
    Window {
        id: id.into(),
        label: None,
        focus: None,
        zoomed: false,
        panes: Vec::new(),
        hidden: Vec::new(),
        borders: Vec::new(),
    }
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
fn the_tab_on_show_is_the_servers_once_it_has_answered() {
    let mut view = View {
        sessions: vec![session("s", Some("b"), &["a", "b", "c"])],
        selected_session: Some("s".into()),
        ..Default::default()
    };
    let shown = |view: &mut View, asked: Option<&str>, answered| {
        view.selected_window = asked.map(str::to_owned);
        view.settle(answered);
        view.selected_window.clone()
    };
    // With nothing asked for, the tab the server shows.
    assert_eq!(shown(&mut view, None, false).as_deref(), Some("b"));
    // A tab just asked for stays while the server has yet to answer, whatever an
    // older word from it says, and gives way to the server's when it has.
    assert_eq!(shown(&mut view, Some("c"), false).as_deref(), Some("c"));
    assert_eq!(shown(&mut view, Some("c"), true).as_deref(), Some("b"));
    // One that has closed does not stay, answered or not.
    assert_eq!(shown(&mut view, Some("gone"), false).as_deref(), Some("b"));
    // Nor does the server's, where the view held is yet to have it: then the first.
    view.sessions[0].active = Some("gone".into());
    assert_eq!(shown(&mut view, None, true).as_deref(), Some("a"));
    view.sessions[0].windows.clear();
    assert_eq!(shown(&mut view, Some("a"), false), None);
    // A session that has ended is on show no longer, with its tab.
    view.sessions.clear();
    view.selected_window = Some("a".into());
    view.settle(true);
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
