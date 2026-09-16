use std::collections::HashMap;

use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
struct SourceBreakpointSpec {
    line: i64,
    condition: Option<String>,
    hit_condition: Option<String>,
    log_message: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ExceptionBreakpointSpec {
    filter: String,
    condition: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingSetBreakpoints {
    source_path: String,
    specs: Vec<SourceBreakpointSpec>,
}

/// Tracks editor/adapter breakpoint state for late-joining control clients.
#[derive(Debug, Default, Clone)]
pub struct BreakpointTracker {
    source_breakpoints: HashMap<String, Vec<SourceBreakpointSpec>>,
    exception_breakpoints: Vec<ExceptionBreakpointSpec>,
    pending_set_breakpoints: HashMap<i64, PendingSetBreakpoints>,
    breakpoint_ids: HashMap<i64, (String, usize)>,
}

impl BreakpointTracker {
    pub fn track_set_breakpoints_request(&mut self, backend_seq: i64, message: &Value) {
        let arguments = message.get("arguments").and_then(Value::as_object);
        let source_path = arguments
            .and_then(|args| args.get("source"))
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let specs = arguments
            .and_then(|args| args.get("breakpoints"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(parse_source_breakpoint)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        self.pending_set_breakpoints
            .insert(backend_seq, PendingSetBreakpoints { source_path, specs });
    }

    pub fn observe_set_breakpoints_response(&mut self, message: &Value) {
        if message.get("success").and_then(Value::as_bool) != Some(true) {
            return;
        }
        let request_seq = message
            .get("request_seq")
            .or_else(|| message.get("requestSeq"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let Some(pending) = self.pending_set_breakpoints.remove(&request_seq) else {
            return;
        };

        let response_breakpoints = message
            .get("body")
            .and_then(|body| body.get("breakpoints"))
            .and_then(Value::as_array);

        let resolved_path = response_breakpoints
            .and_then(|items| items.first())
            .and_then(|bp| bp.get("source"))
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| pending.source_path.clone());

        let specs = if let Some(items) = response_breakpoints {
            merge_breakpoints(items, &pending.specs)
        } else {
            pending.specs
        };

        self.rebuild_source_breakpoints(&resolved_path, specs);
    }

    pub fn observe_breakpoint_event(&mut self, message: &Value) {
        let body = message.get("body").and_then(Value::as_object);
        let reason = body.and_then(|b| b.get("reason")).and_then(Value::as_str);
        let breakpoint = body.and_then(|b| b.get("breakpoint"));
        let Some(bp) = breakpoint else {
            return;
        };

        match reason {
            Some("new") => self.apply_new_breakpoint(bp),
            Some("changed") => self.apply_changed_breakpoint(bp),
            Some("removed") => self.apply_removed_breakpoint(bp),
            _ => {}
        }
    }

    pub fn track_set_exception_breakpoints_request(&mut self, message: &Value) {
        let arguments = message.get("arguments").and_then(Value::as_object);
        let filters = arguments
            .and_then(|args| args.get("filters"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|filter| filter.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let filter_options = arguments
            .and_then(|args| args.get("filterOptions"))
            .and_then(Value::as_array);

        self.exception_breakpoints = filters
            .into_iter()
            .map(|filter| {
                let condition = filter_options.and_then(|options| {
                    options.iter().find_map(|option| {
                        let option_filter = option.get("filterId").and_then(Value::as_str);
                        if option_filter == Some(filter.as_str()) {
                            option
                                .get("condition")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        } else {
                            None
                        }
                    })
                });
                ExceptionBreakpointSpec { filter, condition }
            })
            .collect();
    }

    pub fn snapshot(&self) -> Value {
        let mut files = serde_json::Map::new();
        for (path, specs) in &self.source_breakpoints {
            let entries = specs
                .iter()
                .map(|spec| {
                    json!({
                        "line": spec.line,
                        "condition": spec.condition,
                        "hit_condition": spec.hit_condition,
                        "log_message": spec.log_message,
                    })
                })
                .collect::<Vec<_>>();
            files.insert(path.clone(), Value::Array(entries));
        }

        json!({
            "breakpoints": Value::Object(files),
            "exception_breakpoints": self
                .exception_breakpoints
                .iter()
                .map(|entry| {
                    json!({
                        "filter": entry.filter,
                        "condition": entry.condition,
                    })
                })
                .collect::<Vec<_>>(),
        })
    }

    fn rebuild_source_breakpoints(&mut self, path: &str, specs: Vec<SourceBreakpointSpec>) {
        self.breakpoint_ids
            .retain(|_, (stored_path, _)| stored_path != path);
        if specs.is_empty() {
            self.source_breakpoints.remove(path);
        } else {
            self.source_breakpoints.insert(path.to_string(), specs);
        }
    }

    fn apply_new_breakpoint(&mut self, bp: &Value) {
        let id = bp.get("id").and_then(Value::as_i64);
        let path = bp_source_path(bp);
        let line = bp.get("line").and_then(Value::as_i64);
        if path.is_empty() || line.is_none() {
            return;
        }
        let path = path.to_string();
        let line = line.unwrap_or(0);
        if line < 0 {
            return;
        }

        let spec = SourceBreakpointSpec {
            line,
            condition: bp
                .get("condition")
                .and_then(Value::as_str)
                .map(str::to_string),
            hit_condition: bp
                .get("hitCondition")
                .and_then(Value::as_str)
                .map(str::to_string),
            log_message: bp
                .get("logMessage")
                .and_then(Value::as_str)
                .map(str::to_string),
        };

        let entries = self.source_breakpoints.entry(path.clone()).or_default();
        if let Some((_, index)) = id.and_then(|bp_id| self.breakpoint_ids.get(&bp_id).cloned()) {
            if index < entries.len() {
                entries[index] = spec;
                return;
            }
        }
        let index = entries.len();
        entries.push(spec);
        if let Some(bp_id) = id {
            self.breakpoint_ids.insert(bp_id, (path, index));
        }
    }

    fn apply_changed_breakpoint(&mut self, bp: &Value) {
        let id = bp.get("id").and_then(Value::as_i64);
        if let Some(bp_id) = id {
            if let Some((path, index)) = self.breakpoint_ids.get(&bp_id).cloned() {
                if let Some(entries) = self.source_breakpoints.get_mut(&path) {
                    if index < entries.len() {
                        if let Some(line) = bp.get("line").and_then(Value::as_i64) {
                            entries[index].line = line;
                        }
                        if bp.get("condition").is_some() {
                            entries[index].condition = bp
                                .get("condition")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        if bp.get("hitCondition").is_some() {
                            entries[index].hit_condition = bp
                                .get("hitCondition")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        if bp.get("logMessage").is_some() {
                            entries[index].log_message = bp
                                .get("logMessage")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        return;
                    }
                }
            }
        }
        self.apply_new_breakpoint(bp);
    }

    fn apply_removed_breakpoint(&mut self, bp: &Value) {
        let id = bp.get("id").and_then(Value::as_i64);
        let Some(bp_id) = id else {
            return;
        };
        let Some((path, index)) = self.breakpoint_ids.remove(&bp_id) else {
            return;
        };
        if let Some(entries) = self.source_breakpoints.get_mut(&path) {
            if index < entries.len() {
                entries.remove(index);
            }
            if entries.is_empty() {
                self.source_breakpoints.remove(&path);
            }
        }
        for (stored_path, stored_index) in self.breakpoint_ids.values_mut() {
            if stored_path == &path && *stored_index > index {
                *stored_index -= 1;
            }
        }
    }
}

fn parse_source_breakpoint(value: &Value) -> Option<SourceBreakpointSpec> {
    let line = value.get("line").and_then(Value::as_i64)?;
    Some(SourceBreakpointSpec {
        line,
        condition: value
            .get("condition")
            .and_then(Value::as_str)
            .map(str::to_string),
        hit_condition: value
            .get("hitCondition")
            .and_then(Value::as_str)
            .map(str::to_string),
        log_message: value
            .get("logMessage")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn merge_breakpoints(
    response: &[Value],
    request_specs: &[SourceBreakpointSpec],
) -> Vec<SourceBreakpointSpec> {
    if response.is_empty() {
        return request_specs.to_vec();
    }

    response
        .iter()
        .enumerate()
        .filter_map(|(index, bp)| {
            if bp.get("verified").and_then(Value::as_bool) == Some(false) {
                return None;
            }
            let request = request_specs.get(index);
            let line = bp
                .get("line")
                .and_then(Value::as_i64)
                .or_else(|| request.map(|spec| spec.line))?;
            Some(SourceBreakpointSpec {
                line,
                condition: bp
                    .get("condition")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| request.and_then(|spec| spec.condition.clone())),
                hit_condition: bp
                    .get("hitCondition")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| request.and_then(|spec| spec.hit_condition.clone())),
                log_message: bp
                    .get("logMessage")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| request.and_then(|spec| spec.log_message.clone())),
            })
        })
        .collect()
}

fn bp_source_path(bp: &Value) -> &str {
    bp.get("source")
        .and_then(|source| source.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{request, response};

    #[test]
    fn tracks_set_breakpoints_request_and_response() {
        let mut tracker = BreakpointTracker::default();
        tracker.track_set_breakpoints_request(
            7,
            &request(
                7,
                "setBreakpoints",
                Some(json!({
                    "source": { "path": "/fake/main.py" },
                    "breakpoints": [
                        { "line": 10, "condition": "x > 1" },
                        { "line": 20 }
                    ],
                })),
            ),
        );

        tracker.observe_set_breakpoints_response(&response(
            8,
            7,
            true,
            Some(json!({
                "breakpoints": [
                    { "verified": true, "line": 10 },
                    { "verified": true, "line": 20 },
                ]
            })),
        ));

        let snapshot = tracker.snapshot();
        let entries = snapshot["breakpoints"]["/fake/main.py"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["line"], 10);
        assert_eq!(entries[0]["condition"], "x > 1");
    }

    #[test]
    fn falls_back_to_request_specs_when_response_body_empty() {
        let mut tracker = BreakpointTracker::default();
        tracker.track_set_breakpoints_request(
            3,
            &request(
                3,
                "setBreakpoints",
                Some(json!({
                    "source": { "path": "/fake/main.py" },
                    "breakpoints": [{ "line": 5 }],
                })),
            ),
        );
        tracker.observe_set_breakpoints_response(&response(4, 3, true, None));

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot["breakpoints"]["/fake/main.py"][0]["line"], 5);
    }

    #[test]
    fn tracks_exception_breakpoints_request() {
        let mut tracker = BreakpointTracker::default();
        tracker.track_set_exception_breakpoints_request(&request(
            1,
            "setExceptionBreakpoints",
            Some(json!({
                "filters": ["uncaught"],
                "filterOptions": [{ "filterId": "uncaught", "condition": "err != nil" }],
            })),
        ));

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot["exception_breakpoints"][0]["filter"], "uncaught");
        assert_eq!(
            snapshot["exception_breakpoints"][0]["condition"],
            "err != nil"
        );
    }
}
