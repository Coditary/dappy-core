//! Heuristics and launch payloads for the debugpy / Python adapter.

use serde_json::json;

/// Return true when the adapter id refers to the Python/debugpy backend.
pub fn is_python_adapter(adapter_id: Option<&str>) -> bool {
    matches!(adapter_id, Some("python"))
}

/// Whether a headless launch should include debugpy-specific arguments.
pub fn wants_python_launch_hints(adapter_id: Option<&str>) -> bool {
    is_python_adapter(adapter_id)
}

/// Resolve the canonical program path used in debugpy launch requests.
pub fn python_program_path(program: &str) -> String {
    resolve_python_program_paths(std::path::Path::new(program)).0
}

/// Build a debugpy-compatible `launch` arguments object.
pub fn python_launch_arguments(program: &str) -> serde_json::Value {
    let path = std::path::Path::new(program);
    let (program_path, cwd) = resolve_python_program_paths(path);

    let mut args = json!({
        "type": "debugpy",
        "request": "launch",
        "program": program_path,
        "console": "internalConsole",
        // justMyCode=false + stopOnEntry is very slow on Python 3.14; entry is handled
        // via a line-1 breakpoint in session_init instead.
        "justMyCode": python_just_my_code(),
        "stopOnEntry": false,
    });
    if let Ok(python) = std::env::var("PYTHON") {
        args["python"] = json!(python);
    }
    if let Some(cwd) = cwd {
        args["cwd"] = json!(cwd);
    }
    args
}

fn python_just_my_code() -> bool {
    match std::env::var("DAP_PYTHON_JUST_MY_CODE")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("0") | Some("false") | Some("False") | Some("no") | Some("NO") => false,
        Some("1") | Some("true") | Some("True") | Some("yes") | Some("YES") => true,
        _ => true,
    }
}

fn resolve_python_program_paths(path: &std::path::Path) -> (String, Option<std::path::PathBuf>) {
    if let Ok(abs) = path.canonicalize() {
        return (
            abs.display().to_string(),
            abs.parent().map(std::path::Path::to_path_buf),
        );
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let cwd = parent.and_then(|p| p.canonicalize().ok());

    let program_path = match (&cwd, path.file_name()) {
        (Some(cwd), Some(file)) if !path.is_absolute() => {
            cwd.join(file).display().to_string()
        }
        _ => path.display().to_string(),
    };

    (program_path, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_program() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/main.py")
            .display()
            .to_string()
    }

    #[test]
    fn python_hints_follow_adapter_id() {
        assert!(wants_python_launch_hints(Some("python")));
        assert!(!wants_python_launch_hints(Some("fake")));
        assert!(!wants_python_launch_hints(None));
    }

    #[test]
    fn launch_arguments_include_debugpy_fields() {
        let args = python_launch_arguments(&fixture_program());
        assert_eq!(args["type"], "debugpy");
        assert_eq!(args["request"], "launch");
        assert_eq!(args["stopOnEntry"], false);
        assert_eq!(args["justMyCode"], true);
        assert_eq!(args["program"].as_str().unwrap().ends_with("main.py"), true);
        assert!(
            std::path::Path::new(args["program"].as_str().unwrap()).is_absolute(),
            "program path should be absolute"
        );
        assert!(args.get("cwd").is_some());
    }

    #[test]
    fn launch_arguments_use_absolute_cwd_for_relative_program() {
        let args = python_launch_arguments(&fixture_program());
        let cwd = args["cwd"].as_str().expect("cwd");
        assert!(
            std::path::Path::new(cwd).is_absolute(),
            "cwd should be absolute, got {cwd}"
        );
    }
}
