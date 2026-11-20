use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    NotImplemented,
    Implemented,
    FixtureVerified,
    RealVerified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Ready,
    NeedsSetup,
    Unsupported,
    Degraded,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub id: String,
    pub evidence: Evidence,
    pub availability: Availability,
    pub reason: String,
    pub execution_mode: Option<String>,
    pub requires_foreground: bool,
    pub requires_user_action: bool,
}

impl Capability {
    pub fn pending(id: &str, reason: &str) -> Self {
        Self {
            id: id.into(),
            evidence: Evidence::NotImplemented,
            availability: Availability::Unsupported,
            reason: reason.into(),
            execution_mode: None,
            requires_foreground: false,
            requires_user_action: false,
        }
    }
}
