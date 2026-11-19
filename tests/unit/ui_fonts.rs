use super::*;

#[test]
fn a_cell_is_drawn_by_the_font_for_what_it_holds() {
    // Letters, and symbols written as letters are, whatever room they take.
    assert_eq!(Kind::of("a", false), Kind::Text);
    assert_eq!(Kind::of("日", true), Kind::Text);
    assert_eq!(Kind::of("✓", false), Kind::Text);
    assert_eq!(Kind::of("⚠", false), Kind::Text);
    // A picture is what the terminal gives two cells, of the symbols.
    assert_eq!(Kind::of("😀", true), Kind::Emoji);
    assert_eq!(Kind::of("✅", true), Kind::Emoji);
    assert_eq!(Kind::of("🇺🇸", true), Kind::Emoji);
    assert_eq!(Kind::of("👨\u{200D}👩\u{200D}👧", true), Kind::Emoji);
    // And what asks to be one, or asks not to be.
    assert_eq!(Kind::of("⚠\u{FE0F}", false), Kind::Emoji);
    assert_eq!(Kind::of("❤\u{FE0F}", true), Kind::Emoji);
    assert_eq!(Kind::of("✅\u{FE0E}", true), Kind::Text);
    // A mark is its font's own, from where Unicode leaves room for those.
    assert_eq!(Kind::of("\u{F07C}", false), Kind::Icon);
    assert_eq!(Kind::of("\u{E0A0}", false), Kind::Icon);
    assert_eq!(Kind::of("\u{F0318}", false), Kind::Icon);
}
