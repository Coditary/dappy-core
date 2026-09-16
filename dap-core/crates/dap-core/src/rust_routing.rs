//! Heuristics for routing Rust/Cargo debug targets to the lldb-dap plugin.

/// Return true when `path` looks like a Cargo build output binary.
pub fn looks_like_cargo_binary(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.contains("/target/debug/")
        || normalized.contains("/target/release/")
        || normalized.starts_with("target/debug/")
        || normalized.starts_with("target/release/")
}

/// Return true when the adapter id refers to the Rust/lldb-dap backend.
pub fn is_rust_adapter(adapter_id: Option<&str>) -> bool {
    matches!(adapter_id, Some("rust" | "lldb-dap"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cargo_binary_paths() {
        assert!(looks_like_cargo_binary("./target/debug/myapp"));
        assert!(looks_like_cargo_binary("target/release/foo"));
        assert!(looks_like_cargo_binary(r"crates\foo\target\debug\bar"));
        assert!(!looks_like_cargo_binary("main.py"));
        assert!(!looks_like_cargo_binary("/usr/bin/python3"));
    }
}
