use serde::{Deserialize, Serialize};

/// Client-side data watch (emulated data breakpoint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataWatchEntry {
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub emulated: bool,
}

impl DataWatchEntry {
    pub fn new(expression: String, emulated: bool) -> Self {
        Self {
            expression,
            baseline: None,
            armed: false,
            emulated,
        }
    }
}

/// Returns true when the watch should stop (value changed since baseline).
pub fn data_watch_should_stop(entry: &mut DataWatchEntry, current_value: &str) -> bool {
    if !entry.armed {
        entry.baseline = Some(current_value.to_string());
        entry.armed = true;
        return false;
    }
    if entry.baseline.as_deref() != Some(current_value) {
        entry.baseline = Some(current_value.to_string());
        return true;
    }
    false
}

pub fn uses_emulated_data_breakpoints(supports_data_breakpoints: bool) -> bool {
    !supports_data_breakpoints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_watch_arms_then_fires_on_change() {
        let mut entry = DataWatchEntry::new("x".into(), true);
        assert!(!data_watch_should_stop(&mut entry, "1"));
        assert!(!data_watch_should_stop(&mut entry, "1"));
        assert!(data_watch_should_stop(&mut entry, "2"));
    }
}
