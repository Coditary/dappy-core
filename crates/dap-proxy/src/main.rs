use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use dap_core::{
    AdapterGuard, Backend, ChildSessionBridge, ChildSessionConfig, ChildSessionProfile,
    ControlClient, DapEngine, LaunchOptions, MultiplexOptions, ParentBackendKind,
    ParentSessionContext, ProxyPluginContext, SessionInitConfig, TargetGuard,
    arm_parent_death_signal, merge_default_attach_arguments, prepare_rsp_target, run_debug_request,
    spawn_parent_watch, start_headless_multiplexed_proxy, start_multiplexed_proxy,
};
use dap_plugin_api::{AdapterSpawn, SpawnTransport};
use dap_protocol::DuplexChannel;
use dap_proxy::{
    ChildProcessSupervisor, ProxyChildSpawner, SessionCleanupGuard, now_unix, resolve_adapter_spawn,
};
use instance_manager::{SessionRecord, SessionStore};
use serde_json::Value;
use tracing::info;

#[derive(Parser)]
#[command(name = "dap-proxy", about = "DAP gateway proxy (stdio)")]
struct Cli {
    /// Speak DAP over stdin/stdout.
    #[arg(long)]
    stdio: bool,

    /// Run without an editor client; drive the adapter via the control plane.
    #[arg(long, conflicts_with = "stdio")]
    headless: bool,

    /// Builtin adapter plugin id (from dap-plugins/builtin/<id>/plugin.yaml). Ignored when `--program` triggers routing.
    #[arg(long)]
    adapter: Option<String>,

    /// Override adapter executable (first token is command, rest are args).
    #[arg(long, num_args = 1..)]
    adapter_cmd: Option<Vec<String>>,

    /// Test helper: append `--emit-start-debugging` to the adapter command.
    #[arg(long, hide = true)]
    fake_emit_start_debugging: bool,

    /// Program path used for adapter routing (e.g. `main.py` → python plugin).
    #[arg(long)]
    program: Option<String>,

    /// RSP target id from `target.yaml` (e.g. gdbserver, qemu-x86-kernel, rsp-attach).
    #[arg(long)]
    target: Option<String>,

    /// Plugin root directory (`<id>/plugin.yaml` or flat `*.yaml` manifests).
    #[arg(long, env = "DAP_PLUGINS_DIR")]
    plugins_dir: Option<std::path::PathBuf>,

    /// TCP port for control-plane attach (`0` = ephemeral). Omit with `--no-control`.
    #[arg(long, default_value_t = 0)]
    control_port: u16,

    /// Disable the control attach listener (editor-only mode).
    #[arg(long)]
    no_control: bool,

    /// Optional scope id for session discovery (also `DAP_SCOPE_ID`).
    #[arg(long, env = "DAP_SCOPE_ID")]
    scope: Option<String>,

    /// Spawn child sessions for adapter `startDebugging` reverse requests.
    #[arg(long)]
    child_sessions: bool,

    /// Max descendant generations for child sessions.
    #[arg(long, default_value_t = 1)]
    child_max_depth: u32,

    /// Max concurrent direct child sessions.
    #[arg(long, default_value_t = 16)]
    child_max_children: u32,

    /// Child-session profile preset (`fake`, `debugpy`, `lldb-dap`).
    #[arg(long, default_value = "fake")]
    child_profile: String,

    /// Child-session profile JSON file (overrides `--child-profile`).
    #[arg(long)]
    child_profile_file: Option<std::path::PathBuf>,

    /// Connect to a TCP debug adapter at `host:port` instead of spawning stdio.
    #[arg(long, conflicts_with = "adapter_cmd")]
    adapter_tcp: Option<String>,

    /// Parent instance id when this proxy was spawned as a child session.
    #[arg(long)]
    parent_id: Option<String>,

    /// Remaining child-session depth budget for this proxy.
    #[arg(long, default_value_t = 0)]
    child_depth: u32,

    /// Close the editor DAP client after this many seconds without inbound messages (`0` = disabled).
    #[arg(long, default_value_t = 0)]
    client_idle_timeout_secs: u64,

    /// DAP request used to start debugging in headless mode (`launch` or `attach`).
    #[arg(long)]
    debug_request: Option<String>,

    /// JSON arguments for `--debug-request`.
    #[arg(long, default_value = "{}")]
    debug_args: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    arm_parent_death_signal();
    if cli.headless {
        return run_headless(cli).await;
    }
    if !cli.stdio {
        anyhow::bail!("editor mode requires --stdio");
    }
    run_editor_proxy(cli).await
}

async fn run_editor_proxy(cli: Cli) -> Result<()> {
    let mut engine = DapEngine::new();
    if let Some(dir) = &cli.plugins_dir {
        engine.load_plugins_from_dir(dir)?;
    } else {
        engine.load_builtin_plugins()?;
    }

    let launch = build_launch_options(&cli)?;
    let mut target_guard = TargetGuard::new(None);
    let attach_defaults = if let Some(target_id) = &cli.target {
        let program = launch
            .program
            .as_deref()
            .context("--target requires --program")?;
        let prepared = prepare_rsp_target(&engine, target_id, program).await?;
        target_guard = TargetGuard::new(prepared.process);
        Some(prepared.attach_arguments)
    } else {
        None
    };

    let adapter_spawn = resolve_backend_spawn(&cli, &engine, &launch)?;
    let adapter_cmd = adapter_cmd_from_spawn(&adapter_spawn, cli.adapter_cmd.as_deref());
    let backend = Backend::spawn(&adapter_spawn).await?;

    let route_launch = LaunchOptions {
        request: launch.request.clone(),
        program: launch.program.clone(),
        adapter: launch.adapter.clone(),
        extra: launch.extra.clone(),
    };
    let session = engine.prepare_session(route_launch).await?;
    engine.mark_running(&session).await?;

    let mut mux_options = build_mux_options(&cli);
    mux_options.plugin_context = engine
        .registry
        .get(&session.adapter_id)
        .cloned()
        .map(ProxyPluginContext::new);
    mux_options.attach_defaults = attach_defaults;

    if let Some(port) = mux_options.control_port {
        info!(
            adapter = %session.adapter_id,
            target = cli.target.as_deref().unwrap_or("none"),
            instance = %session.instance_id,
            requested_control_port = port,
            "dap-proxy starting multiplexed session"
        );
    } else {
        info!(
            adapter = %session.adapter_id,
            instance = %session.instance_id,
            "dap-proxy starting multiplexed session (no control port)"
        );
    }

    let (child_sessions, child_supervisor) =
        build_child_session_bridge(&cli, &session.instance_id.to_string(), &adapter_cmd)?;

    let editor = DuplexChannel::from_stdio();
    let (backend_duplex, mut adapter_guard) = AdapterGuard::from_backend(backend);
    let proxy =
        start_multiplexed_proxy(editor, backend_duplex, mux_options, child_sessions).await?;

    let session_store = SessionStore::open(SessionStore::default_dir())?;
    let mut cleanup = SessionCleanupGuard::new(session_store);
    if let Some(port) = proxy.control_port {
        let _ = engine.set_control_port(&session, port).await?;
        eprintln!(
            r#"{{"controlPort":{},"instanceId":"{}"}}"#,
            port, session.instance_id
        );

        cleanup.save(&SessionRecord {
            instance_id: session.instance_id.to_string(),
            pid: std::process::id(),
            control_port: port,
            adapter_id: session.adapter_id.clone(),
            program: cli.program.clone(),
            scope: cli.scope.clone(),
            parent_id: cli.parent_id.clone(),
            started_at_unix: now_unix(),
        })?;
        cleanup.arm(session.instance_id.to_string());
    }

    let parent_shutdown = spawn_parent_watch(Duration::from_secs(1));
    let run_result = run_until_shutdown(proxy.join, parent_shutdown).await;
    if let Some(supervisor) = child_supervisor {
        supervisor.teardown().await;
    }
    run_result??;
    adapter_guard.shutdown().await;
    target_guard.shutdown().await;
    Ok(())
}

async fn run_headless(cli: Cli) -> Result<()> {
    let debug_request = cli
        .debug_request
        .clone()
        .context("--headless requires --debug-request")?;
    let mut debug_args =
        serde_json::from_str::<Value>(&cli.debug_args).context("parse --debug-args as JSON")?;

    let mut engine = DapEngine::new();
    if let Some(dir) = &cli.plugins_dir {
        engine.load_plugins_from_dir(dir)?;
    } else {
        engine.load_builtin_plugins()?;
    }

    let launch = build_launch_options(&cli)?;
    let mut target_guard = TargetGuard::new(None);
    let attach_defaults = if let Some(target_id) = &cli.target {
        let program = launch
            .program
            .as_deref()
            .context("--target requires --program")?;
        let prepared = prepare_rsp_target(&engine, target_id, program).await?;
        target_guard = TargetGuard::new(prepared.process);
        merge_default_attach_arguments(&mut debug_args, &prepared.attach_arguments);
        Some(prepared.attach_arguments)
    } else {
        None
    };

    let launch = LaunchOptions {
        request: debug_request.clone(),
        program: launch.program.clone(),
        adapter: launch.adapter.clone(),
        extra: debug_args.clone(),
    };

    let adapter_spawn = resolve_backend_spawn(&cli, &engine, &launch)?;
    let adapter_cmd = adapter_cmd_from_spawn(&adapter_spawn, cli.adapter_cmd.as_deref());
    let backend = Backend::spawn(&adapter_spawn).await?;

    let route_launch = LaunchOptions {
        request: launch.request.clone(),
        program: launch.program.clone(),
        adapter: launch.adapter.clone(),
        extra: launch.extra.clone(),
    };
    let session = engine.prepare_session(route_launch).await?;
    engine.mark_running(&session).await?;

    let mut mux_options = build_mux_options(&cli);
    mux_options.plugin_context = engine
        .registry
        .get(&session.adapter_id)
        .cloned()
        .map(ProxyPluginContext::new);
    mux_options.attach_defaults = attach_defaults;

    let (child_sessions, child_supervisor) =
        build_child_session_bridge(&cli, &session.instance_id.to_string(), &adapter_cmd)?;

    let (backend_duplex, mut adapter_guard) = AdapterGuard::from_backend(backend);
    let proxy =
        start_headless_multiplexed_proxy(backend_duplex, mux_options, child_sessions).await?;

    let control_port = proxy
        .control_port
        .context("headless proxy requires a control port")?;
    let mut client = ControlClient::connect(control_port)
        .await
        .context("connect headless control client")?;

    let mut init_config =
        SessionInitConfig::new(cli.program.clone().unwrap_or_else(|| "child".into()))
            .with_adapter_id(session.adapter_id.clone());
    if let Some(manifest) = engine.registry.get(&session.adapter_id).cloned() {
        init_config = init_config.with_manifest(manifest);
    }
    run_debug_request(&mut client, &init_config, &debug_request, &debug_args)
        .await
        .context("headless debug request")?;

    let session_store = SessionStore::open(SessionStore::default_dir())?;
    let mut cleanup = SessionCleanupGuard::new(session_store);
    eprintln!(
        r#"{{"controlPort":{},"instanceId":"{}"}}"#,
        control_port, session.instance_id
    );
    cleanup.save(&SessionRecord {
        instance_id: session.instance_id.to_string(),
        pid: std::process::id(),
        control_port,
        adapter_id: session.adapter_id.clone(),
        program: cli.program.clone(),
        scope: cli.scope.clone(),
        parent_id: cli.parent_id.clone(),
        started_at_unix: now_unix(),
    })?;
    cleanup.arm(session.instance_id.to_string());

    let parent_shutdown = spawn_parent_watch(Duration::from_secs(1));
    let run_result = run_until_shutdown(proxy.join, parent_shutdown).await;
    if let Some(supervisor) = child_supervisor {
        supervisor.teardown().await;
    }
    run_result??;
    adapter_guard.shutdown().await;
    target_guard.shutdown().await;
    Ok(())
}

fn build_launch_options(cli: &Cli) -> Result<LaunchOptions> {
    if cli.target.is_some() && cli.program.is_none() {
        anyhow::bail!("--target requires --program");
    }
    Ok(LaunchOptions {
        request: "launch".into(),
        program: cli.program.clone(),
        adapter: cli
            .adapter
            .clone()
            .or_else(|| cli.target.as_ref().map(|_| "gdb-remote".to_string())),
        extra: serde_json::json!({}),
    })
}

fn build_mux_options(cli: &Cli) -> MultiplexOptions {
    MultiplexOptions {
        control_port: if cli.headless {
            Some(cli.control_port)
        } else if cli.no_control {
            None
        } else {
            Some(cli.control_port)
        },
        client_idle_timeout: if cli.stdio && cli.client_idle_timeout_secs > 0 {
            Some(Duration::from_secs(cli.client_idle_timeout_secs))
        } else {
            None
        },
        ..Default::default()
    }
}

fn build_child_session_bridge(
    cli: &Cli,
    parent_instance_id: &str,
    parent_adapter_cmd: &[String],
) -> Result<(
    Option<ChildSessionBridge>,
    Option<Arc<ChildProcessSupervisor>>,
)> {
    let remaining_depth = if cli.headless {
        cli.child_depth
    } else {
        cli.child_max_depth
    };
    if remaining_depth == 0 {
        return Ok((None, None));
    }
    if !cli.headless && !cli.child_sessions {
        return Ok((None, None));
    }

    let config = ChildSessionConfig {
        auto_spawn: true,
        max_children: cli.child_max_children,
        max_depth: remaining_depth,
        profile: load_child_profile(cli)?,
    };
    let supervisor = Arc::new(ChildProcessSupervisor::new(
        parent_instance_id.to_string(),
        cli.scope.clone(),
        cli.child_profile.clone(),
        cli.child_profile_file.clone(),
        cli.child_max_children,
    )?);
    let spawner: Arc<dyn dap_core::ChildSessionSpawner> =
        Arc::new(ProxyChildSpawner::new(supervisor.clone()));
    Ok((
        Some(ChildSessionBridge {
            config,
            remaining_depth,
            parent: parent_context(cli, parent_adapter_cmd.to_vec()),
            spawner,
            active_children: Arc::new(AtomicU32::new(0)),
        }),
        Some(supervisor),
    ))
}

fn load_child_profile(cli: &Cli) -> Result<ChildSessionProfile> {
    if let Some(path) = &cli.child_profile_file {
        let body = std::fs::read_to_string(path)
            .with_context(|| format!("read child profile file {}", path.display()))?;
        let value = serde_json::from_str(&body).context("parse child profile file as JSON")?;
        return ChildSessionProfile::from_json_value(value).map_err(anyhow::Error::msg);
    }
    ChildSessionProfile::from_preset(&cli.child_profile).with_context(|| {
        format!(
            "unknown child profile preset '{}'; expected fake, debugpy, or lldb-dap",
            cli.child_profile
        )
    })
}

fn parent_context(cli: &Cli, adapter_cmd: Vec<String>) -> ParentSessionContext {
    ParentSessionContext {
        backend: if cli.adapter_tcp.is_some() {
            ParentBackendKind::Tcp
        } else {
            ParentBackendKind::Stdio
        },
        adapter_cmd,
        tcp_endpoint: cli.adapter_tcp.clone(),
    }
}

fn resolve_backend_spawn(
    cli: &Cli,
    engine: &DapEngine,
    launch: &LaunchOptions,
) -> Result<AdapterSpawn> {
    if let Some(endpoint) = &cli.adapter_tcp {
        return Ok(AdapterSpawn {
            transport: SpawnTransport::Tcp,
            command: endpoint.clone(),
            args: vec![],
        });
    }
    let mut spawn = resolve_adapter_spawn(cli.adapter_cmd.as_deref(), engine, launch)?;
    if cli.fake_emit_start_debugging {
        spawn.args.push("--emit-start-debugging".into());
    }
    Ok(spawn)
}

fn adapter_cmd_from_spawn(
    spawn: &dap_plugin_api::AdapterSpawn,
    explicit: Option<&[String]>,
) -> Vec<String> {
    if let Some(cmd) = explicit {
        return cmd.to_vec();
    }
    let mut parts = vec![spawn.command.clone()];
    parts.extend(spawn.args.clone());
    parts
}

async fn run_until_shutdown(
    join: tokio::task::JoinHandle<Result<()>>,
    parent_shutdown: Arc<tokio::sync::Notify>,
) -> Result<Result<()>> {
    let join_abort = join.abort_handle();

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let run_result = tokio::select! {
        result = join => result,
        _ = parent_shutdown.notified() => {
            info!("parent process exited, shutting down dap-proxy");
            join_abort.abort();
            Ok(Ok(()))
        }
        _ = tokio::signal::ctrl_c() => {
            info!("received SIGINT, shutting down dap-proxy");
            join_abort.abort();
            Ok(Ok(()))
        }
        _ = async {
            #[cfg(unix)]
            {
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => {
            info!("received SIGTERM, shutting down dap-proxy");
            join_abort.abort();
            Ok(Ok(()))
        }
    };

    Ok(run_result?)
}
