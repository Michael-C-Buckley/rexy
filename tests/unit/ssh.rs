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

#[test]
fn the_servers_socket_is_where_its_own_rex_found_it() {
    use serde_json::json;
    let (first, second) = ("/run/user/1000/rex/server.sock", "/run/rex/server.sock");
    let listeners = json!([
        {"endpoint": "127.0.0.1:0", "addresses": ["127.0.0.1:52061"], "transport": "tcp"},
        {"endpoint": "unix:///run/rex/server.sock", "addresses": [second],
         "transport": "unix", "primary": true},
    ]);
    let socket = |status: Value| remote_socket(&status);
    let both = json!({"endpoint": first, "listeners": listeners});
    assert_eq!(socket(both).as_deref(), Some(first));
    // Rex writes a socket as a URL, its path percent-encoded.
    let mac = "unix:///Users/me/Library/Application%20Support/rex/server.sock";
    let path = "/Users/me/Library/Application Support/rex/server.sock";
    assert_eq!(socket(json!({"endpoint": mac})).as_deref(), Some(path));
    // An endpoint that is no socket leaves it to the list.
    let listed = json!({"endpoint": "127.0.0.1:7297", "listeners": listeners});
    assert_eq!(socket(listed).as_deref(), Some(second));
    let unnamed = json!({"listeners": listeners});
    assert_eq!(socket(unnamed).as_deref(), Some(second));
    assert_eq!(socket(json!({"endpoint": "127.0.0.1:7297"})), None);
    assert_eq!(socket(json!({"endpoint": "unix:here.sock"})), None);
    assert_eq!(socket(json!({"pid": 4242})), None);
}

#[tokio::test]
async fn what_ssh_says_is_read_to_its_end_and_the_last_of_it_kept() {
    let mut said = b"Warning: something\n".repeat(1000);
    said.extend(b"Timeout, server studio not responding.\n");
    let kept = drain(&said[..]).await;
    assert_eq!(kept.len(), KEPT);
    assert_eq!(reason(&kept), "Timeout, server studio not responding.");
    assert_eq!(drain(&b""[..]).await, b"");
}

#[test]
fn the_directory_for_sockets_is_this_users_own() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no arguments and cannot fail.
    let user = unsafe { libc::geteuid() };
    let name = format!("rex-lite-test-{}", std::process::id());
    let root = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&root);
    let (own, link, file) = (root.join("own"), root.join("link"), root.join("file"));
    private(&own, user).unwrap();
    let mode = std::fs::metadata(&own).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    // There already, it will do.
    private(&own, user).unwrap();
    // But not if it is another's, or only a link to a directory, or no directory.
    assert!(private(&own, user + 1).is_err());
    std::os::unix::fs::symlink(&own, &link).unwrap();
    assert!(private(&link, user).is_err());
    std::fs::write(&file, "").unwrap();
    assert!(private(&file, user).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
