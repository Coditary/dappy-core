use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Inputs for `${…}` placeholders in plugin manifests.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    pub program: String,
    pub program_abs: Option<String>,
    pub program_parent: Option<PathBuf>,
    pub plugin_id: String,
}

impl TemplateContext {
    pub fn new(program: impl Into<String>, plugin_id: impl Into<String>) -> Self {
        let program = program.into();
        let path = Path::new(&program);
        let (program_abs, program_parent) = resolve_program_paths(path);
        Self {
            program,
            program_abs,
            program_parent,
            plugin_id: plugin_id.into(),
        }
    }
}

fn resolve_program_paths(path: &Path) -> (Option<String>, Option<PathBuf>) {
    if let Ok(abs) = path.canonicalize() {
        return (
            Some(abs.display().to_string()),
            abs.parent().map(std::path::Path::to_path_buf),
        );
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let cwd = parent.and_then(|p| p.canonicalize().ok());

    let program_abs = match (&cwd, path.file_name()) {
        (Some(cwd), Some(file)) if !path.is_absolute() => {
            Some(cwd.join(file).display().to_string())
        }
        _ => None,
    };

    (program_abs, cwd)
}

enum ResolveOutcome {
    Keep(Value),
    Omit,
}

/// Recursively resolve `${…}` placeholders in JSON values.
pub fn resolve_value(value: &Value, ctx: &TemplateContext) -> Option<Value> {
    match resolve_value_inner(value, ctx) {
        ResolveOutcome::Keep(value) => Some(value),
        ResolveOutcome::Omit => None,
    }
}

fn resolve_value_inner(value: &Value, ctx: &TemplateContext) -> ResolveOutcome {
    match value {
        Value::String(text) => match resolve_string(text, ctx) {
            Some(resolved) => ResolveOutcome::Keep(Value::String(resolved)),
            None => ResolveOutcome::Omit,
        },
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                if let ResolveOutcome::Keep(resolved) = resolve_value_inner(child, ctx) {
                    out.insert(key.clone(), resolved);
                }
            }
            ResolveOutcome::Keep(Value::Object(out))
        }
        Value::Array(items) => ResolveOutcome::Keep(Value::Array(
            items
                .iter()
                .filter_map(|item| resolve_value(item, ctx))
                .collect(),
        )),
        other => ResolveOutcome::Keep(other.clone()),
    }
}

fn resolve_string(text: &str, ctx: &TemplateContext) -> Option<String> {
    if !(text.starts_with("${") && text.ends_with('}')) {
        return Some(text.to_string());
    }

    let token = text[2..text.len() - 1].trim();
    match token {
        "program" => Some(ctx.program.clone()),
        "program.abs" => Some(
            ctx.program_abs
                .clone()
                .unwrap_or_else(|| ctx.program.clone()),
        ),
        "program.parent" | "cwd" => ctx
            .program_parent
            .as_ref()
            .map(|path| path.display().to_string()),
        "plugin.id" => Some(ctx.plugin_id.clone()),
        other if other.starts_with("env:") => {
            std::env::var(other.strip_prefix("env:").unwrap_or("")).ok()
        }
        _ => Some(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_program_and_env_templates() {
        let ctx = TemplateContext::new("main.py", "python");
        assert_eq!(
            resolve_string("${program}", &ctx).as_deref(),
            Some("main.py")
        );
        unsafe {
            std::env::set_var("DAP_TEST_TEMPLATE_VAR", "hello");
        }
        assert_eq!(
            resolve_string("${env:DAP_TEST_TEMPLATE_VAR}", &ctx).as_deref(),
            Some("hello")
        );
        assert!(resolve_string("${env:DAP_TEST_TEMPLATE_MISSING}", &ctx).is_none());
    }

    #[test]
    fn omits_missing_env_fields_in_objects() {
        let ctx = TemplateContext::new("main.py", "python");
        let input = json!({
            "program": "${program}",
            "python": "${env:DAP_TEST_TEMPLATE_MISSING}"
        });
        let resolved = resolve_value(&input, &ctx).expect("resolved");
        assert_eq!(resolved["program"], "main.py");
        assert!(resolved.get("python").is_none());
    }
}
