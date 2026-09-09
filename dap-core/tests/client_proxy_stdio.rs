use dap_core::{
    ProxyStdioOptions, SessionInitConfig, run_session_init, spawn_proxy_stdio,
};

#[tokio::test]
async fn spawn_proxy_stdio_runs_session_init_with_fake_adapter() {
    let adapter = adapter_bin();

    let mut opts = ProxyStdioOptions::program("main.py")
        .with_adapter_cmd(vec![adapter.to_string_lossy().into_owned()])
        .with_adapter("fake");
    opts.proxy_bin = Some(proxy_bin());
    let mut session = spawn_proxy_stdio(opts)
        .await
        .expect("spawn dap-proxy");

    let result = run_session_init(
        &mut session.client,
        &SessionInitConfig::new("main.py").with_adapter_id("fake"),
    )
    .await
    .expect("session init");

    assert!(result.ready);
    assert_eq!(result.stop_reason.as_deref(), Some("entry"));

    let _ = session.child.kill().await;
}

fn adapter_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("fake-dap-adapter")
        })
}

#[allow(dead_code)]
fn proxy_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_dap-proxy")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("dap-proxy")
        })
}
