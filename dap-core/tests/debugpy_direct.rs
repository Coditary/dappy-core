use std::time::Duration;

use dap_core::{
    Backend, ControlClient, SessionInitConfig, run_session_init,
};
use dap_plugin_api::{AdapterSpawn, SpawnTransport};

#[tokio::test]
async fn debugpy_direct_session_init_when_available() {
    if !debugpy_available() {
        eprintln!("skipping debugpy_direct_session_init: debugpy not installed");
        return;
    }

    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: "python3".into(),
        args: vec!["-m".into(), "debugpy.adapter".into()],
    })
    .await
    .expect("spawn debugpy adapter");

    let (duplex, _child) = backend.detach_adapter();
    let mut client = ControlClient::from_duplex(duplex);

    let program = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/fixtures/main.py");
    let program = program
        .canonicalize()
        .unwrap_or_else(|_| program);

    let result = tokio::time::timeout(
        Duration::from_secs(30),
        run_session_init(
            &mut client,
            &SessionInitConfig::new(program.display().to_string()).with_adapter_id("python"),
        ),
    )
    .await;
    match result {
        Ok(Ok(init)) => {
            assert!(init.ready);
            assert!(
                matches!(init.stop_reason.as_deref(), Some("entry") | Some("breakpoint")),
                "unexpected stop reason: {:?}",
                init.stop_reason
            );
        }
        Ok(Err(err)) => panic!("debugpy session init failed: {err:#}"),
        Err(_) => panic!("debugpy session init timed out after 30s"),
    }
}

fn debugpy_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import debugpy"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
