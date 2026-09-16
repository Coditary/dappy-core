//! Proxy-specific DAP extensions (plugin metadata for clients).

use dap_plugin_api::PluginManifest;
use dap_protocol::{Message, Request, Response};
use serde_json::{Value, json};

/// DAP request command for plugin metadata lookup.
pub const DAP_PROXY_PLUGIN_INFO_COMMAND: &str = "dapProxyPluginInfo";

/// Capability flag advertised in the proxied `initialize` response body.
pub const SUPPORTS_DAP_PROXY_PLUGIN_INFO_REQUEST: &str = "supportsDapProxyPluginInfoRequest";

/// Active adapter plugin metadata for a multiplexed proxy session.
#[derive(Debug, Clone)]
pub struct ProxyPluginContext {
    pub manifest: PluginManifest,
}

impl ProxyPluginContext {
    pub fn new(manifest: PluginManifest) -> Self {
        Self { manifest }
    }

    pub fn plugin_info_body(&self) -> Value {
        let mut body = json!({
            "pluginId": self.manifest.id,
            "adapterId": self.manifest.id,
            "name": self.manifest.name,
            "version": self.manifest.version,
            "launchTypes": self.manifest.launch_types,
            "fileExtensions": self.manifest.file_extensions,
        });
        if let Some(spec) = &self.manifest.attachment {
            body["attachmentSpec"] = json!(spec);
        }
        if let Some(path) = self.manifest.attachment_path() {
            body["attachment"] = json!(path.display().to_string());
        }
        body
    }
}

/// Inject proxy capability into an adapter `initialize` response.
pub fn augment_initialize_response(message: &mut Message) {
    let Message::Response(response) = message else {
        return;
    };
    if response.command.as_deref() != Some("initialize") {
        return;
    }
    let body = response.body.get_or_insert_with(|| json!({}));
    if let Value::Object(map) = body {
        map.insert(
            SUPPORTS_DAP_PROXY_PLUGIN_INFO_REQUEST.to_string(),
            Value::Bool(true),
        );
    }
}

/// Answer a client-originated `dapProxyPluginInfo` request locally.
pub fn handle_plugin_info_request(request: &Request, context: &ProxyPluginContext) -> Message {
    Message::Response(Response {
        seq: 0,
        request_seq: request.seq,
        success: true,
        command: Some(DAP_PROXY_PLUGIN_INFO_COMMAND.to_string()),
        message: None,
        body: Some(context.plugin_info_body()),
    })
}

/// Returns a response when the proxy handles the request without forwarding upstream.
pub fn try_handle_client_request(
    message: &Message,
    context: &ProxyPluginContext,
) -> Option<Message> {
    match message {
        Message::Request(request) if request.command == DAP_PROXY_PLUGIN_INFO_COMMAND => {
            Some(handle_plugin_info_request(request, context))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dap_plugin_api::{AdapterSpawn, PluginManifest, SpawnTransport};
    use dap_protocol::Response;

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: "python".into(),
            name: "Python".into(),
            version: "0.1.0".into(),
            languages: vec!["python".into()],
            launch_types: vec!["python".into()],
            file_extensions: vec!["py".into()],
            adapter: AdapterSpawn {
                transport: SpawnTransport::Stdio,
                command: "python3".into(),
                args: vec![],
            },
            initialize: None,
            launch: None,
            init: None,
            attachment: Some("attachment.yaml".into()),
            source_path: Some(std::path::PathBuf::from(
                "/plugins/builtin/python/plugin.yaml",
            )),
        }
    }

    #[test]
    fn augment_initialize_injects_capability() {
        let mut message = Message::Response(Response {
            seq: 2,
            request_seq: 1,
            success: true,
            command: Some("initialize".into()),
            message: None,
            body: Some(json!({ "adapterID": "debugpy" })),
        });
        augment_initialize_response(&mut message);
        let Message::Response(response) = &message else {
            panic!("expected response");
        };
        let body = response.body.as_ref().expect("body");
        assert_eq!(
            body[SUPPORTS_DAP_PROXY_PLUGIN_INFO_REQUEST],
            Value::Bool(true)
        );
    }

    #[test]
    fn plugin_info_body_includes_attachment() {
        let context = ProxyPluginContext::new(sample_manifest());
        let body = context.plugin_info_body();
        assert_eq!(body["pluginId"], "python");
        assert_eq!(body["attachmentSpec"], "attachment.yaml");
        assert!(
            body["attachment"]
                .as_str()
                .unwrap()
                .ends_with("python/attachment.yaml")
        );
    }

    #[test]
    fn try_handle_returns_plugin_info_response() {
        let context = ProxyPluginContext::new(sample_manifest());
        let request = Message::Request(Request {
            seq: 7,
            command: DAP_PROXY_PLUGIN_INFO_COMMAND.into(),
            arguments: None,
        });
        let response = try_handle_client_request(&request, &context).expect("handled");
        let Message::Response(resp) = response else {
            panic!("expected response");
        };
        assert_eq!(resp.request_seq, 7);
        assert_eq!(resp.body.as_ref().unwrap()["pluginId"], "python");
    }
}
