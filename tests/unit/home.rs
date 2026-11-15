use super::*;

#[test]
fn home_is_the_one_rex_follows() {
    let file = || Some(r#"{"endpoint": "ssh://studio", "host_id": "0e9d"}"#.to_owned());
    let named = || Some("attic".to_owned());
    // REX_HOME comes before the home that was chosen with `rex home set`.
    assert_eq!(home(named(), file()).as_deref(), Some("attic"));
    assert_eq!(home(None, file()).as_deref(), Some("ssh://studio"));
    assert_eq!(home(Some(String::new()), file()).as_deref(), Some("ssh://studio"));
    // With neither, and with a file that names none, home is this machine.
    assert_eq!(home(None, None), None);
    assert_eq!(home(None, Some("{}".into())), None);
    assert_eq!(home(None, Some("not JSON".into())), None);
}
