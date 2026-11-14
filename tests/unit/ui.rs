use super::*;

fn what(name: &str) -> egui::Id {
    egui::Id::unique(name)
}

#[test]
fn something_busy_closes_only_when_asked_twice() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
    assert!(!closing.press(what("tab"), false, true, now));
    assert_eq!(closing.ask(what("tab"), false), Ask::Again);
    assert!(closing.press(what("tab"), false, true, now));
    // Having closed, it waits for nothing.
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
    assert_eq!(closing.lapse(now), None);
}

#[test]
fn something_with_nothing_to_lose_closes_at_once() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert_eq!(closing.ask(what("tab"), true), Ask::No);
    assert!(closing.press(what("tab"), true, true, now));
    assert_eq!(closing.lapse(now), None);
}

#[test]
fn asking_for_one_is_not_asking_for_another() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert!(!closing.press(what("one"), false, true, now));
    // A press meant for the first that lands on its neighbour closes neither.
    assert!(!closing.press(what("other"), false, true, now));
    assert_eq!(closing.ask(what("one"), false), Ask::First);
    assert_eq!(closing.ask(what("other"), false), Ask::Again);
    // Nor does it spare the neighbour the asking, were that one idle.
    assert_eq!(closing.ask(what("one"), true), Ask::No);
}

#[test]
fn the_wait_ends_when_the_pointer_leaves() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    closing.press(what("tab"), false, true, now);
    // The drawing it was asked in counts the pointer as by it.
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    closing.keep();
    let later = now + Duration::from_secs(1);
    assert_eq!(
        closing.lapse(later),
        Some(Closing::TIME - Duration::from_secs(1))
    );
    // Not kept since, it ends, and says so once.
    assert_eq!(closing.lapse(later), Some(Duration::ZERO));
    assert_eq!(closing.lapse(later), None);
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
}

#[test]
fn the_wait_ends_with_time_wherever_the_pointer_is() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    // Asked with a key, it does not go by the pointer.
    closing.press(what("pane"), false, false, now);
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    assert_eq!(closing.lapse(now + Closing::TIME), Some(Duration::ZERO));
    assert_eq!(closing.ask(what("pane"), false), Ask::First);
    // Asked with the pointer and kept, it ends with time too.
    closing.press(what("tab"), false, true, now);
    closing.keep();
    assert_eq!(closing.lapse(now + Closing::TIME), Some(Duration::ZERO));
}

/// A turn of the wheel toward what came before, by a wheel's notches or a
/// touchpad's points, over cells twenty points high.
fn steps(wheel: Wheel, unit: egui::MouseWheelUnit, by: f32, rest: &mut f32) -> i32 {
    let turned = egui::Event::MouseWheel {
        unit,
        delta: egui::vec2(0.0, by),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    };
    let grid = Grid {
        rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 400.0)),
        cell: egui::vec2(8.0, 20.0),
        font_size: 15.0,
    };
    wheel.steps(&[turned], grid, rest)
}

#[test]
fn a_notch_is_three_lines_and_one_step_for_a_program() {
    use egui::MouseWheelUnit::Line;
    assert_eq!(steps(Wheel::History, Line, 1.0, &mut 0.0), -3);
    assert_eq!(steps(Wheel::Arrows, Line, -1.0, &mut 0.0), 3);
    assert_eq!(steps(Wheel::Program, Line, 1.0, &mut 0.0), -1);
    // A fast turn is several notches in one event, and none is lost.
    assert_eq!(steps(Wheel::Program, Line, -4.0, &mut 0.0), 4);
}

#[test]
fn a_touchpad_moves_as_far_as_it_says_and_keeps_what_is_left() {
    use egui::MouseWheelUnit::{Line, Point};
    let (mut rest, mut program) = (0.0, 0.0);
    // A line and a half, then half a line.
    let history = [30.0, 10.0].map(|by| steps(Wheel::History, Point, by, &mut rest));
    assert_eq!((history, rest), ([-1, -1], 0.0));
    // A program is told once the touchpad has gone as far as a notch goes.
    let told = [30.0, 30.0].map(|by| steps(Wheel::Program, Point, by, &mut program));
    assert_eq!(told, [0, -1]);
    // Under X11 a touchpad says parts of a notch.
    let parts = [0.5, 0.5].map(|by| steps(Wheel::History, Line, by, &mut rest));
    assert_eq!((parts, rest), ([-1, -2], 0.0));
}
