use std::path::{Path, PathBuf};

use serde_json::Value;

/// Location within a source file from a stack frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLocation {
    pub path: String,
    pub line: i64,
    pub column: Option<i64>,
    pub name: Option<String>,
    pub frame_id: Option<i64>,
}

/// Options for rendering a source snippet.
#[derive(Debug, Clone, Copy)]
pub struct SourceShowOptions {
    /// Lines shown before and after the current line (default 2 → 5 lines total).
    pub context_lines: usize,
}

impl Default for SourceShowOptions {
    fn default() -> Self {
        Self { context_lines: 2 }
    }
}

/// Extract file/line metadata from a DAP stack frame object.
pub fn frame_location(frame: &Value) -> Option<FrameLocation> {
    let line = frame.get("line").and_then(Value::as_i64)?;
    if line <= 0 {
        return None;
    }
    let path = frame
        .get("source")
        .and_then(|source| source.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())?;
    Some(FrameLocation {
        path: path.to_string(),
        line,
        column: frame.get("column").and_then(Value::as_i64),
        name: frame
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        frame_id: frame.get("id").and_then(Value::as_i64),
    })
}

/// Pick a stack frame by id, or the first frame when `frame_id` is missing from the trace.
pub fn select_stack_frame(stack: &Value, frame_id: i64) -> Option<Value> {
    let frames = stack.get("stackFrames").and_then(Value::as_array)?;
    if let Some(frame) = frames
        .iter()
        .find(|frame| frame.get("id").and_then(Value::as_i64) == Some(frame_id))
    {
        return Some(frame.clone());
    }
    frames.first().cloned()
}

/// Read source text from disk, trying the path as given, relative to the cwd,
/// and relative to any optional hint directories (e.g. the launched program's folder).
pub fn read_source_file(path: &str) -> Option<String> {
    read_source_file_with_hints(path, &[])
}

pub fn read_source_file_with_hints(path: &str, hint_dirs: &[PathBuf]) -> Option<String> {
    for candidate in source_path_candidates(path, hint_dirs) {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    None
}

/// Directories used when resolving relative source paths from stack frames.
pub fn default_source_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    dirs
}

/// Hint directories derived from the launched program path.
pub fn source_search_dirs_for_program(program: &str) -> Vec<PathBuf> {
    let mut dirs = default_source_search_dirs();
    let path = Path::new(program);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            dirs.push(parent.to_path_buf());
            if parent.is_relative() {
                if let Ok(cwd) = std::env::current_dir() {
                    dirs.push(cwd.join(parent));
                }
            }
        }
    }

    if path.exists() {
        if let Ok(abs) = path.canonicalize() {
            if let Some(parent) = abs.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

fn source_path_candidates(path: &str, hint_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let raw = Path::new(path);
    out.push(raw.to_path_buf());
    if raw.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            out.push(cwd.join(raw));
        }
        for dir in hint_dirs {
            out.push(dir.join(raw));
        }
    }
    out
}

/// Render a gdb-style source listing with line numbers and a `>` marker on the current line.
pub fn format_source_show(
    location: &FrameLocation,
    source: Option<&str>,
    options: SourceShowOptions,
) -> String {
    format_source_show_styled(location, source, options, None)
}

/// Render source with optional ANSI styling and light Python highlighting.
pub fn format_source_show_styled(
    location: &FrameLocation,
    source: Option<&str>,
    options: SourceShowOptions,
    style: Option<crate::terminal_style::TerminalStyle>,
) -> String {
    let style = style.unwrap_or(crate::terminal_style::TerminalStyle { enabled: false });
    let header_name = location
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or("?");

    let mut out = String::from("Source:\n");
    let file = Path::new(&location.path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| location.path.clone());
    let header = format!("  {}:{} in {}", file, location.line, header_name);
    out.push_str(&style.cyan(&header));
    out.push('\n');

    match source {
        Some(text) => {
            out.push_str(&format_source_snippet_styled(
                location.line,
                text,
                options.context_lines,
                &style,
            ));
        }
        None => {
            out.push_str(&style.dim("  (source file not found)\n"));
            out.push_str(&format!("  path: {}\n", location.path));
            out.push_str("  hint: run `show` from the script directory, or launch with a path like ./main.py\n");
        }
    }

    out
}

fn format_source_snippet(line: i64, source: &str, context_lines: usize) -> String {
    format_source_snippet_styled(
        line,
        source,
        context_lines,
        &crate::terminal_style::TerminalStyle { enabled: false },
    )
}

fn format_source_snippet_styled(
    line: i64,
    source: &str,
    context_lines: usize,
    style: &crate::terminal_style::TerminalStyle,
) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return "(empty source file)\n".into();
    }

    let current = line.clamp(1, lines.len() as i64) as usize;
    let start = current.saturating_sub(context_lines).max(1);
    let end = (current + context_lines).min(lines.len());
    let width = end.to_string().len().max(3);

    let mut out = String::new();
    for line_no in start..=end {
        let marker = if line_no == current {
            style.green(">")
        } else {
            " ".to_string()
        };
        let line_num = style.dim(&format!(
            "{line_no:>width$}",
            line_no = line_no,
            width = width
        ));
        let text = highlight_source_line(lines[line_no - 1], style);
        out.push_str(&format!("  {marker} {line_num} | {text}\n"));
    }
    out
}

fn highlight_source_line(line: &str, style: &crate::terminal_style::TerminalStyle) -> String {
    if !style.enabled {
        return line.to_string();
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return style.dim(line);
    }
    highlight_python_line(line, style)
}

fn highlight_python_line(line: &str, style: &crate::terminal_style::TerminalStyle) -> String {
    if !style.enabled {
        return line.to_string();
    }

    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return style.dim(line);
    }

    let mut out = String::new();
    let mut idx = 0usize;
    let bytes = line.as_bytes();
    while idx < bytes.len() {
        if bytes[idx] == b'#' {
            out.push_str(&style.dim(&line[idx..]));
            break;
        }
        if bytes[idx] == b'"' || bytes[idx] == b'\'' {
            let quote = bytes[idx];
            let start = idx;
            idx += 1;
            while idx < bytes.len() {
                if bytes[idx] == b'\\' {
                    idx = (idx + 2).min(bytes.len());
                    continue;
                }
                if bytes[idx] == quote {
                    idx += 1;
                    break;
                }
                idx += 1;
            }
            out.push_str(&style.yellow(&line[start..idx]));
            continue;
        }
        if line[idx..].starts_with("def ")
            || line[idx..].starts_with("class ")
            || line[idx..].starts_with("return ")
            || line[idx..].starts_with("import ")
            || line[idx..].starts_with("from ")
            || line[idx..].starts_with("if ")
            || line[idx..].starts_with("for ")
            || line[idx..].starts_with("while ")
            || line[idx..].starts_with("elif ")
            || line[idx..].starts_with("else:")
            || line[idx..].starts_with("try:")
            || line[idx..].starts_with("except")
        {
            let kw_end = line[idx..]
                .find(|c: char| c.is_whitespace() || c == ':')
                .map(|offset| idx + offset)
                .unwrap_or(line.len());
            out.push_str(&style.blue(&line[idx..kw_end]));
            idx = kw_end;
            continue;
        }
        out.push_str(&line[idx..idx + 1]);
        idx += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_frame_location() {
        let frame = json!({
            "id": 3,
            "name": "main",
            "line": 10,
            "column": 4,
            "source": { "path": "main.py" }
        });
        let location = frame_location(&frame).expect("location");
        assert_eq!(location.path, "main.py");
        assert_eq!(location.line, 10);
        assert_eq!(location.name.as_deref(), Some("main"));
        assert_eq!(location.frame_id, Some(3));
    }

    #[test]
    fn formats_source_snippet_with_marker() {
        let source = "line1\nline2\nline3\nline4\nline5\n";
        let text = format_source_snippet(3, source, 2);
        assert!(text.contains("| line2"));
        assert!(text.contains("| line3"));
        assert!(
            text.lines()
                .any(|line| line.contains('>') && line.contains("line3"))
        );
        assert!(text.contains("| line4"));
    }

    #[test]
    fn read_source_file_uses_hint_directories() {
        let dir = std::env::temp_dir().join(format!(
            "dap-source-hint-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("main.py"), "line1\nline2\n").expect("write file");

        let found = read_source_file_with_hints("main.py", &[dir.clone()]);
        assert!(found.is_some());
        assert!(found.unwrap().contains("line2"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn show_reports_missing_file() {
        let location = FrameLocation {
            path: "missing.py".into(),
            line: 1,
            column: None,
            name: Some("main".into()),
            frame_id: Some(1),
        };
        let text = format_source_show(&location, None, SourceShowOptions::default());
        assert!(text.contains("Source:"));
        assert!(text.contains("missing.py:1 in main"));
        assert!(text.contains("source file not found"));
    }
}
