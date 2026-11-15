use super::*;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
}

/// Lay a tab out at `time`, and say where its panes are drawn.
fn lay(
    motion: &mut Motion,
    ctx: &egui::Context,
    time: f64,
    tab: &str,
    belong: &[(&str, Rect)],
    still: bool,
) -> Vec<Rect> {
    let input = egui::RawInput { time: Some(time), ..Default::default() };
    let mut shown = Vec::new();
    let output = ctx.run_ui(input, |ui| {
        shown = motion.place(ui.ctx(), tab, belong, still);
    });
    output.drop_without_applying_deltas();
    shown
}

#[test]
fn a_pane_moves_to_its_place_and_a_new_one_takes_the_room_given_up() {
    let ctx = egui::Context::default();
    let mut motion = Motion::default();
    let whole = rect(0.0, 0.0, 100.0, 100.0);
    let (left, right) = (rect(0.0, 0.0, 50.0, 100.0), rect(50.0, 0.0, 50.0, 100.0));
    // A tab that comes on show is where it belongs at once.
    assert_eq!(lay(&mut motion, &ctx, 1.0, "t", &[("a", whole)], false), [whole]);
    // Split, the one gives way as the other comes in from the far side, and the
    // two never lie over one another.
    let split = [("a", left), ("b", right)];
    let start = lay(&mut motion, &ctx, 2.0, "t", &split, false);
    assert_eq!(start[0], whole);
    assert_eq!(start[1].width(), 0.0);
    let midway = lay(&mut motion, &ctx, 2.0 + TIME / 2.0, "t", &split, false);
    assert!(midway[0].right() > 50.0 && midway[0].right() < 100.0);
    assert!((midway[0].right() - midway[1].left()).abs() < 0.01);
    assert_eq!(midway[1].right(), 100.0);
    assert_eq!(lay(&mut motion, &ctx, 3.0, "t", &split, false), [left, right]);
    // One that closes leaves its room to the other, which grows into it.
    let grown = lay(&mut motion, &ctx, 4.0 + TIME / 2.0, "t", &[("a", whole)], false);
    assert_eq!(grown, [left]);
    let grown = lay(&mut motion, &ctx, 4.0 + TIME, "t", &[("a", whole)], false);
    assert!(grown[0].width() > 50.0);
}

#[test]
fn nothing_moves_while_it_is_held_or_when_another_tab_comes_on_show() {
    let ctx = egui::Context::default();
    let mut motion = Motion::default();
    let (whole, half) = (rect(0.0, 0.0, 100.0, 100.0), rect(0.0, 0.0, 50.0, 100.0));
    lay(&mut motion, &ctx, 1.0, "t", &[("a", whole)], false);
    // A border that is dragged takes its panes with it.
    assert_eq!(lay(&mut motion, &ctx, 2.0, "t", &[("a", half)], true), [half]);
    // Another tab is simply there, and so is this one on coming back.
    assert_eq!(lay(&mut motion, &ctx, 3.0, "u", &[("b", whole)], false), [whole]);
    assert_eq!(lay(&mut motion, &ctx, 4.0, "t", &[("a", whole)], false), [whole]);
}
