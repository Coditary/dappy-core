use std::path::{Path, PathBuf};

/// Find the 1-based line number of a top-level function/method definition in source text.
pub fn resolve_function_line(content: &str, name: &str) -> Option<i64> {
    for (index, line) in content.lines().enumerate() {
        if line_matches_function_definition(line, name) {
            return Some(index as i64 + 1);
        }
    }
    None
}

fn line_matches_function_definition(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
        return python_def_name(trimmed) == Some(name);
    }
    if trimmed.starts_with("fn ") || trimmed.contains(" fn ") {
        return rust_fn_name(trimmed) == Some(name);
    }
    false
}

fn python_def_name(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("async ")
        .unwrap_or(line)
        .strip_prefix("def ")
        .unwrap_or(line);
    rest.split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .filter(|name| !name.is_empty())
}

fn rust_fn_name(line: &str) -> Option<&str> {
    let marker = line.find("fn ")?;
    let rest = &line[marker + 3..];
    rest.split(|c: char| c == '(' || c == '<' || c.is_whitespace())
        .next()
        .filter(|name| !name.is_empty())
}

/// Search `dirs` for `name` in common source extensions.
pub fn find_function_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<(PathBuf, i64)> {
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_source_file(&path) {
                continue;
            }
            let content = std::fs::read_to_string(&path).ok()?;
            if let Some(line) = resolve_function_line(&content, name) {
                return Some((path, line));
            }
        }
    }
    None
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("py") | Some("rs") | Some("js") | Some("ts") | Some("go")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_python_and_rust_definitions() {
        let py = "class Foo:\n    pass\n\ndef accumulate(items):\n    return 0\n";
        assert_eq!(resolve_function_line(py, "accumulate"), Some(4));
        assert_eq!(resolve_function_line(py, "missing"), None);

        let rs = "fn helper() {}\nfn accumulate(x: i32) -> i32 { x }\n";
        assert_eq!(resolve_function_line(rs, "accumulate"), Some(2));
    }
}
