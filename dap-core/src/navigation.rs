use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Debugger navigation commands exposed by the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationType {
    Continue,
    StepOver,
    StepIn,
    StepOut,
    Pause,
    StepBack,
    ReverseContinue,
}

impl NavigationType {
    pub fn dap_command(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::StepOver => "next",
            Self::StepIn => "stepIn",
            Self::StepOut => "stepOut",
            Self::Pause => "pause",
            Self::StepBack => "stepBack",
            Self::ReverseContinue => "reverseContinue",
        }
    }

    pub fn waits_for_stop(self) -> bool {
        matches!(self, Self::Continue | Self::Pause | Self::ReverseContinue)
    }

    pub fn arguments(self, thread_id: i64) -> Value {
        match self {
            Self::Continue | Self::ReverseContinue => {
                json!({ "threadId": thread_id })
            }
            Self::StepOver | Self::StepIn | Self::StepOut | Self::StepBack => {
                json!({ "threadId": thread_id })
            }
            Self::Pause => json!({ "threadId": thread_id }),
        }
    }
}

impl fmt::Display for NavigationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Continue => write!(f, "continue"),
            Self::StepOver => write!(f, "step_over"),
            Self::StepIn => write!(f, "step_in"),
            Self::StepOut => write!(f, "step_out"),
            Self::Pause => write!(f, "pause"),
            Self::StepBack => write!(f, "step_back"),
            Self::ReverseContinue => write!(f, "reverse_continue"),
        }
    }
}

impl FromStr for NavigationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.replace('-', "_").to_ascii_lowercase().as_str() {
            "continue" => Ok(Self::Continue),
            "next" | "step_over" | "stepover" => Ok(Self::StepOver),
            "step_in" | "stepin" => Ok(Self::StepIn),
            "step_out" | "stepout" => Ok(Self::StepOut),
            "pause" => Ok(Self::Pause),
            "step_back" | "stepback" => Ok(Self::StepBack),
            "reverse_continue" | "reversecontinue" => Ok(Self::ReverseContinue),
            other => Err(format!("unknown navigation type: {other}")),
        }
    }
}

/// Result of a navigation command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    pub navigation_type: NavigationType,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
}
