//! Per-return diagnostic values, never custody or permission to replay a grant.
//! Keep the original creation cause when cleanup cannot finish; that complete
//! grant remains retained by PreparedDmaDomain's existing fallback.

use super::dma;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Stream,
    Backend,
    ResetAdmission,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Backend(dma::Error),
    ResetRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    /// Internal preparation error before its enclosing grant has been consumed.
    NotAttempted,
    /// Explicit cancellation finished the original preparation and admission.
    Confirmed,
    /// Failed or uncertain cancellation retained the complete original grant.
    Retained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    pub stage: Stage,
    pub cause: Cause,
    pub cleanup: Cleanup,
}

impl Failure {
    pub(super) fn backend(stage: Stage, cause: dma::Error) -> Self {
        Self {
            stage,
            cause: Cause::Backend(cause),
            cleanup: Cleanup::NotAttempted,
        }
    }

    pub(super) fn reset(stage: Stage) -> Self {
        Self {
            stage,
            cause: Cause::ResetRejected,
            cleanup: Cleanup::NotAttempted,
        }
    }
}
