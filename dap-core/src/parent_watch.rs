use std::sync::Arc;
use std::time::Duration;

use instance_manager::is_pid_alive;
use tokio::sync::Notify;
use tracing::info;

/// Ask the kernel to deliver `SIGTERM` when our parent process exits (Linux).
pub fn arm_parent_death_signal() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
}

/// Returns the parent pid at watch start.
pub fn current_ppid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::getppid() as u32 }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// True when the original parent is gone or we were reparented away from it.
pub fn parent_went_away(initial_ppid: u32) -> bool {
    if initial_ppid == 0 {
        return false;
    }
    let ppid = current_ppid();
    if ppid == 1 && initial_ppid != 1 {
        return true;
    }
    if ppid != initial_ppid {
        return true;
    }
    !is_pid_alive(initial_ppid)
}

/// Poll parent liveness and notify when the spawning process disappears.
pub fn spawn_parent_watch(poll_interval: Duration) -> Arc<Notify> {
    let notify = Arc::new(Notify::new());
    let initial_ppid = current_ppid();
    if initial_ppid == 0 {
        return notify;
    }

    let shutdown = notify.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(poll_interval).await;
            if parent_went_away(initial_ppid) {
                info!(
                    initial_ppid,
                    current_ppid = current_ppid(),
                    "parent process exited; requesting proxy shutdown"
                );
                shutdown.notify_waiters();
                break;
            }
        }
    });

    notify
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_went_away_false_for_current_parent() {
        let ppid = current_ppid();
        if ppid > 0 {
            assert!(!parent_went_away(ppid));
        }
    }

    #[test]
    fn parent_went_away_true_for_dead_pid() {
        assert!(parent_went_away(9_999_999));
    }
}
