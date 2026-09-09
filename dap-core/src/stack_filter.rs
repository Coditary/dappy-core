use crate::stop_policy::path_matches_skip;

/// Path substrings hidden from default stack traces (`bt` without `full`).
pub const DEFAULT_HIDDEN_STACK_PATTERNS: &[&str] = &[
    "site-packages",
    "runpy.py",
    "/runpy",
    "_run_code",
    "/rustc/",
    "/target/deps/",
    "debugpy/",
];

pub fn is_hidden_stack_frame(path: &str, full: bool) -> bool {
    if full {
        return false;
    }
    DEFAULT_HIDDEN_STACK_PATTERNS
        .iter()
        .any(|pattern| path_matches_skip(path, pattern))
}

/// When smart-step is enabled, auto-continue through runtime/stdlib frames.
pub fn smart_step_should_skip(path: &str, enabled: bool) -> bool {
    enabled && is_hidden_stack_frame(path, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_step_skips_hidden_frames() {
        assert!(smart_step_should_skip("/usr/lib/python3/site-packages/foo.py", true));
        assert!(!smart_step_should_skip("/home/proj/main.py", true));
        assert!(!smart_step_should_skip("/usr/lib/python3/site-packages/foo.py", false));
    }
}
