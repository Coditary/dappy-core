use std::sync::Arc;

use anyhow::{Context, Result};
use dap_core::{
    AdapterSpawnOptions, Backend, ControlClient, DapEngine, LaunchOptions, MultiplexOptions,
    SessionInitConfig, SessionInitResult, kill_adapter_process, run_session_init,
    start_headless_multiplexed_proxy, RunningMultiplexedProxy,
};
use tokio::process::Child;

use crate::repl::cancel::SharedStartupKill;

/// A locally spawned headless debug session (control plane only, no dummy editor).
pub struct HeadlessSession {
    pub client: ControlClient,
    pub init: SessionInitResult,
    proxy: RunningMultiplexedProxy,
    adapter_process: Option<Child>,
}

struct SessionStartup {
    adapter_process: Option<Child>,
    proxy: Option<RunningMultiplexedProxy>,
}

impl SessionStartup {
    fn disarm(
        &mut self,
        client: ControlClient,
        init: SessionInitResult,
    ) -> HeadlessSession {
        HeadlessSession {
            client,
            init,
            proxy: std::mem::take(&mut self.proxy).expect("session proxy"),
            adapter_process: std::mem::take(&mut self.adapter_process),
        }
    }
}

impl Drop for SessionStartup {
    fn drop(&mut self) {
        kill_adapter_process(&mut self.adapter_process);
        if let Some(proxy) = &self.proxy {
            proxy.join.abort();
        }
    }
}

impl Drop for HeadlessSession {
    fn drop(&mut self) {
        kill_adapter_process(&mut self.adapter_process);
        self.proxy.join.abort();
    }
}

impl HeadlessSession {
    pub async fn spawn(
        program: &str,
        adapter: Option<&str>,
        startup_kill: Option<Arc<SharedStartupKill>>,
    ) -> Result<Self> {
        let mut startup = SessionStartup {
            adapter_process: None,
            proxy: None,
        };

        let engine = DapEngine::for_headless_session()?;
        let launch = LaunchOptions {
            request: "launch".into(),
            program: Some(program.to_string()),
            adapter: adapter.map(str::to_string),
            extra: serde_json::json!({}),
        };

        let handle = engine.prepare_session(launch.clone()).await?;
        let adapter_spawn = engine
            .resolve_adapter_spawn(&launch)
            .context("resolve adapter for program")?;
        let backend = Backend::spawn_with_options(
            &adapter_spawn,
            AdapterSpawnOptions {
                new_session: true,
            },
        )
        .await
        .context("spawn debug adapter")?;
        let (duplex, adapter_process) = backend.detach_adapter();
        startup.adapter_process = adapter_process;
        if let (Some(kill), Some(process)) = (startup_kill.as_ref(), startup.adapter_process.as_ref())
        {
            kill.register_adapter_process(process);
        }
        eprintln!("  2/3 Initializing DAP session");

        let init_config =
            SessionInitConfig::new(program).with_adapter_id(handle.adapter_id.clone());

        let use_direct_stdio = handle.adapter_id == "python";
        let (client, init) = if use_direct_stdio {
            eprintln!("  3/3 Launching program (direct debugpy)");
            let mut client = ControlClient::from_duplex(duplex);
            let init = run_session_init(&mut client, &init_config)
                .await
                .context("initialize debug session")?;
            startup.proxy = Some(noop_mux_proxy());
            (client, init)
        } else {
            let proxy = start_headless_multiplexed_proxy(
                duplex,
                MultiplexOptions {
                    control_port: Some(0),
                    ..Default::default()
                },
                None,
            )
            .await
            .context("start headless multiplexed proxy")?;
            if let Some(kill) = startup_kill.as_ref() {
                kill.set_proxy_abort(proxy.join.abort_handle());
            }
            startup.proxy = Some(proxy);

            let control_port = startup
                .proxy
                .as_ref()
                .and_then(|proxy| proxy.control_port)
                .context("control port not available")?;
            let mut client = ControlClient::connect(control_port)
                .await
                .context("connect control client")?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            eprintln!("  3/3 Launching program");
            let init = run_session_init(&mut client, &init_config)
                .await
                .context("initialize debug session")?;
            (client, init)
        };

        Ok(startup.disarm(client, init))
    }

    pub async fn shutdown(mut self) -> Result<()> {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.client.disconnect(),
        )
        .await;
        kill_adapter_process(&mut self.adapter_process);
        self.proxy.join.abort();
        Ok(())
    }
}

fn noop_mux_proxy() -> RunningMultiplexedProxy {
    RunningMultiplexedProxy {
        control_port: None,
        join: tokio::spawn(async { Ok(()) }),
    }
}
