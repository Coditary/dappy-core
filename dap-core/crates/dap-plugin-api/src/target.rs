use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::PluginError;
use crate::launch::{InitSpec, InitStep};
use crate::template::{TemplateContext, resolve_value};

fn default_host() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    1234
}

/// gdb-remote connection endpoint for an RSP target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetConnect {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for TargetConnect {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// Process spawn specification for an RSP backend (QEMU, gdbserver, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSpawn {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Declarative RSP target metadata (`target.yaml`), parallel to `plugin.yaml` for DAP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, rename = "targetTypes")]
    pub target_types: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spawn: Option<TargetSpawn>,
    #[serde(default)]
    pub connect: Option<TargetConnect>,
    /// Fields merged into the builtin gdb-remote attach request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<InitSpec>,
    /// Manifest file path (set by the loader, not serialized).
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

/// Spawn command with templates resolved for a concrete program path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTargetSpawn {
    pub command: String,
    pub args: Vec<String>,
}

/// Fully resolved target ready for spawn + gdb-remote attach.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub spawn: Option<ResolvedTargetSpawn>,
    pub host: String,
    pub port: u16,
    pub attach_fields: Value,
    pub init_steps: Vec<InitStep>,
}

impl TargetManifest {
    pub fn from_yaml(text: &str, path: &Path) -> Result<Self, PluginError> {
        let mut manifest: Self = serde_yaml::from_str(text).map_err(|source| PluginError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        manifest.source_path = Some(path.to_path_buf());
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), PluginError> {
        if self.id.trim().is_empty() {
            return Err(PluginError::InvalidManifest("id must not be empty".into()));
        }
        if self.name.trim().is_empty() {
            return Err(PluginError::InvalidManifest("name must not be empty".into()));
        }
        if self.version.trim().is_empty() {
            return Err(PluginError::InvalidManifest("version must not be empty".into()));
        }
        if self.target_types.is_empty() {
            return Err(PluginError::InvalidManifest(
                "at least one targetType is required".into(),
            ));
        }
        if self.spawn.is_some() {
            let spawn = self.spawn.as_ref().expect("checked");
            if spawn.command.trim().is_empty() {
                return Err(PluginError::InvalidManifest(
                    "spawn.command must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn resolve(&self, program: &str) -> Result<ResolvedTarget, PluginError> {
        let ctx = TemplateContext::new(program, self.id.clone());
        let connect = self.connect.clone().unwrap_or_default();
        let spawn = self
            .spawn
            .as_ref()
            .map(|spawn| resolve_spawn(spawn, &ctx))
            .transpose()?;
        let attach_fields = merge_attach_fields(self.attach.as_ref(), &connect, &ctx);
        let init_steps = self
            .init
            .as_ref()
            .map(|init| init.steps.clone())
            .unwrap_or_default();
        Ok(ResolvedTarget {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            spawn,
            host: connect.host,
            port: connect.port,
            attach_fields,
            init_steps,
        })
    }
}

impl ResolvedTarget {
    pub fn attach_arguments(&self) -> Value {
        let args = self.attach_fields.clone();
        match args {
            Value::Object(mut map) => {
                map.entry("host".to_string())
                    .or_insert_with(|| json!(self.host));
                map.entry("port".to_string())
                    .or_insert_with(|| json!(self.port));
                Value::Object(map)
            }
            _ => json!({
                "host": self.host,
                "port": self.port,
            }),
        }
    }
}

fn resolve_spawn(spawn: &TargetSpawn, ctx: &TemplateContext) -> Result<ResolvedTargetSpawn, PluginError> {
    let command = resolve_required_string(&spawn.command, ctx, "spawn.command")?;
    let args = spawn
        .args
        .iter()
        .map(|arg| resolve_required_string(arg, ctx, "spawn.args"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ResolvedTargetSpawn { command, args })
}

fn merge_attach_fields(
    attach: Option<&Value>,
    connect: &TargetConnect,
    ctx: &TemplateContext,
) -> Value {
    let mut args = json!({});
    if let Some(patch) = attach {
        if let Some(resolved) = resolve_value(patch, ctx) {
            merge_json_object(&mut args, resolved);
        }
    }
    if let Value::Object(map) = &mut args {
        map.entry("host".to_string())
            .or_insert_with(|| json!(connect.host));
        map.entry("port".to_string())
            .or_insert_with(|| json!(connect.port));
    }
    args
}

fn resolve_required_string(
    template: &str,
    ctx: &TemplateContext,
    field: &str,
) -> Result<String, PluginError> {
    let resolved = resolve_value(&json!(template), ctx).ok_or_else(|| {
        PluginError::InvalidManifest(format!("failed to resolve template for {field}"))
    })?;
    resolved
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| PluginError::InvalidManifest(format!("{field} must resolve to a string")))
}

fn merge_json_object(target: &mut Value, patch: Value) {
    let Value::Object(patch_map) = patch else {
        return;
    };
    let Value::Object(target_map) = target else {
        return;
    };
    for (key, value) in patch_map {
        target_map.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_yaml() -> &'static str {
        r#"
id: gdbserver
name: GDB Server
version: 0.1.0
targetTypes: [gdb-remote]
spawn:
  command: gdbserver
  args: [":1234", "${program.abs}"]
connect:
  host: 127.0.0.1
  port: 1234
attach:
  decompiler: auto
"#
    }

    #[test]
    fn loads_and_resolves_target_manifest() {
        let manifest =
            TargetManifest::from_yaml(sample_manifest_yaml(), Path::new("target.yaml")).expect("load");
        let resolved = manifest.resolve("/tmp/demo.elf").expect("resolve");
        assert_eq!(resolved.id, "gdbserver");
        assert_eq!(resolved.spawn.as_ref().map(|s| s.command.as_str()), Some("gdbserver"));
        let attach = resolved.attach_arguments();
        assert_eq!(attach["decompiler"], json!("auto"));
        assert_eq!(attach["port"], json!(1234));
    }

    #[test]
    fn attach_only_manifest_has_no_spawn() {
        let manifest = TargetManifest::from_yaml(
            r#"
id: rsp-attach
name: Attach
version: 0.1.0
targetTypes: [gdb-remote]
connect:
  host: qemu
  port: 4321
"#,
            Path::new("target.yaml"),
        )
        .expect("load");
        let resolved = manifest.resolve("fw.elf").expect("resolve");
        assert!(resolved.spawn.is_none());
        assert_eq!(resolved.host, "qemu");
    }
}
