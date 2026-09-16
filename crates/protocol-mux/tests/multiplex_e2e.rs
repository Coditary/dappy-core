use protocol_mux::{ClientEndpoint, ClientRole, MultiplexSession, event, request, response};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn editor_and_control_share_events() {
    let (to_upstream, mut from_mux) = mpsc::unbounded_channel();
    let (to_mux, from_upstream) = mpsc::unbounded_channel();

    let (session, join) = MultiplexSession::start(from_upstream, to_upstream);

    let mut editor = session.attach(ClientRole::Editor).await.unwrap();
    let mut control = session.attach(ClientRole::Control).await.unwrap();

    let to_mux_for_upstream = to_mux.clone();
    let upstream = tokio::spawn(async move {
        while let Some(msg) = from_mux.recv().await {
            let backend_seq = msg["seq"].as_i64().unwrap();
            let command = msg["command"].as_str().unwrap();
            match command {
                "initialize" => {
                    let _ = to_mux_for_upstream.send(response(1, backend_seq, true, None));
                    let _ = to_mux_for_upstream.send(event(2, "initialized", None));
                }
                "setBreakpoints" => {
                    let _ = to_mux_for_upstream.send(response(3, backend_seq, true, None));
                }
                _ => {}
            }
        }
    });

    editor
        .send(request(1, "initialize", None))
        .expect("send initialize");
    let init_resp = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("timeout")
        .expect("initialize response");
    assert_eq!(init_resp["request_seq"], 1);

    let initialized = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("timeout")
        .expect("initialized event");
    assert_eq!(initialized["event"], "initialized");

    let stopped = event(10, "stopped", Some(json!({ "reason": "breakpoint" })));
    to_mux.send(stopped).expect("send stopped");

    let editor_stopped = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("timeout")
        .expect("editor stopped");
    assert_eq!(editor_stopped["event"], "stopped");

    let control_stopped = recv_broadcast_event_named(&mut control, "stopped").await;
    assert_eq!(control_stopped["event"], "stopped");

    control
        .send(request(42, "setBreakpoints", None))
        .expect("send breakpoints");
    let bp_resp = timeout(Duration::from_secs(1), control.recv_message())
        .await
        .expect("timeout")
        .expect("breakpoints response");
    assert_eq!(bp_resp["success"], true);
    assert_eq!(bp_resp["request_seq"], 42);

    let editor_timeout = timeout(Duration::from_millis(50), editor.recv_message()).await;
    assert!(
        editor_timeout.is_err(),
        "editor should not receive control response"
    );

    upstream.abort();
    join.abort();
}

#[tokio::test]
async fn late_join_replays_cached_events() {
    let (to_upstream, mut from_mux) = mpsc::unbounded_channel();
    let (to_mux, from_upstream) = mpsc::unbounded_channel();

    let (session, join) = MultiplexSession::start(from_upstream, to_upstream);

    let mut editor = session.attach(ClientRole::Editor).await.unwrap();

    let to_mux_upstream = to_mux.clone();
    let upstream = tokio::spawn(async move {
        if let Some(msg) = from_mux.recv().await {
            let backend_seq = msg["seq"].as_i64().unwrap();
            let _ = to_mux_upstream.send(response(1, backend_seq, true, None));
            let _ = to_mux_upstream.send(event(2, "initialized", None));
            let _ = to_mux_upstream.send(event(3, "stopped", Some(json!({ "reason": "entry" }))));
        }
    });

    editor.send(request(1, "initialize", None)).unwrap();
    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("init response timeout")
        .expect("init response");
    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("initialized timeout")
        .expect("initialized");
    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("stopped timeout")
        .expect("stopped");

    let mut late = session.attach(ClientRole::Control).await.unwrap();
    let replay_initialized = timeout(Duration::from_secs(1), late.recv_message())
        .await
        .expect("replay initialized timeout")
        .expect("replay initialized");
    assert_eq!(replay_initialized["event"], "initialized");

    let replay_stopped = timeout(Duration::from_secs(1), late.recv_message())
        .await
        .expect("replay stopped timeout")
        .expect("replay stopped");
    assert_eq!(replay_stopped["event"], "stopped");

    upstream.abort();
    join.abort();
}

#[tokio::test]
async fn control_can_fetch_breakpoint_snapshot() {
    let (to_upstream, mut from_mux) = mpsc::unbounded_channel();
    let (to_mux, from_upstream) = mpsc::unbounded_channel();

    let (session, join) = MultiplexSession::start(from_upstream, to_upstream);

    let mut editor = session.attach(ClientRole::Editor).await.unwrap();
    let mut control = session.attach(ClientRole::Control).await.unwrap();

    let to_mux_upstream = to_mux.clone();
    let upstream = tokio::spawn(async move {
        while let Some(msg) = from_mux.recv().await {
            let backend_seq = msg["seq"].as_i64().unwrap();
            let command = msg["command"].as_str().unwrap();
            if command == "setBreakpoints" {
                let _ = to_mux_upstream.send(response(
                    backend_seq + 100,
                    backend_seq,
                    true,
                    Some(json!({
                        "breakpoints": [{ "verified": true, "line": 10 }]
                    })),
                ));
            }
        }
    });

    editor
        .send(request(
            1,
            "setBreakpoints",
            Some(json!({
                "source": { "path": "/fake/main.py" },
                "breakpoints": [{ "line": 10, "condition": "x > 1" }],
            })),
        ))
        .unwrap();

    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("setBreakpoints response timeout")
        .expect("setBreakpoints response");

    control
        .send(request(42, protocol_mux::BREAKPOINT_SNAPSHOT_COMMAND, None))
        .expect("snapshot request");
    let snapshot = timeout(Duration::from_secs(1), control.recv_message())
        .await
        .expect("snapshot timeout")
        .expect("snapshot response");
    assert_eq!(snapshot["success"], true);
    assert_eq!(snapshot["request_seq"], 42);
    assert_eq!(
        snapshot["body"]["breakpoints"]["/fake/main.py"][0]["line"],
        10
    );
    assert_eq!(
        snapshot["body"]["breakpoints"]["/fake/main.py"][0]["condition"],
        "x > 1"
    );

    upstream.abort();
    join.abort();
}

#[tokio::test]
async fn cancel_translation_on_editor_path() {
    let (to_upstream, mut from_mux) = mpsc::unbounded_channel();
    let (to_mux, from_upstream) = mpsc::unbounded_channel();

    let (session, join) = MultiplexSession::start(from_upstream, to_upstream);
    let mut editor = session.attach(ClientRole::Editor).await.unwrap();

    let upstream = tokio::spawn(async move {
        while let Some(msg) = from_mux.recv().await {
            let backend_seq = msg["seq"].as_i64().unwrap();
            let command = msg["command"].as_str().unwrap();
            if command == "longRunning" {
                let _ = to_mux.send(response(1, backend_seq, true, None));
            } else if command == "cancel" {
                let cancelled = msg["arguments"]["requestId"].as_i64().unwrap();
                assert_eq!(cancelled, 1, "cancel should reference backend seq");
                let _ = to_mux.send(response(2, backend_seq, true, None));
                break;
            }
        }
    });

    editor.send(request(10, "longRunning", None)).unwrap();
    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("longRunning response timeout");

    editor
        .send(request(11, "cancel", Some(json!({ "requestId": 10 }))))
        .unwrap();
    let _ = timeout(Duration::from_secs(1), editor.recv_message())
        .await
        .expect("cancel response timeout");

    upstream.abort();
    join.abort();
}

async fn recv_broadcast_event_named(client: &mut ClientEndpoint, name: &str) -> serde_json::Value {
    loop {
        let msg = timeout(Duration::from_secs(1), client.recv_event())
            .await
            .expect("timeout")
            .expect("broadcast message");
        if msg["type"] == "event" && msg["event"] == name {
            return (*msg).clone();
        }
    }
}
