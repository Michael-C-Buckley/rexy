use super::*;

#[test]
fn ignores_terminal_output_but_tracks_structural_changes() {
    let event = |kind| {
        let payload = json!({"session_id": "s", "event": {"type": kind, "payload": {}}});
        json!({"type": "event", "payload": {"type": "session_event", "payload": payload}})
    };
    assert!(structure_changed(&event("session_view_changed")));
    assert!(!structure_changed(&event("block_event")));
    assert!(structure_changed(
        &json!({"payload":{"type":"session_created"}})
    ));
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
