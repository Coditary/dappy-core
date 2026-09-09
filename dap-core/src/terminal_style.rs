use std::io::IsTerminal;

/// ANSI styling for human-readable REPL output. Disabled when `NO_COLOR` is set or stdout is not a TTY.
#[derive(Debug, Clone, Copy)]
pub struct TerminalStyle {
    pub enabled: bool,
}

impl TerminalStyle {
    pub fn detect() -> Self {
        let enabled = std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal()
            && !std::env::var("DAP_NO_COLOR")
                .is_ok_and(|value| !value.is_empty() && value != "0");
        Self { enabled }
    }

    pub fn wrap(&self, text: &str, code: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap(text, "1")
    }

    pub fn dim(&self, text: &str) -> String {
        self.wrap(text, "2")
    }

    pub fn cyan(&self, text: &str) -> String {
        self.wrap(text, "36")
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap(text, "32")
    }

    pub fn yellow(&self, text: &str) -> String {
        self.wrap(text, "33")
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap(text, "31")
    }

    pub fn blue(&self, text: &str) -> String {
        self.wrap(text, "34")
    }
}
