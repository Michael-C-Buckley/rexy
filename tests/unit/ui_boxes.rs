use super::*;

#[test]
fn lines_are_known_by_their_arms() {
    use Drawing::Arms;
    // Left, right, up, down.
    assert_eq!(drawing('─'), Some(Arms([1, 1, 0, 0])));
    assert_eq!(drawing('┃'), Some(Arms([0, 0, 2, 2])));
    assert_eq!(drawing('┌'), Some(Arms([0, 1, 0, 1])));
    assert_eq!(drawing('┘'), Some(Arms([1, 0, 1, 0])));
    assert_eq!(drawing('┝'), Some(Arms([0, 2, 1, 1])));
    assert_eq!(drawing('╈'), Some(Arms([2, 2, 1, 2])));
    assert_eq!(drawing('╼'), Some(Arms([1, 2, 0, 0])));
    assert_eq!(drawing('╷'), Some(Arms([0, 0, 0, 1])));
}

#[test]
fn dashes_arcs_and_blocks_are_drawn_and_the_rest_left_to_the_font() {
    use Drawing::*;
    let dashes = |upright, count, heavy| Dashes {
        upright,
        count,
        heavy,
    };
    assert_eq!(drawing('┄'), Some(dashes(false, 3, false)));
    assert_eq!(drawing('┇'), Some(dashes(true, 3, true)));
    assert_eq!(drawing('┈'), Some(dashes(false, 4, false)));
    assert_eq!(drawing('╏'), Some(dashes(true, 2, true)));
    let arc = |right, down| Arc { right, down };
    let arcs = [drawing('╭'), drawing('╮'), drawing('╯'), drawing('╰')];
    let turns = [(true, true), (false, true), (false, false), (true, false)];
    assert_eq!(arcs, turns.map(|(right, down)| Some(arc(right, down))));
    assert_eq!(drawing('▀'), Some(Block([0, 0, 8, 4])));
    assert_eq!(drawing('▁'), Some(Block([0, 7, 8, 8])));
    assert_eq!(drawing('█'), Some(Block([0, 0, 8, 8])));
    assert_eq!(drawing('▉'), Some(Block([0, 0, 7, 8])));
    assert_eq!(drawing('▏'), Some(Block([0, 0, 1, 8])));
    assert_eq!(drawing('▐'), Some(Block([4, 0, 8, 8])));
    assert_eq!(drawing('▒'), Some(Shade(2)));
    assert_eq!(drawing('▖'), Some(Quarters(0b0100)));
    assert_eq!(drawing('▚'), Some(Quarters(0b1001)));
    assert_eq!(drawing('▟'), Some(Quarters(0b1110)));
    // Double lines, diagonals and everything else.
    for other in ['═', '╬', '╱', '╳', 'a', '→', '◆'] {
        assert_eq!(drawing(other), None, "{other}");
    }
}
