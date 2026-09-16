use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::profile::{
    ChildBackendTemplate, ChildSessionConfig, ChildSessionRule, DebugRequestTemplate,
    ParentBackendKind, PortTemplate,
};

/// Parsed `startDebugging` reverse-request arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartDebuggingArgs {
    pub request: String,
    #[serde(default)]
    pub configuration: Value,
}

impl StartDebuggingArgs {
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Parent session metadata used when resolving child-session rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentSessionContext {
    pub backend: ParentBackendKind,
    pub adapter_cmd: Vec<String>,
    pub tcp_endpoint: Option<String>,
}

/// Resolved child backend transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildBackendPlan {
    Stdio { command: String, args: Vec<String> },
    Tcp { host: String, port: u16 },
}

/// Resolved plan for spawning a child debug session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpawnPlan {
    pub backend: ChildBackendPlan,
    pub debug_request: String,
    pub debug_arguments: Value,
    pub child_depth: u32,
}

/// Result of a successful child spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpawnResult {
    pub control_port: u16,
    pub instance_id: String,
}

pub fn strip_emit_start_debugging(cmd: &[String]) -> Vec<String> {
    cmd.iter()
        .filter(|arg| *arg != "--emit-start-debugging")
        .cloned()
        .collect()
}

/// Resolve a `startDebugging` reverse request into a child spawn plan.
pub fn resolve_child_spawn(
    config: &ChildSessionConfig,
    remaining_depth: u32,
    active_children: u32,
    parent: &ParentSessionContext,
    args: &StartDebuggingArgs,
) -> Result<ChildSpawnPlan, String> {
    if !config.auto_spawn {
        return Err("child sessions are disabled".into());
    }
    if remaining_depth == 0 {
        return Err("max child depth reached".into());
    }
    if active_children >= config.max_children {
        return Err("max concurrent children reached".into());
    }

    let context = serde_json::to_value(args).map_err(|err| err.to_string())?;
    let rule = config
        .profile
        .rules
        .iter()
        .find(|rule| rule_matches(rule, args, &context, parent) && backend_compatible(rule, parent))
        .ok_or_else(|| {
            config
                .profile
                .unsupported_message
                .clone()
                .unwrap_or_else(|| "no matching child-session rule".into())
        })?;

    let backend = build_child_backend(&rule.child_backend, parent, &context)?;
    let debug_request = resolve_debug_request(&rule.debug_request, &context)?;

    Ok(ChildSpawnPlan {
        backend,
        debug_request: debug_request.0,
        debug_arguments: debug_request.1,
        child_depth: remaining_depth - 1,
    })
}

fn rule_matches(
    rule: &ChildSessionRule,
    args: &StartDebuggingArgs,
    context: &Value,
    parent: &ParentSessionContext,
) -> bool {
    if let Some(expected) = &rule.when.request {
        if &args.request != expected {
            return false;
        }
    }
    for path in &rule.when.exists {
        if context.pointer(&dotted_to_pointer(path)).is_none() {
            return false;
        }
    }
    if !rule.when.parent_backend.is_empty() && !rule.when.parent_backend.contains(&parent.backend) {
        return false;
    }
    true
}

fn backend_compatible(rule: &ChildSessionRule, parent: &ParentSessionContext) -> bool {
    match &rule.child_backend {
        ChildBackendTemplate::ParentBackend => parent.backend == ParentBackendKind::Tcp,
        ChildBackendTemplate::InheritParentStdio => parent.backend == ParentBackendKind::Stdio,
        ChildBackendTemplate::Stdio { .. } | ChildBackendTemplate::Tcp { .. } => true,
    }
}

fn build_child_backend(
    template: &ChildBackendTemplate,
    parent: &ParentSessionContext,
    context: &Value,
) -> Result<ChildBackendPlan, String> {
    match template {
        ChildBackendTemplate::Stdio { cmd, args } => {
            let command = resolve_string_template(cmd, context)?;
            let args = args
                .iter()
                .map(|arg| resolve_string_template(arg, context))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ChildBackendPlan::Stdio { command, args })
        }
        ChildBackendTemplate::Tcp { host, port } => {
            let host = resolve_string_template(host, context)?;
            let port = resolve_port(port, context)?;
            Ok(ChildBackendPlan::Tcp { host, port })
        }
        ChildBackendTemplate::ParentBackend => {
            let endpoint = parent
                .tcp_endpoint
                .as_deref()
                .ok_or_else(|| "parent has no tcp endpoint".to_string())?;
            let (host, port) = parse_host_port(endpoint)?;
            Ok(ChildBackendPlan::Tcp { host, port })
        }
        ChildBackendTemplate::InheritParentStdio => {
            let cmd = strip_emit_start_debugging(&parent.adapter_cmd);
            let command = cmd
                .first()
                .cloned()
                .ok_or_else(|| "parent adapter command is empty".to_string())?;
            let args = cmd.into_iter().skip(1).collect();
            Ok(ChildBackendPlan::Stdio { command, args })
        }
    }
}

fn resolve_debug_request(
    template: &DebugRequestTemplate,
    context: &Value,
) -> Result<(String, Value), String> {
    let request = resolve_string_template(&template.request, context)?;
    let arguments = resolve_value_template(&template.arguments, context)?;
    Ok((request, arguments))
}

fn resolve_port(template: &PortTemplate, context: &Value) -> Result<u16, String> {
    match template {
        PortTemplate::Number(port) => Ok(*port),
        PortTemplate::Template(expr) => {
            let value = resolve_template_value(expr, context)?;
            value
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| format!("port template {expr} did not resolve to u16"))
        }
    }
}

fn resolve_string_template(template: &str, context: &Value) -> Result<String, String> {
    if template.starts_with("${") && template.ends_with('}') {
        let value = resolve_template_value(template, context)?;
        return match value {
            Value::String(text) => Ok(text),
            other => Ok(other.to_string()),
        };
    }
    Ok(template.to_string())
}

fn resolve_value_template(template: &Value, context: &Value) -> Result<Value, String> {
    match template {
        Value::String(expr) if expr.starts_with("${") && expr.ends_with('}') => {
            resolve_template_value(expr, context)
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                out.insert(key.clone(), resolve_value_template(value, context)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| resolve_value_template(item, context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

fn resolve_template_value(expr: &str, context: &Value) -> Result<Value, String> {
    let path = expr
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .ok_or_else(|| format!("invalid template expression: {expr}"))?;
    if path == "configuration" {
        return Ok(context.get("configuration").cloned().unwrap_or(Value::Null));
    }
    if path == "request" {
        return Ok(context.get("request").cloned().unwrap_or(Value::Null));
    }
    context
        .pointer(&dotted_to_pointer(path))
        .cloned()
        .ok_or_else(|| format!("template path not found: {path}"))
}

fn dotted_to_pointer(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    format!("/{}", path.replace('.', "/"))
}

fn parse_host_port(endpoint: &str) -> Result<(String, u16), String> {
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid tcp endpoint: {endpoint}"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid tcp port in endpoint: {endpoint}"))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::child_session::profile::ChildSessionProfile;
    use crate::child_session::profile::{
        ChildBackendTemplate, ChildSessionRule, DebugRequestTemplate, RuleCondition,
    };

    fn debugpy_config() -> ChildSessionConfig {
        ChildSessionConfig {
            auto_spawn: true,
            max_children: 4,
            max_depth: 1,
            profile: ChildSessionProfile::debugpy_preset(),
        }
    }

    fn stdio_parent(cmd: &[&str]) -> ParentSessionContext {
        ParentSessionContext {
            backend: ParentBackendKind::Stdio,
            adapter_cmd: cmd.iter().map(|s| s.to_string()).collect(),
            tcp_endpoint: None,
        }
    }

    #[test]
    fn fake_preset_matches_launch_and_inherits_stdio() {
        let config = ChildSessionConfig {
            auto_spawn: true,
            max_children: 4,
            max_depth: 1,
            profile: ChildSessionProfile::fake_preset(),
        };
        let plan = resolve_child_spawn(
            &config,
            1,
            0,
            &stdio_parent(&["fake-adapter", "--emit-start-debugging"]),
            &StartDebuggingArgs {
                request: "launch".into(),
                configuration: Value::Object(Default::default()),
            },
        )
        .expect("plan");
        assert_eq!(
            plan.backend,
            ChildBackendPlan::Stdio {
                command: "fake-adapter".into(),
                args: vec![],
            }
        );
    }

    #[test]
    fn debugpy_preset_resolves_tcp_attach() {
        let plan = resolve_child_spawn(
            &debugpy_config(),
            1,
            0,
            &stdio_parent(&["debugpy"]),
            &StartDebuggingArgs {
                request: "attach".into(),
                configuration: serde_json::json!({
                    "connect": { "host": "127.0.0.1", "port": 5678 }
                }),
            },
        )
        .expect("plan");
        assert_eq!(
            plan.backend,
            ChildBackendPlan::Tcp {
                host: "127.0.0.1".into(),
                port: 5678,
            }
        );
        assert_eq!(plan.debug_request, "attach");
    }

    #[test]
    fn debugpy_preset_rejects_launch_requests() {
        let err = resolve_child_spawn(
            &debugpy_config(),
            1,
            0,
            &stdio_parent(&["debugpy"]),
            &StartDebuggingArgs {
                request: "launch".into(),
                configuration: Value::Object(Default::default()),
            },
        )
        .expect_err("no rule");
        assert!(err.contains("no matching"));
    }

    #[test]
    fn lldb_dap_preset_fails_closed_for_stdio_parent() {
        let config = ChildSessionConfig {
            auto_spawn: true,
            max_children: 4,
            max_depth: 1,
            profile: ChildSessionProfile::lldb_dap_preset(),
        };
        let err = resolve_child_spawn(
            &config,
            1,
            0,
            &stdio_parent(&["lldb-dap"]),
            &StartDebuggingArgs {
                request: "attach".into(),
                configuration: Value::Object(Default::default()),
            },
        )
        .expect_err("unsupported");
        assert!(err.contains("lldb-dap"));
    }

    #[test]
    fn lldb_dap_preset_reuses_parent_tcp_endpoint() {
        let config = ChildSessionConfig {
            auto_spawn: true,
            max_children: 4,
            max_depth: 1,
            profile: ChildSessionProfile::lldb_dap_preset(),
        };
        let parent = ParentSessionContext {
            backend: ParentBackendKind::Tcp,
            adapter_cmd: vec![],
            tcp_endpoint: Some("127.0.0.1:4711".into()),
        };
        let plan = resolve_child_spawn(
            &config,
            1,
            0,
            &parent,
            &StartDebuggingArgs {
                request: "attach".into(),
                configuration: serde_json::json!({ "targetId": 1 }),
            },
        )
        .expect("plan");
        assert_eq!(
            plan.backend,
            ChildBackendPlan::Tcp {
                host: "127.0.0.1".into(),
                port: 4711,
            }
        );
    }

    #[test]
    fn stdio_template_rule_resolves_literal_backend() {
        let config = ChildSessionConfig {
            auto_spawn: true,
            max_children: 4,
            max_depth: 1,
            profile: ChildSessionProfile {
                rules: vec![ChildSessionRule {
                    when: RuleCondition {
                        request: Some("launch".into()),
                        exists: vec![],
                        parent_backend: vec![],
                    },
                    child_backend: ChildBackendTemplate::Stdio {
                        cmd: "worker".into(),
                        args: vec!["--flag".into()],
                    },
                    debug_request: DebugRequestTemplate {
                        request: "launch".into(),
                        arguments: serde_json::json!({}),
                    },
                }],
                unsupported_message: None,
            },
        };
        let plan = resolve_child_spawn(
            &config,
            1,
            0,
            &stdio_parent(&["parent"]),
            &StartDebuggingArgs {
                request: "launch".into(),
                configuration: serde_json::json!({}),
            },
        )
        .expect("plan");
        assert_eq!(
            plan.backend,
            ChildBackendPlan::Stdio {
                command: "worker".into(),
                args: vec!["--flag".into()],
            }
        );
    }

    #[test]
    fn resolve_fails_when_auto_spawn_disabled() {
        let config = ChildSessionConfig::default();
        let err = resolve_child_spawn(
            &config,
            1,
            0,
            &stdio_parent(&["parent"]),
            &StartDebuggingArgs {
                request: "launch".into(),
                configuration: serde_json::json!({}),
            },
        )
        .expect_err("disabled");
        assert!(err.contains("disabled"));
    }
}
