use super::*;
use crate::{model::Wait, program::Program};

#[test]
fn a_pane_is_listed_under_what_it_is_doing_and_a_shell_at_rest_is_not() {
    let pane = |activity, program: &str| Pane {
        activity,
        program: Program::named([program]).unwrap_or_default(),
        ..Default::default()
    };
    let under = |activity, program| kind(&pane(activity, program)).map(|kind| KINDS[kind]);
    assert_eq!(
        under(Activity::Blocked(Wait::Approval), "zsh"),
        Some("Needs you")
    );
    assert_eq!(under(Activity::Error, "cargo"), Some("Needs you"));
    assert_eq!(under(Activity::Done, "zsh"), Some("Done"));
    assert_eq!(under(Activity::Working, "zsh"), Some("Working"));
    // With nothing to say, only a coding agent is worth a row.
    assert_eq!(under(Activity::Idle, "claude"), Some("Idle"));
    assert_eq!(under(Activity::Idle, "zsh"), None);
}

#[test]
fn how_long_ago_is_said_in_a_word() {
    let ago = |seconds| ago(Duration::from_secs(seconds));
    assert_eq!(ago(0), "now");
    assert_eq!(ago(59), "now");
    assert_eq!(ago(60), "1m");
    assert_eq!(ago(3_599), "59m");
    assert_eq!(ago(7_300), "2h");
    assert_eq!(ago(200_000), "2d");
}
