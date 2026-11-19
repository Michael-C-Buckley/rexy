use eframe::egui::{Modifiers, Pos2, Rect};

use super::*;

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
        spread: 0.0,
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

#[test]
fn only_the_buttons_rex_names_are_a_programs() {
    use egui::PointerButton::{Extra1, Extra2, Middle, Primary, Secondary};
    let named = [Primary, Secondary, Middle].map(mouse_button);
    assert_eq!(named, [Some("left"), Some("right"), Some("middle")]);
    // Those at a mouse's side are no left click.
    assert_eq!([Extra1, Extra2].map(mouse_button), [None, None]);
}
