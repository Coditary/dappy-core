use dap_core::resolve_control_port;
use instance_manager::{SessionRecord, SessionStore};

#[test]
fn resolve_control_port_requires_active_session() {
    let dir = std::env::temp_dir().join(format!(
        "dap-resolve-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let store = SessionStore::open(&dir).expect("open store");
    let err = resolve_control_port(&store, None, None).unwrap_err();
    assert!(err.to_string().contains("no active debug session"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolve_control_port_rejects_ambiguous_sessions() {
    let dir = std::env::temp_dir().join(format!(
        "dap-resolve-amb-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let store = SessionStore::open(&dir).expect("open store");

    let listener_a = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind a");
    let listener_b = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind b");
    let port_a = listener_a.local_addr().expect("addr a").port();
    let port_b = listener_b.local_addr().expect("addr b").port();

    for (port, id) in [(port_a, "a"), (port_b, "b")] {
        store
            .save(&SessionRecord {
                instance_id: id.into(),
                pid: std::process::id(),
                control_port: port,
                adapter_id: "fake".into(),
                program: None,
                scope: None,
                parent_id: None,
                started_at_unix: 0,
            })
            .expect("save");
    }

    let err = resolve_control_port(&store, None, None).unwrap_err();
    assert!(err.to_string().contains("ambiguous"));

    drop(listener_a);
    drop(listener_b);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolve_control_port_honors_explicit_port() {
    let dir = std::env::temp_dir().join(format!(
        "dap-resolve-port-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let store = SessionStore::open(&dir).expect("open store");
    let port = resolve_control_port(&store, Some(4242), None).expect("port");
    assert_eq!(port, 4242);
    let _ = std::fs::remove_dir_all(dir);
}
