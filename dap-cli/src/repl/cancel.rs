use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use tokio::process::Child;
use tokio::task::AbortHandle;

/// Processes to tear down when startup is cancelled.
struct StartupKillTargets {
    adapter_pid: Option<u32>,
    proxy_abort: Option<AbortHandle>,
}

impl StartupKillTargets {
    fn kill_all(&mut self) {
        if let Some(pid) = self.adapter_pid.take() {
            kill_process_tree(pid);
        }
        if let Some(abort) = self.proxy_abort.take() {
            abort.abort();
        }
    }
}

/// Shared kill targets updated as the headless session comes online.
pub struct SharedStartupKill {
    inner: std::sync::Mutex<StartupKillTargets>,
}

impl SharedStartupKill {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(StartupKillTargets {
                adapter_pid: None,
                proxy_abort: None,
            }),
        }
    }

    pub fn register_adapter_process(&self, process: &Child) {
        self.inner
            .lock()
            .expect("startup kill lock")
            .adapter_pid = process.id();
    }

    pub fn set_proxy_abort(&self, abort: AbortHandle) {
        self.inner
            .lock()
            .expect("startup kill lock")
            .proxy_abort = Some(abort);
    }

    pub fn kill_all(&self) {
        self.inner.lock().expect("startup kill lock").kill_all();
    }
}

/// OS-level startup cancellation that does not depend on the Tokio runtime polling `ctrl_c`.
///
/// Tokio's `ctrl_c()` can fail to run while a long adapter handshake blocks worker threads.
/// This uses `signal-hook` atomics for SIGINT/SIGTERM only (no raw TTY mode — that breaks
/// interactive `read_line` at the `dap>` prompt).
pub struct StartupCancel {
    triggered: Arc<AtomicBool>,
    _sigint: signal_hook::SigId,
    _sigterm: signal_hook::SigId,
}

impl StartupCancel {
    pub fn register() -> io::Result<Self> {
        let triggered = Arc::new(AtomicBool::new(false));
        let sigint = flag::register(SIGINT, triggered.clone())?;
        let sigterm = flag::register(SIGTERM, triggered.clone())?;
        Ok(Self {
            triggered,
            _sigint: sigint,
            _sigterm: sigterm,
        })
    }

    pub fn triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    pub async fn wait(&self) {
        while !self.triggered() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pid = pid as i32;
        unsafe {
            let _ = libc::kill(pid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            let _ = libc::kill(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}
