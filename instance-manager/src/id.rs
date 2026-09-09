use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque identifier for a managed instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(String);

impl InstanceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for InstanceId {
    type Err = InstanceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InstanceError::Other("instance id cannot be empty".into()));
        }
        Ok(Self(s.to_owned()))
    }
}

use crate::error::InstanceError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_display_instance_id() {
        let id = InstanceId::generate();
        assert!(!id.as_str().is_empty());
        assert_eq!(id.to_string(), id.as_str());
    }

    #[test]
    fn rejects_empty_instance_id() {
        assert!(InstanceId::from_str("").is_err());
    }

    #[test]
    fn accepts_custom_instance_id() {
        let id = InstanceId::new("session-1");
        assert_eq!(id.as_str(), "session-1");
    }
}
