use instance_manager::{
    InstanceManager, InstanceSpec, InstanceState, SessionRecord, SessionStore,
    session_record_from_handle,
};

#[tokio::test]
async fn register_and_transition() {
    let manager = InstanceManager::new();
    let spec = InstanceSpec::new("dap-session").with_label("main.py");
    let id = manager.register(spec).await.unwrap();

    assert_eq!(manager.count().await, 1);

    manager
        .set_state(&id, InstanceState::Starting)
        .await
        .unwrap();
    manager
        .set_state(&id, InstanceState::Running)
        .await
        .unwrap();

    let handle = manager.get(&id).await.unwrap();
    assert_eq!(handle.state(), InstanceState::Running);
}

#[tokio::test]
async fn parent_child_and_control_port() {
    let manager = InstanceManager::new();
    let parent = manager
        .register(InstanceSpec::new("dap-session").with_control_port(4711))
        .await
        .unwrap();

    let child_spec = InstanceSpec::new("dap-child").with_parent_id(parent.clone());
    let child = manager.register(child_spec).await.unwrap();

    let parent_handle = manager.get(&parent).await.unwrap();
    assert_eq!(parent_handle.spec.control_port, Some(4711));

    let child_handle = manager.get(&child).await.unwrap();
    assert_eq!(child_handle.spec.parent_id.as_ref(), Some(&parent));
}

#[tokio::test]
async fn reject_invalid_transition() {
    let manager = InstanceManager::new();
    let id = manager
        .register(InstanceSpec::new("lsp-server"))
        .await
        .unwrap();

    let err = manager
        .set_state(&id, InstanceState::Running)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("transition"));
}

#[tokio::test]
async fn list_filters_by_scope_and_remove_instance() {
    let manager = InstanceManager::new();
    let scoped = manager
        .register(InstanceSpec::new("dap-session").with_scope("ws"))
        .await
        .unwrap();
    let _other = manager
        .register(InstanceSpec::new("dap-session"))
        .await
        .unwrap();

    assert_eq!(manager.list(Some("ws")).await.len(), 1);
    manager
        .set_state(&scoped, InstanceState::Starting)
        .await
        .unwrap();
    manager
        .set_state(&scoped, InstanceState::Running)
        .await
        .unwrap();
    manager.set_control_port(&scoped, 9000).await.unwrap();
    manager.remove(&scoped).await.unwrap();
    assert_eq!(manager.count().await, 1);
}

#[tokio::test]
async fn prune_stale_removes_dead_pid_instances() {
    let manager = InstanceManager::new();
    let id = manager
        .register(
            InstanceSpec::new("dap-session")
                .with_pid(9_999_999)
                .with_tag("adapter", "fake"),
        )
        .await
        .unwrap();
    manager
        .set_state(&id, InstanceState::Running)
        .await
        .unwrap();

    let removed = manager.prune_stale().await;
    assert_eq!(removed, vec![id]);
    assert_eq!(manager.count().await, 0);
}

#[tokio::test]
async fn import_from_store_registers_active_sessions() {
    let dir = std::env::temp_dir().join(format!(
        "dap-instance-import-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let store = SessionStore::open(&dir).expect("open store");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let record = SessionRecord {
        instance_id: "imported-session".into(),
        pid: std::process::id(),
        control_port: port,
        adapter_id: "fake".into(),
        program: Some("main.py".into()),
        scope: Some("ws".into()),
        parent_id: None,
        started_at_unix: 1,
    };
    store.save(&record).expect("save");

    let manager = InstanceManager::new();
    let imported = manager.import_from_store(&store).await.expect("import");
    assert_eq!(imported.len(), 1);

    let handle = manager.get(&imported[0]).await.expect("get");
    assert_eq!(handle.spec.control_port, Some(port));
    assert_eq!(handle.spec.label.as_deref(), Some("main.py"));
    assert_eq!(handle.state(), InstanceState::Running);

    let persisted = session_record_from_handle(&handle).expect("record");
    assert_eq!(persisted.instance_id, "imported-session");
    assert_eq!(persisted.adapter_id, "fake");

    let _ = std::fs::remove_dir_all(dir);
}
