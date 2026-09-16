use serde::{Deserialize, Serialize};

/// How to reach an upstream server process or socket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase", tag = "transport")]
pub enum SpawnSpec {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Tcp {
        host: String,
        port: u16,
    },
    Builtin {
        id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpawnTransport {
    Stdio,
    Tcp,
    Builtin,
}

impl SpawnSpec {
    pub fn stdio(command: impl Into<String>, args: Vec<String>) -> Self {
        Self::Stdio {
            command: command.into(),
            args,
        }
    }

    pub fn transport(&self) -> SpawnTransport {
        match self {
            Self::Stdio { .. } => SpawnTransport::Stdio,
            Self::Tcp { .. } => SpawnTransport::Tcp,
            Self::Builtin { .. } => SpawnTransport::Builtin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_spec_reports_transport() {
        let spec = SpawnSpec::stdio("echo", vec!["hi".into()]);
        assert_eq!(spec.transport(), SpawnTransport::Stdio);
    }

    #[test]
    fn tcp_spec_reports_transport() {
        let spec = SpawnSpec::Tcp {
            host: "127.0.0.1".into(),
            port: 1234,
        };
        assert_eq!(spec.transport(), SpawnTransport::Tcp);
    }
}
