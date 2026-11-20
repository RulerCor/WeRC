use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    AwaitingApproval,
    Queued,
    Preparing,
    AwaitingUser,
    Submitting,
    Succeeded,
    Failed,
    Unknown,
    Canceled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub account_id: String,
    pub creator_id: String,
    pub kind: String,
    pub asset_id: String,
    pub request_digest: String,
    pub state: OperationState,
    pub created_at: String,
    pub updated_at: String,
    pub result: Option<serde_json::Value>,
    pub error_code: Option<String>,
}

impl OperationState {
    // 纯本地分析没有对外提交阶段，不能用于发信/发布状态迁移。
    pub fn finish_local_analysis(self, next: Self) -> AppResult<Self> {
        if self == Self::Preparing && matches!(next, Self::Succeeded | Self::Failed) {
            Ok(next)
        } else {
            Err(AppError::new(
                "INVALID_TRANSITION",
                "本地分析无法从当前状态完成",
            ))
        }
    }

    pub fn transition(self, next: Self) -> AppResult<Self> {
        use OperationState::*;
        let valid = matches!(
            (self, next),
            (AwaitingApproval, Queued | Canceled | Expired)
                | (Queued, Preparing | Canceled | Expired)
                | (Preparing, AwaitingUser | Submitting | Failed | Canceled)
                | (AwaitingUser, Submitting | Canceled | Expired)
                | (Submitting, Succeeded | Failed | Unknown)
                | (Unknown, Succeeded | Failed)
        );
        if valid {
            Ok(next)
        } else {
            Err(AppError::new("INVALID_TRANSITION", "操作状态不允许此转换"))
        }
    }

    pub fn after_restart(self) -> Self {
        if self == Self::Submitting {
            Self::Unknown
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OperationState::*;
    #[test]
    fn unknown_never_queues_again() {
        assert!(Unknown.transition(Queued).is_err());
        assert!(Unknown.transition(Submitting).is_err());
        assert_eq!(Submitting.after_restart(), Unknown);
        assert!(Unknown.transition(Succeeded).is_ok());
    }
    #[test]
    fn submitted_cannot_be_locally_canceled() {
        assert!(Submitting.transition(Canceled).is_err());
        assert!(Succeeded.transition(Queued).is_err());
        assert!(AwaitingApproval.transition(Submitting).is_err());
    }
}
