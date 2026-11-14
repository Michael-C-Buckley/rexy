use super::*;

fn pane(id: &str, [x, y, width, height]: [f32; 4]) -> Pane {
    Pane {
        id: id.into(),
        title: String::new(),
        program: Program::default(),
        activity: Activity::Idle,
        x,
        y,
        width,
        height,
        frame: None,
        status: String::new(),
        owner: false,
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
