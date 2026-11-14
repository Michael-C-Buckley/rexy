use super::*;

#[test]
fn keeps_up_with_structure_titles_and_programs_and_nothing_else() {
    let session = |event: Value| {
        let payload = json!({"session_id": "s", "event": event});
        json!({"type": "event", "payload": {"type": "session_event", "payload": payload}})
    };
    let block = |name: &str, payload: Value| {
        let payload = json!({"block_id": "b", "name": name, "payload": payload});
        session(json!({"type": "block_event", "payload": payload}))
    };
    let structure = session(json!({"type": "session_view_changed", "payload": {}}));
    assert_eq!(event(&structure), Some(Event::Structure));
    let created = json!({"payload": {"type": "session_created"}});
    assert_eq!(event(&created), Some(Event::Structure));
    let title = Event::Title {
        block: "b".into(),
        title: "vim".into(),
    };
    let changed = block("title_changed", json!({"title": "vim"}));
    assert_eq!(event(&changed), Some(title));
    // A program has the icon of its script, of what it was started as, or of its executable.
    let program = |foreground: Value, icon: Option<&str>| {
        let event = event(&block("process_changed", json!({"foreground": foreground})));
        let (block, program) = ("b".into(), Program::named(icon));
        assert_eq!(event, Some(Event::Program { block, program }));
    };
    program(
        json!({"name": "2.1.325", "argv0": "claude"}),
        Some("claude-code"),
    );
    let script = "/opt/lib/pi-coding-agent/dist/cli.js";
    let script = json!({"name": "node", "argv0": "node", "script": script});
    program(script, Some("pi"));
    program(json!({"name": "zsh", "argv0": "-zsh"}), None);
    program(Value::Null, None);
    let status = Some(Event::Status { block: "b".into() });
    assert_eq!(event(&block("program_status_changed", json!({}))), status);
    assert_eq!(event(&block("program_status_removed", json!({}))), status);
    assert_eq!(event(&block("size_changed", json!({}))), None);
    assert_eq!(event(&json!({"payload": {"type": "client_joined"}})), None);
}

#[tokio::test]
async fn nonreading_peer_cannot_stall_a_control_write_forever() {
    let (stream, _nonreading_peer) = tokio::io::duplex(8);
    let io: Box<dyn Io> = Box::new(stream);
    let mut socket = WebSocketStream::from_raw_socket(
        tokio_tungstenite::MaybeTlsStream::Plain(io),
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let error = send_with_deadline(
        &mut socket,
        Message::Text("a".repeat(256).into()),
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("write timed out"));
}
