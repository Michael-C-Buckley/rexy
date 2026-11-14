use super::*;

#[test]
fn what_is_typed_finds_a_part_of_any_word_in_any_case() {
    let words = ["Split Pane Right", "pane.split.right"];
    assert!(found("", &words) && found(" ", &words));
    assert!(found("split p", &words) && found("PANE.SP", &words));
    assert!(!found("spilt", &words) && !found("split.pane", &words));
}
