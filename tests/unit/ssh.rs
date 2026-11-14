use super::*;

#[test]
fn an_endpoint_splits_into_a_destination_and_an_optional_socket() {
    assert_eq!(parse("ssh://studio"), ("ssh://studio", None));
    assert_eq!(
        parse("ssh://me@studio:2222"),
        ("ssh://me@studio:2222", None)
    );
    assert_eq!(
        parse("ssh://me@studio/run/rex/server.sock"),
        ("ssh://me@studio", Some("/run/rex/server.sock"))
    );
}

#[test]
fn the_reason_is_the_last_thing_ssh_said() {
    let stderr = b"Warning: something\nme@studio: Permission denied (publickey).\n\n";
    assert_eq!(reason(stderr), "me@studio: Permission denied (publickey).");
    assert_eq!(reason(b""), "ssh gave no reason");
}
