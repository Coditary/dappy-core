use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::manifest::PluginManifest;
use crate::template::{TemplateContext, resolve_value};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeSpec {
    #[serde(rename = "adapterID", default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchSpec {
    #[serde(default = "default_launch_request")]
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Value>,
}

fn default_launch_request() -> String {
    "launch".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitSpec {
    #[serde(default)]
    pub steps: Vec<InitStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitStep {
    pub request: Option<String>,
    #[serde(default)]
    pub use_launch_arguments: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for_response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_entry: Option<RecordEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordEntry {
    pub path: String,
    pub line: i64,
}

impl PluginManifest {
    pub fn template_context(&self, program: &str) -> TemplateContext {
        TemplateContext::new(program, self.id.clone())
    }

    pub fn build_initialize_arguments(
        &self,
        client_id: &str,
        fallback_adapter_id: Option<&str>,
    ) -> Value {
        let adapter_id = self
            .initialize
            .as_ref()
            .and_then(|spec| spec.adapter_id.clone())
            .or_else(|| fallback_adapter_id.map(str::to_string))
            .unwrap_or_else(|| self.id.clone());

        let mut args = json!({
            "clientID": client_id,
            "adapterID": adapter_id,
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path",
            "supportsRunInTerminalRequest": true,
        });

        if let Some(defaults) = self
            .initialize
            .as_ref()
            .and_then(|spec| spec.defaults.as_ref())
        {
            merge_object(&mut args, defaults, &self.template_context(""));
        }
        args
    }

    pub fn build_launch_arguments(&self, program: &str) -> Value {
        let ctx = self.template_context(program);
        let launch = self.launch.as_ref();

        if launch.is_none() {
            return json!({
                "program": ctx
                    .program_abs
                    .clone()
                    .unwrap_or_else(|| ctx.program.clone())
            });
        }

        let launch = launch.expect("checked");
        let mut args = json!({});
        if let Some(defaults) = &launch.defaults {
            merge_object(&mut args, defaults, &ctx);
        }
        if let Some(fields) = &launch.fields {
            merge_object(&mut args, fields, &ctx);
        }
        if args.as_object().is_some_and(|map| map.is_empty()) {
            args["program"] = json!(ctx.program_abs.clone().unwrap_or(ctx.program));
        }
        args
    }

    pub fn launch_request_name(&self) -> &str {
        self.launch
            .as_ref()
            .map(|launch| launch.request.as_str())
            .unwrap_or("launch")
    }

    pub fn init_steps(&self) -> &[InitStep] {
        self.init
            .as_ref()
            .map(|init| init.steps.as_slice())
            .unwrap_or(&[])
    }
}

fn merge_object(target: &mut Value, patch: &Value, ctx: &TemplateContext) {
    let Some(resolved) = resolve_value(patch, ctx) else {
        return;
    };
    let Value::Object(resolved_map) = resolved else {
        return;
    };
    let Value::Object(target_map) = target else {
        return;
    };
    for (key, value) in resolved_map {
        target_map.insert(key, value);
    }
}
