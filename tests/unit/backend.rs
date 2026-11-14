use super::*;

#[test]
fn nested_split_geometry_respects_server_ratios() {
    let layout = json!({"split": {
        "direction": "horizontal",
        "ratio": 0.6,
        "before": {"block_id": "a"},
        "after": {"split": {
            "direction": "vertical",
            "ratio": 0.25,
            "before": {"block_id": "b"},
            "after": {"block_id": "c"},
        }},
    }});
    let view = json!({"windows": [{"window_id": "w", "layers": [{"layout": layout}]}]});
    let windows = parse_windows(&view, "me");
    let panes = &windows[0].panes;
    assert_eq!(panes.len(), 3);
    assert!((panes[1].x - 0.6).abs() < 0.001);
    assert!((panes[2].height - 0.75).abs() < 0.001);
    assert!((panes.iter().map(|p| p.width * p.height).sum::<f32>() - 1.).abs() < 0.001);
}

#[test]
fn zoom_and_minimized_layers_are_respected() {
    let view = json!({"windows": [{"window_id": "w", "zoomed_block_id": "b", "layers": [
        {
            "layout": {"block_id": "b"},
            "bounds": [0.2, 0.2, 0.5, 0.5],
            "blocks": [{"block_id": "b", "owner": "me"}],
        },
        {"state": "minimized", "layout": {"block_id": "hidden"}},
    ]}]});
    let windows = parse_windows(&view, "me");
    assert_eq!(windows[0].panes.len(), 1);
    assert_eq!(windows[0].panes[0].width, 1.);
    assert!(windows[0].panes[0].owner);
}

#[test]
fn control_and_release_keys_never_duplicate_text() {
    assert_eq!(key_payload("c", "key_c", model::CTRL, true)["text"], "");
    assert_eq!(key_payload("A", "key_a", model::SHIFT, true)["text"], "A");
    assert_eq!(key_payload("a", "key_a", 0, false)["text"], "");
}

#[test]
fn notifications_coalesce_until_snapshot_acknowledges() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    let wake = Wake {
        pending: AtomicBool::new(false),
        callback: Arc::new(move || {
            seen.fetch_add(1, Ordering::Relaxed);
        }),
    };
    for _ in 0..100 {
        wake.notify();
    }
    assert_eq!(count.load(Ordering::Relaxed), 1);
    wake.pending.store(false, Ordering::Release);
    wake.notify();
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

fn client() -> (Client, mpsc::Receiver<Action>) {
    let (actions, queue) = mpsc::channel(256);
    let shared = Shared {
        state: Arc::default(),
        wake: Arc::new(Wake {
            pending: AtomicBool::new(false),
            callback: Arc::new(|| {}),
        }),
    };
    (Client { actions, shared }, queue)
}

#[test]
fn input_is_accepted_only_while_connected() {
    let (client, mut queue) = client();
    assert!(client.try_send(Action::CreateWindow).is_err());
    client.try_send(Action::Reconnect).unwrap();
    client.shared.status("Connected", true);
    client.try_send(Action::CreateWindow).unwrap();
    assert!(matches!(queue.try_recv(), Ok(Action::Reconnect)));
    assert!(matches!(queue.try_recv(), Ok(Action::CreateWindow)));
    assert!(queue.try_recv().is_err());
}

#[test]
fn input_racing_a_disconnect_never_outlives_it() {
    let (client, mut queue) = client();
    let client = Arc::new(client);
    let stop = Arc::new(AtomicBool::new(false));
    let typing = {
        let (client, stop) = (client.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = client.try_send(Action::CreateWindow);
            }
        })
    };
    for _ in 0..20_000 {
        client.shared.status("Connected", true);
        std::thread::yield_now();
        // What `run` does when a connection fails.
        client.shared.status("Disconnected", false);
        while queue.try_recv().is_ok() {}
        // Input accepted before the failure is gone, and none may follow it.
        std::thread::yield_now();
        assert!(queue.try_recv().is_err(), "input outlived its connection");
    }
    stop.store(true, Ordering::Relaxed);
    typing.join().unwrap();
}
