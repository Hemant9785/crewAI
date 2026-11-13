//! Security status for repository-question execution.
//!
//! A child process being separate is not the same thing as being read-only.
//! This module is the policy gate used by `/check` before it may construct a
//! runner request.  It deliberately refuses to infer safety from a prompt,
//! filesystem permissions that were not installed by this process, or a
//! display label.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkPolicy {
    Denied,
    ExplicitlyRequired,
    Unavailable,
}

impl NetworkPolicy {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Denied => "denied by execution policy",
            Self::ExplicitlyRequired => "explicitly required by request",
            Self::Unavailable => "network isolation unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IsolationStatus {
    EnforcedReadOnly { mechanism: &'static str },
    BestEffortReadOnly { reason: &'static str },
    Unavailable { reason: &'static str },
}

impl IsolationStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::EnforcedReadOnly { .. } => "enforced read-only",
            Self::BestEffortReadOnly { .. } => "best-effort read-only",
            Self::Unavailable { .. } => "failed to establish isolation",
        }
    }

    pub(crate) const fn is_enforced(self) -> bool {
        matches!(self, Self::EnforcedReadOnly { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckExecutionPolicy {
    pub repository_path: PathBuf,
    pub isolation: IsolationStatus,
    pub network: NetworkPolicy,
    pub confirmation_required: bool,
}

/// Return the policy for `/check` on this build and operating system.
///
/// No read-only boundary is currently installed by Herdr itself.  In
/// particular, ordinary Windows ACLs, a read-only prompt, and a separate
/// process are not treated as enforcement.  Keeping the result explicit lets
/// the command layer disable or confirmation-gate the feature instead of
/// making a false security claim.
pub(crate) fn assess_check_policy(repository_path: &Path) -> CheckExecutionPolicy {
    CheckExecutionPolicy {
        repository_path: repository_path.to_path_buf(),
        isolation: IsolationStatus::Unavailable {
            reason: "this build has no verified OS-level read-only filesystem boundary",
        },
        network: NetworkPolicy::Unavailable,
        confirmation_required: true,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckAuthorization {
    Enforced,
    ConfirmedBestEffort,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BoundaryError {
    IsolationUnavailable(&'static str),
    ConfirmationRequired,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IsolationUnavailable(reason) => {
                write!(formatter, "repository isolation unavailable: {reason}")
            }
            Self::ConfirmationRequired => formatter.write_str(
                "repository execution requires explicit confirmation because isolation is not enforced",
            ),
        }
    }
}

impl std::error::Error for BoundaryError {}

/// Authorize execution only when the caller can truthfully report its status.
///
/// `allow_unisolated` is intentionally opt-in and is not used by the normal
/// `/check` path.  It exists for a future explicit confirmation flow and
/// returns `ConfirmedBestEffort`, never `Enforced`.
pub(crate) fn authorize_check(
    policy: &CheckExecutionPolicy,
    allow_unisolated: bool,
) -> Result<CheckAuthorization, BoundaryError> {
    if policy.isolation.is_enforced() {
        return Ok(CheckAuthorization::Enforced);
    }
    if policy.confirmation_required && !allow_unisolated {
        return Err(BoundaryError::ConfirmationRequired);
    }
    match policy.isolation {
        IsolationStatus::BestEffortReadOnly { .. } => Ok(CheckAuthorization::ConfirmedBestEffort),
        IsolationStatus::Unavailable { reason } => {
            if allow_unisolated {
                Ok(CheckAuthorization::ConfirmedBestEffort)
            } else {
                Err(BoundaryError::IsolationUnavailable(reason))
            }
        }
        IsolationStatus::EnforcedReadOnly { .. } => Ok(CheckAuthorization::Enforced),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_check_policy_does_not_claim_read_only() {
        let policy = assess_check_policy(Path::new(r"E:\repo"));
        assert_eq!(policy.isolation.label(), "failed to establish isolation");
        assert!(!policy.isolation.is_enforced());
        assert_eq!(policy.network, NetworkPolicy::Unavailable);
        assert!(policy.confirmation_required);
        assert!(matches!(
            authorize_check(&policy, false),
            Err(BoundaryError::ConfirmationRequired)
        ));
    }

    #[test]
    fn unisolated_execution_is_explicitly_marked_after_confirmation() {
        let policy = assess_check_policy(Path::new(r"E:\repo"));
        assert_eq!(
            authorize_check(&policy, true),
            Ok(CheckAuthorization::ConfirmedBestEffort)
        );
        assert!(!policy.isolation.is_enforced());
    }

    #[test]
    fn enforced_status_is_the_only_status_that_can_be_reported_as_safe() {
        let policy = CheckExecutionPolicy {
            repository_path: PathBuf::from(r"E:\repo"),
            isolation: IsolationStatus::EnforcedReadOnly {
                mechanism: "test boundary",
            },
            network: NetworkPolicy::Denied,
            confirmation_required: false,
        };
        assert_eq!(
            authorize_check(&policy, false),
            Ok(CheckAuthorization::Enforced)
        );
    }
}
