use crate::proxy::kill_adapter_process;

/// Keeps an RSP target subprocess alive until explicit shutdown.
pub struct TargetGuard {
    process: Option<tokio::process::Child>,
}

impl TargetGuard {
    pub fn new(process: Option<tokio::process::Child>) -> Self {
        Self { process }
    }

    pub async fn shutdown(&mut self) {
        kill_adapter_process(&mut self.process);
        if let Some(mut child) = self.process.take() {
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
        }
    }
}
