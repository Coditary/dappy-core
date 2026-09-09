use instance_manager::{SessionRecord, SessionStore};

#[test]
fn save_load_and_delete_session() {
    let dir = std::env::temp_dir().join(format!("dap-sessions-{}", uuid_like()));
    let store = SessionStore::open(&dir).expect("open store");
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind temp port");
    let port = listener.local_addr().expect("local addr").port();

    let record = SessionRecord {
        instance_id: "test-instance".into(),
        pid: std::process::id(),
        control_port: port,
        adapter_id: "fake".into(),
        program: Some("main.py".into()),
        scope: Some("workspace".into()),
        parent_id: None,
        started_at_unix: 1,
    };

    store.save(&record).expect("save");
    let loaded = store.get("test-instance").expect("get").expect("record");
    assert_eq!(loaded, record);

    let active = store.list_active().expect("list");
    assert_eq!(active.len(), 1);

    store.delete("test-instance").expect("delete");
    assert!(store.get("test-instance").expect("get").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stale_pid_is_pruned_on_list() {
    let dir = std::env::temp_dir().join(format!("dap-sessions-{}", uuid_like()));
    let store = SessionStore::open(&dir).expect("open store");

    store
        .save(&SessionRecord {
            instance_id: "dead".into(),
            pid: 999_999_999,
            control_port: 1,
            adapter_id: "fake".into(),
            program: None,
            scope: None,
            parent_id: None,
            started_at_unix: 1,
        })
        .expect("save");

    let active = store.list_active().expect("list");
    assert!(active.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dead_control_port_is_pruned_on_list() {
    let dir = std::env::temp_dir().join(format!("dap-sessions-{}", uuid_like()));
    let store = SessionStore::open(&dir).expect("open store");

    store
        .save(&SessionRecord {
            instance_id: "dead-port".into(),
            pid: std::process::id(),
            control_port: 9,
            adapter_id: "fake".into(),
            program: None,
            scope: None,
            parent_id: None,
            started_at_unix: 1,
        })
        .expect("save");

    let active = store.list_active().expect("list");
    assert!(active.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_session_file_is_removed_on_list() {
    let dir = std::env::temp_dir().join(format!("dap-sessions-{}", uuid_like()));
    let store = SessionStore::open(&dir).expect("open store");
    let path = dir.join("broken.json");
    std::fs::write(&path, "{not json").expect("write corrupt file");

    let active = store.list_active().expect("list");
    assert!(active.is_empty());
    assert!(!path.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

fn uuid_like() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
