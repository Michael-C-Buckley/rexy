use super::*;

const AREA: Rect = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(400.0, 300.0));

/// A tab split left and right, and its right side split above and below. Draws
/// both borders while the pointer does `events`, and says what is asked of each.
fn draw(ctx: &egui::Context, events: Vec<egui::Event>) -> [Option<f32>; 2] {
    let border = |after: &str, upright, room, at| Border {
        before: "a".into(),
        after: after.into(),
        upright,
        room,
        at,
        float: None,
    };
    let across = border("b", true, [0.0, 0.0, 1.0, 1.0], 0.5);
    let down = border("c", false, [0.5, 0.0, 0.5, 1.0], 0.4);
    let input = egui::RawInput { screen_rect: Some(AREA), events, ..Default::default() };
    let mut asked = [None; 2];
    let output = ctx.run_ui(input, |ui| {
        asked = [&across, &down].map(|border| drag(ui, border, AREA));
    });
    output.drop_without_applying_deltas();
    asked
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_border_goes_where_it_is_dragged_within_its_room() {
    let ctx = egui::Context::default();
    let moved = |x, y| vec![egui::Event::PointerMoved(egui::pos2(x, y))];
    // The upright border, at the middle of the tab. Pressing it asks nothing.
    assert_eq!(draw(&ctx, moved(200.0, 20.0)), [None, None]);
    assert_eq!(draw(&ctx, vec![button(egui::pos2(200.0, 20.0), true)]), [None, None]);
    assert_eq!(draw(&ctx, moved(300.0, 20.0)), [Some(0.75), None]);
    // A pointer that rests asks nothing more, and neither side is squeezed out.
    assert_eq!(draw(&ctx, vec![]), [None, None]);
    assert_eq!(draw(&ctx, moved(900.0, 20.0)), [Some(0.95), None]);
    draw(&ctx, vec![button(egui::pos2(900.0, 20.0), false)]);
    // The border under it lies across the right half alone, and moves up and down.
    assert_eq!(draw(&ctx, moved(100.0, 120.0)), [None, None]);
    draw(&ctx, vec![button(egui::pos2(100.0, 120.0), true)]);
    assert_eq!(draw(&ctx, moved(100.0, 60.0)), [None, None]);
    draw(&ctx, vec![button(egui::pos2(100.0, 60.0), false)]);
    draw(&ctx, moved(300.0, 120.0));
    draw(&ctx, vec![button(egui::pos2(300.0, 120.0), true)]);
    assert_eq!(draw(&ctx, moved(300.0, 60.0)), [None, Some(0.2)]);
    draw(&ctx, vec![button(egui::pos2(300.0, 60.0), false)]);
}

#[test]
fn a_double_click_gives_both_sides_the_same_room() {
    let ctx = egui::Context::default();
    let at = egui::pos2(300.0, 120.0);
    draw(&ctx, vec![egui::Event::PointerMoved(at)]);
    let clicks = [true, false, true, false].map(|pressed| draw(&ctx, vec![button(at, pressed)]));
    assert_eq!(clicks, [[None, None], [None, None], [None, None], [None, Some(0.5)]]);
}
