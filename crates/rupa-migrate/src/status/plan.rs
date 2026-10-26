use rupa_core::DialectId;

use super::Status;
use crate::{ApplyOptions, Catalog, CatalogEntry, Plan, ValidationError, Version};

/// What apply would do on a target: its status and the exact steps it
/// would run, if nothing blocks it.
#[derive(Debug, Clone)]
pub struct ApplyPlan<'c> {
    /// The target's status; apply runs `steps` only when its blockers are
    /// empty. Late blockers the options permit are moved to `late`.
    pub status: Status<'c>,
    /// The pending entries, in the order apply runs them.
    pub steps: Vec<Step<'c>>,
    /// The pending versions below an installed version that the options
    /// permit; apply runs them with the other steps and reports them late.
    pub late: Vec<Version>,
}

/// One pending entry apply would run: its identity, policy and body.
#[derive(Debug, Clone)]
pub struct Step<'c> {
    /// The entry: version, checksum and metadata, policy included.
    pub entry: &'c CatalogEntry,
    /// What its body sends.
    pub execution: Execution,
}

/// What a pending entry's body sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Execution {
    /// A planned body: one script per plan operation, exactly as apply sends
    /// them for the session's dialect.
    Static(Vec<String>),
    /// An imperative Rust body, opaque until it runs.
    Opaque,
}

impl Execution {
    /// What a body sends: `plan`'s operations rendered for `dialect`, or
    /// opaque without a plan.
    pub(crate) fn of(plan: Option<&Plan>, dialect: DialectId) -> Result<Self, ValidationError> {
        let Some(plan) = plan else {
            return Ok(Self::Opaque);
        };
        let scripts = plan
            .operations()
            .iter()
            .map(|operation| crate::catalog::render(operation, dialect))
            .collect::<Result<_, _>>()?;

        Ok(Self::Static(scripts))
    }
}

impl<M> Catalog<'_, M> {
    /// The steps an apply with `options` would run for `status`, rendered
    /// for `dialect`.
    pub fn plan_for<'c>(
        &self,
        mut status: Status<'c>,
        dialect: DialectId,
        options: &ApplyOptions,
    ) -> Result<ApplyPlan<'c>, ValidationError> {
        let late = options.order.permit(&mut status.blockers);
        let steps = status
            .pending
            .iter()
            .map(|entry| {
                let execution = Execution::of(entry.plan.as_ref(), dialect)?;

                Ok(Step { entry, execution })
            })
            .collect::<Result<_, ValidationError>>()?;

        Ok(ApplyPlan {
            status,
            steps,
            late,
        })
    }
}
