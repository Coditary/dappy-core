use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const LLDB_DAP_STDIO_UNSUPPORTED_MESSAGE: &str =
    "startDebugging unsupported for lldb-dap profile with stdio parent backend; lldb-dap session handoff requires a reusable tcp server endpoint";

/// Whether and how a proxy spawns child sessions for `startDebugging` reverse requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionConfig {
    #[serde(default)]
    pub auto_spawn: bool,
    #[serde(default = "default_max_children")]
    pub max_children: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default)]
    pub profile: ChildSessionProfile,
}

fn default_max_children() -> u32 {
    16
}

fn default_max_depth() -> u32 {
    1
}

impl Default for ChildSessionConfig {
    fn default() -> Self {
        Self {
            auto_spawn: false,
            max_children: default_max_children(),
            max_depth: default_max_depth(),
            profile: ChildSessionProfile::default(),
        }
    }
}

/// Declarative profile mapping reverse requests to child backends.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionProfile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ChildSessionRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParentBackendKind {
    Stdio,
    Tcp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionRule {
    #[serde(default)]
    pub when: RuleCondition,
    pub child_backend: ChildBackendTemplate,
    pub debug_request: DebugRequestTemplate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleCondition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exists: Vec<String>,
    #[serde(default, rename = "parentBackend", skip_serializing_if = "Vec::is_empty")]
    pub parent_backend: Vec<ParentBackendKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChildBackendTemplate {
    Stdio {
        cmd: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Tcp {
        host: String,
        port: PortTemplate,
    },
    #[serde(rename = "parentBackend")]
    ParentBackend,
    #[serde(rename = "inheritParentStdio")]
    InheritParentStdio,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PortTemplate {
    Number(u16),
    Template(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DebugRequestTemplate {
    pub request: String,
    pub arguments: Value,
}

impl ChildSessionProfile {
    pub fn from_preset(name: &str) -> Option<Self> {
        match name {
            "fake" => Some(Self::fake_preset()),
            "debugpy" => Some(Self::debugpy_preset()),
            "lldb-dap" => Some(Self::lldb_dap_preset()),
            _ => None,
        }
    }

    pub fn from_json_value(value: Value) -> Result<Self, String> {
        deserialize_profile(value)
    }

    /// Fake adapter / integration tests: inherit parent stdio adapter on `launch`.
    pub fn fake_preset() -> Self {
        Self {
            rules: vec![ChildSessionRule {
                when: RuleCondition {
                    request: Some("launch".into()),
                    exists: vec![],
                    parent_backend: vec![],
                },
                child_backend: ChildBackendTemplate::InheritParentStdio,
                debug_request: DebugRequestTemplate {
                    request: "${request}".into(),
                    arguments: Value::String("${configuration}".into()),
                },
            }],
            unsupported_message: None,
        }
    }

    /// debugpy subprocess connect-back: attach via TCP to `configuration.connect`.
    pub fn debugpy_preset() -> Self {
        Self {
            rules: vec![ChildSessionRule {
                when: RuleCondition {
                    request: Some("attach".into()),
                    exists: vec![
                        "configuration.connect.host".into(),
                        "configuration.connect.port".into(),
                    ],
                    parent_backend: vec![],
                },
                child_backend: ChildBackendTemplate::Tcp {
                    host: "${configuration.connect.host}".into(),
                    port: PortTemplate::Template("${configuration.connect.port}".into()),
                },
                debug_request: DebugRequestTemplate {
                    request: "${request}".into(),
                    arguments: Value::String("${configuration}".into()),
                },
            }],
            unsupported_message: None,
        }
    }

    /// lldb-dap handoff: reuse parent TCP endpoint (stdio parent has no matching rule).
    pub fn lldb_dap_preset() -> Self {
        Self {
            rules: vec![ChildSessionRule {
                when: RuleCondition {
                    request: None,
                    exists: vec![],
                    parent_backend: vec![ParentBackendKind::Tcp],
                },
                child_backend: ChildBackendTemplate::ParentBackend,
                debug_request: DebugRequestTemplate {
                    request: "${request}".into(),
                    arguments: Value::String("${configuration}".into()),
                },
            }],
            unsupported_message: Some(LLDB_DAP_STDIO_UNSUPPORTED_MESSAGE.into()),
        }
    }
}

impl<'de> Deserialize<'de> for ChildSessionProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        deserialize_profile(value).map_err(serde::de::Error::custom)
    }
}

fn deserialize_profile(value: Value) -> Result<ChildSessionProfile, String> {
    match value {
        Value::String(name) => ChildSessionProfile::from_preset(&name).ok_or_else(|| {
            format!(
                "unknown child-session profile preset '{name}'; expected \"fake\", \"debugpy\", \"lldb-dap\", or an explicit rules object"
            )
        }),
        Value::Object(_) => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Explicit {
                #[serde(default)]
                rules: Vec<ChildSessionRule>,
                #[serde(default)]
                unsupported_message: Option<String>,
            }
            let explicit: Explicit = serde_json::from_value(value)
                .map_err(|err| format!("invalid child-session profile object: {err}"))?;
            Ok(ChildSessionProfile {
                rules: explicit.rules,
                unsupported_message: explicit.unsupported_message,
            })
        }
        other => Err(format!(
            "child-session profile must be a preset name or rules object, got {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preset_names_deserialize() {
        let debugpy: ChildSessionProfile = serde_json::from_value(json!("debugpy")).unwrap();
        assert_eq!(debugpy, ChildSessionProfile::debugpy_preset());
        let lldb: ChildSessionProfile = serde_json::from_value(json!("lldb-dap")).unwrap();
        assert_eq!(lldb, ChildSessionProfile::lldb_dap_preset());
    }

    #[test]
    fn explicit_rules_object_deserializes() {
        let value = json!({
            "rules": [{
                "when": { "request": "launch" },
                "childBackend": { "type": "inheritParentStdio" },
                "debugRequest": {
                    "request": "${request}",
                    "arguments": "${configuration}"
                }
            }]
        });
        let profile: ChildSessionProfile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.rules.len(), 1);
    }
}
