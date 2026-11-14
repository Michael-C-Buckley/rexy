use std::fmt::Write as _;

use libghostty_vt::Terminal;

use super::*;
use crate::terminal::tests::render;

const SIZE: f32 = 14.0;

/// What a pane shows once a program has written these bytes.
fn frame(cols: u16, rows: u16, bytes: &[u8]) -> Arc<Frame> {
    let mut terminal = Terminal::new(cols, rows).unwrap();
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
    let galley = |view: &PaneView| Arc::clone(&view.cached_rows[0].glyphs[0].galley);

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
