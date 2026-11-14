use std::fmt::Write as _;

use super::*;
use crate::terminal::tests::{render, terminal};

const SIZE: f32 = 14.0;

/// What a pane shows once a program has written these bytes.
fn frame(cols: u16, rows: u16, bytes: &[u8]) -> Arc<Frame> {
    let mut terminal = terminal(cols, rows);
    terminal.vt_write(bytes);
    render(&terminal)
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    style::apply(&ctx);
    ctx
}

/// One pass at a display scale that paints a pane and nothing else.
fn paint(ctx: &egui::Context, view: &mut PaneView, frame: &Frame, scale: f32) -> egui::FullOutput {
    let mut input = egui::RawInput::default();
    let window = input.viewports.entry(egui::ViewportId::ROOT).or_default();
    window.native_pixels_per_point = Some(scale);
    let mut output = ctx.run_ui(input, |ui| {
        let grid = Grid::new(ui.ctx(), ui.max_rect(), SIZE);
        view.paint(ui.painter(), grid, frame, true);
    });
    output.textures_delta.clear();
    output
}

/// A screen where no two neighbouring runs look alike.
fn colourful(cols: u16, rows: u16) -> String {
    let mut screen = String::new();
    for run in 0..rows * cols / 4 {
        let look = ["", ";1", ";3", ";4", ";4:3", ";9", ";2", ";53"][usize::from(run % 8)];
        let (ink, ground) = (run % 200 + 20, run % 7);
        write!(screen, "\x1b[0;38;5;{ink};48;5;{ground}{look}m").unwrap();
        screen.push_str(["ab/d", "==> ", "é://", "x  y"][usize::from(run % 4)]);
    }
    screen
}

#[test]
fn a_screen_of_colours_is_one_mesh_and_only_a_glyph_too_wide_is_clipped() {
    let ctx = context();
    let (cols, rows) = (100, 30);
    let screen = colourful(cols, rows);
    let fits = frame(cols, rows, screen.as_bytes());
    let output = paint(&ctx, &mut PaneView::default(), &fits, 1.0);
    assert!(output.shapes.len() > usize::from(cols * rows / 4));
    assert_eq!(ctx.tessellate(output.shapes, 1.0).len(), 1);

    // egui's own icons are wider than a cell, which is all the terminal gives them.
    let screen = format!("{screen}\x1b[10;10H\u{e624}\x1b[20;20H\u{e624}");
    let spills = frame(cols, rows, screen.as_bytes());
    let output = paint(&ctx, &mut PaneView::default(), &spills, 1.0);
    let meshes = ctx.tessellate(output.shapes, 1.0);
    let cell = ctx.fonts_mut(|f| f.glyph_width(&FontId::monospace(SIZE), 'M'));
    let widths = meshes.iter().map(|mesh| mesh.clip_rect.width());
    let clipped = widths.filter(|width| (width - cell).abs() < 0.01).count();
    assert_eq!((meshes.len(), clipped), (5, 2));
}

#[test]
fn rows_are_shaped_again_when_the_scale_or_the_font_atlas_changes() {
    let ctx = context();
    let frame = frame(20, 2, b"hello");
    let mut view = PaneView::default();
    let galley = |view: &PaneView| {
        let mut rows = view.cached_rows.values();
        Arc::clone(&rows.find_map(|row| row.glyphs.first()).unwrap().galley)
    };

    paint(&ctx, &mut view, &frame, 1.0);
    let first = galley(&view);
    paint(&ctx, &mut view, &frame, 1.0);
    assert!(Arc::ptr_eq(&first, &galley(&view)));

    paint(&ctx, &mut view, &frame, 2.0);
    assert_eq!(galley(&view).pixels_per_point, 2.0);

    // A new atlas, as egui also makes when the old one fills up.
    let second = galley(&view);
    ctx.all_styles_mut(|style| style.visuals.text_options.font_hinting ^= true);
    paint(&ctx, &mut view, &frame, 2.0);
    assert!(!Arc::ptr_eq(&second, &galley(&view)));
}

#[test]
fn a_row_that_moves_on_screen_keeps_its_shaped_text_and_is_drawn_where_it_is() {
    let ctx = context();
    let first = frame(20, 3, b"one\r\ntwo\r\nthree");
    // The same rows a row further up, as output that scrolls leaves them.
    let rows = [&first.lines[1], &first.lines[2], &first.lines[0]];
    let mut moved = (*first).clone();
    moved.lines = rows.map(Arc::clone).to_vec();
    let mut view = PaneView::default();
    // How far down the screen a pass draws a text.
    let mut top = |frame: &Frame, text: &str| {
        let output = paint(&ctx, &mut view, frame, 1.0);
        let shapes = output.shapes.iter().map(|clipped| &clipped.shape);
        let at = |shape: &Shape| match shape {
            Shape::Text(shape) if shape.galley.text() == text => Some(shape.pos.y),
            _ => None,
        };
        shapes.filter_map(at).next().unwrap()
    };
    assert!(top(&first, "one") < top(&first, "two"));
    let shaped = ctx.fonts(|fonts| fonts.num_galleys_in_cache());
    assert!(top(&moved, "two") < top(&moved, "one"));
    // egui forgets a text that a whole pass did not ask it for, as it has all three
    // rows: none was shaped again.
    assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), shaped - 3);
    // A row that is shown no more is forgotten here.
    moved.lines.truncate(2);
    top(&moved, "two");
    assert_eq!(view.cached_rows.len(), 2);
}

#[test]
fn each_kind_of_underline_is_drawn_as_itself() {
    let ctx = context();
    let kinds = b"\x1b[4:3mcurly\x1b[4:4mdotted\x1b[4:5mdashed\x1b[4:2mdouble\x1b[4:1msingle";
    let output = paint(&ctx, &mut PaneView::default(), &frame(40, 1, kinds), 1.0);
    let count = |wanted: fn(&Shape) -> bool| {
        let shapes = output.shapes.iter().map(|clipped| &clipped.shape);
        shapes.filter(|shape| wanted(shape)).count()
    };
    // A wave over five cells, two dots in each of six, and a dash in each of six.
    let waves = count(|shape| matches!(shape, Shape::Path(wave) if wave.points.len() == 41));
    let dots = count(|shape| matches!(shape, Shape::Circle(_)));
    let lines = count(|shape| matches!(shape, Shape::LineSegment { .. }));
    assert_eq!((waves, dots, lines), (1, 12, 6 + 2 + 1));
    assert_eq!(ctx.tessellate(output.shapes, 1.0).len(), 1);
}

#[test]
#[ignore = "a benchmark: run it alone, with --release and --nocapture"]
fn bench_rows() {
    use crate::terminal::tests::{Bench, Meter, SCREENS, measure};
    for (name, write) in SCREENS {
        let ctx = context();
        let mut view = PaneView::default();
        // egui has no fonts until its first pass.
        paint(&ctx, &mut view, &frame(2, 1, b""), 1.0);
        let mut screen = Bench::new(write);
        measure(&format!("shape: {name}"), |meter| {
            let frame = screen.next(&mut Meter::default());
            meter.time(|| {
                for row in &frame.lines {
                    CachedRow::new(&ctx, row, SIZE);
                }
            });
        });
        // A pass of egui's own, which forgets the text that the pass before did not show.
        let mut screen = Bench::new(write);
        measure(&format!("paint: {name}"), |meter| {
            let frame = screen.next(&mut Meter::default());
            meter.time(|| paint(&ctx, &mut view, &frame, 1.0));
        });
    }
}
