use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::capabilities::AdapterCapabilities;
use crate::navigation::NavigationType;

/// Match stored breakpoint paths against stack-frame paths (slashes, suffixes, basenames).
pub fn source_paths_match(stored: &str, current: &str) -> bool {
    if stored == current {
        return true;
    }

    fn normalize(path: &str) -> String {
        path.replace('\\', "/").trim_end_matches('/').to_string()
    }

    let stored_norm = normalize(stored);
    let current_norm = normalize(current);
    if stored_norm == current_norm {
        return true;
    }
    if current_norm.ends_with(&stored_norm)
        || stored_norm.ends_with(&current_norm)
        || current_norm.ends_with(&format!("/{stored_norm}"))
        || stored_norm.ends_with(&format!("/{current_norm}"))
    {
        return true;
    }

    let stored_base = Path::new(&stored_norm)
        .file_name()
        .map(|name| name.to_string_lossy().to_string());
    let current_base = Path::new(&current_norm)
        .file_name()
        .map(|name| name.to_string_lossy().to_string());
    stored_base.is_some() && stored_base == current_base
}

/// Whether a hit condition is satisfied for the given stop count.
pub fn hit_count_matches(hit_condition: &str, count: u32) -> bool {
    let trimmed = hit_condition.trim();
    if trimmed.is_empty() {
        return true;
    }

    if let Some(rest) = trimmed.strip_prefix(">=") {
        return count >= rest.trim().parse().unwrap_or(0);
    }
    if let Some(rest) = trimmed.strip_prefix("<=") {
        return count <= rest.trim().parse().unwrap_or(0);
    }
    if let Some(rest) = trimmed.strip_prefix('>') {
        return count > rest.trim().parse().unwrap_or(0);
    }
    if let Some(rest) = trimmed.strip_prefix('<') {
        return count < rest.trim().parse().unwrap_or(0);
    }
    if let Some(rest) = trimmed.strip_prefix('%') {
        let modulus = rest.trim().parse().unwrap_or(1).max(1);
        return count % modulus == 0;
    }

    count >= trimmed.parse().unwrap_or(1)
}

/// Match a source path against a skip pattern (`*` wildcards supported).
pub fn path_matches_skip(path: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if pattern.contains('*') {
        let parts = pattern.split('*').collect::<Vec<_>>();
        let mut cursor = path;
        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if index == 0 {
                if !cursor.starts_with(part) {
                    return false;
                }
                cursor = &cursor[part.len()..];
                continue;
            }
            if index == parts.len() - 1 {
                return cursor.ends_with(part);
            }
            match cursor.find(part) {
                Some(pos) => cursor = &cursor[pos + part.len()..],
                None => return false,
            }
        }
        return true;
    }

    path.contains(pattern)
}

/// Validate that `line` exists in a local source file. Returns a warning when suspicious.
pub fn validate_source_line(path: &str, line: i64) -> Option<String> {
    if line < 1 {
        return Some(format!("line {line} is less than 1"));
    }

    let file = Path::new(path);
    if !file.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(file).ok()?;
    let line_count = content.lines().count() as i64;
    if line > line_count {
        return Some(format!(
            "line {line} is past end of file ({line_count} lines)"
        ));
    }

    let line_text = content.lines().nth((line - 1) as usize).unwrap_or("");
    if line_text.trim().is_empty() {
        return Some(format!("line {line} appears empty"));
    }

    None
}

pub fn ensure_navigation_supported(
    capabilities: &AdapterCapabilities,
    navigation_type: NavigationType,
) -> Result<(), String> {
    match navigation_type {
        NavigationType::StepBack | NavigationType::ReverseContinue
            if !capabilities.supports_step_back =>
        {
            Err(format!(
                "adapter does not support {navigation_type} (supportsStepBack=false)"
            ))
        }
        _ => Ok(()),
    }
}

pub fn uses_emulated_exception_condition(
    capabilities: &AdapterCapabilities,
    condition: Option<&str>,
) -> bool {
    condition.is_some() && !capabilities.supports_exception_filter_options
}

pub fn uses_client_stop_policy(
    capabilities: &AdapterCapabilities,
    condition: Option<&str>,
    hit_condition: Option<&str>,
    log_message: Option<&str>,
) -> ClientStopPolicy {
    ClientStopPolicy {
        condition: condition.is_some() && !capabilities.supports_conditional_breakpoints,
        hit: hit_condition.is_some() && !capabilities.supports_hit_conditional_breakpoints,
        log: log_message.is_some() && !capabilities.supports_log_points,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStopPolicy {
    pub condition: bool,
    pub hit: bool,
    pub log: bool,
}

impl ClientStopPolicy {
    pub fn any(&self) -> bool {
        self.condition || self.hit || self.log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_count_exact_and_comparators() {
        assert!(!hit_count_matches("3", 2));
        assert!(hit_count_matches("3", 3));
        assert!(!hit_count_matches(">3", 3));
        assert!(hit_count_matches(">3", 4));
        assert!(hit_count_matches("%2", 2));
    }

    #[test]
    fn source_paths_match_normalizes_and_compares_basenames() {
        assert!(source_paths_match("main.py", "/proj/main.py"));
        assert!(source_paths_match("/proj/main.py", "main.py"));
        assert!(source_paths_match("src\\main.py", "src/main.py"));
        assert!(!source_paths_match("other.py", "main.py"));
    }

    #[test]
    fn emulated_exception_condition_when_adapter_lacks_support() {
        let caps = AdapterCapabilities::default();
        assert!(uses_emulated_exception_condition(
            &caps,
            Some("type(e) == ValueError")
        ));
        assert!(!uses_emulated_exception_condition(&caps, None));

        let caps = AdapterCapabilities {
            supports_exception_filter_options: true,
            ..Default::default()
        };
        assert!(!uses_emulated_exception_condition(
            &caps,
            Some("type(e) == ValueError")
        ));
    }

    #[test]
    fn skip_patterns() {
        assert!(path_matches_skip("/home/proj/target/debug/foo", "/target/"));
        assert!(path_matches_skip("/foo/bar.rs", "*/bar.rs"));
        assert!(!path_matches_skip("/foo/bar.rs", "/vendor/"));
        assert!(path_matches_skip("/fake/main.py", "/fake/"));
    }

    #[test]
    fn hit_count_modulus_and_less_than() {
        assert!(hit_count_matches("%3", 3));
        assert!(!hit_count_matches("%3", 4));
        assert!(hit_count_matches("<=2", 2));
        assert!(!hit_count_matches("<=2", 3));
        assert!(hit_count_matches("<3", 2));
    }

    #[test]
    fn client_stop_policy_when_adapter_lacks_features() {
        let caps = AdapterCapabilities::default();
        let policy = uses_client_stop_policy(&caps, Some("x > 0"), Some("3"), Some("here"));
        assert!(policy.condition);
        assert!(policy.hit);
        assert!(policy.log);
        assert!(policy.any());
    }

    #[test]
    fn client_stop_policy_skips_native_features() {
        let caps = AdapterCapabilities {
            supports_conditional_breakpoints: true,
            supports_hit_conditional_breakpoints: true,
            supports_log_points: true,
            ..Default::default()
        };
        let policy = uses_client_stop_policy(&caps, Some("x > 0"), Some("3"), Some("here"));
        assert!(!policy.condition);
        assert!(!policy.hit);
        assert!(!policy.log);
        assert!(!policy.any());
    }

    #[test]
    fn navigation_gated_by_capabilities() {
        let caps = AdapterCapabilities::default();
        assert!(ensure_navigation_supported(&caps, NavigationType::StepOver).is_ok());
        assert!(ensure_navigation_supported(&caps, NavigationType::StepBack).is_err());
        assert!(ensure_navigation_supported(&caps, NavigationType::ReverseContinue).is_err());

        let caps = AdapterCapabilities {
            supports_step_back: true,
            ..Default::default()
        };
        assert!(ensure_navigation_supported(&caps, NavigationType::StepBack).is_ok());
    }

    #[test]
    fn validate_source_line_warns_on_empty_and_past_eof() {
        let dir = std::env::temp_dir().join(format!(
            "dap-stop-policy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("sample.rs");
        std::fs::write(&file, "fn main() {\n    \n}\n").expect("write sample");

        assert!(
            validate_source_line(file.to_str().unwrap(), 0)
                .unwrap()
                .contains("less than 1")
        );
        assert!(
            validate_source_line(file.to_str().unwrap(), 99)
                .unwrap()
                .contains("past end")
        );
        assert!(
            validate_source_line(file.to_str().unwrap(), 2)
                .unwrap()
                .contains("empty")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
