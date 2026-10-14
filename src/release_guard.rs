use crate::infrai_errors::{CaptureReceipt, ExceptionPayload, InfraiError, InfraiErrors};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
pub struct BuildEvent {
    pub run_id: String,
    pub agent: String,
    pub step: String,
    pub commit_sha: String,
    pub status: BuildStatus,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "release", rename_all = "snake_case")]
pub enum ReleaseOperation {
    Proceed {
        commit_sha: String,
    },
    Block {
        commit_sha: String,
        diagnostic: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("failed build event requires a diagnostic")]
    MissingDiagnostic,
    #[error(transparent)]
    Tracking(#[from] InfraiError),
}

pub fn decide_release(event: &BuildEvent) -> Result<ReleaseOperation, GuardError> {
    match event.status {
        BuildStatus::Passed => Ok(ReleaseOperation::Proceed {
            commit_sha: event.commit_sha.clone(),
        }),
        BuildStatus::Failed => {
            let diagnostic = event
                .diagnostic
                .clone()
                .filter(|text| !text.trim().is_empty())
                .ok_or(GuardError::MissingDiagnostic)?;
            Ok(ReleaseOperation::Block {
                commit_sha: event.commit_sha.clone(),
                diagnostic,
            })
        }
    }
}

pub async fn evaluate_and_track(
    errors: &InfraiErrors,
    event: BuildEvent,
) -> Result<(ReleaseOperation, Option<CaptureReceipt>), GuardError> {
    let operation = decide_release(&event)?;
    let receipt = match &operation {
        ReleaseOperation::Proceed { .. } => None,
        ReleaseOperation::Block { diagnostic, .. } => {
            let exception = ExceptionPayload {
                title: format!("{}/{} build failed", event.agent, event.step),
                message: diagnostic.clone(),
                level: "error".into(),
                fingerprint: vec![event.agent.clone(), event.step.clone()],
                exception: diagnostic.clone(),
                context: json!({
                    "run_id": event.run_id,
                    "agent": event.agent,
                    "step": event.step,
                    "commit_sha": event.commit_sha,
                }),
            };
            let key = format!("build-failure:{}:{}", event.run_id, event.step);
            Some(errors.capture(&exception, &key).await?)
        }
    };
    Ok((operation, receipt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_build_blocks_the_exact_commit_with_its_diagnostic() {
        let event = BuildEvent {
            run_id: "run-1042".into(),
            agent: "release-agent".into(),
            step: "cargo-test".into(),
            commit_sha: "a1b2c3d".into(),
            status: BuildStatus::Failed,
            diagnostic: Some("snapshot assertion changed".into()),
        };

        assert_eq!(
            decide_release(&event).unwrap(),
            ReleaseOperation::Block {
                commit_sha: "a1b2c3d".into(),
                diagnostic: "snapshot assertion changed".into(),
            }
        );
    }
}
