use crate::Version;

/// An operator's assertion that an unmanaged target already holds the
/// schema and required data of every incremental at or below `through`.
/// Adoption records them as covered with its reason and evidence; it never
/// executes a body, and nothing proves the assertion for the operator.
#[derive(Debug, Clone, Copy)]
pub struct Adoption<'a> {
    /// The boundary: the newest version the target is equivalent to.
    pub through: Version,
    /// Why the target is equivalent. Required.
    pub reason: &'a str,
    /// Supporting evidence, such as a schema comparison or ticket.
    pub evidence: Option<&'a str>,
}

impl<'a> Adoption<'a> {
    /// Asserts equivalence through `through` for `reason`, without evidence.
    pub const fn new(through: Version, reason: &'a str) -> Self {
        Self {
            through,
            reason,
            evidence: None,
        }
    }

    /// Adds supporting evidence.
    pub const fn with_evidence(mut self, evidence: &'a str) -> Self {
        self.evidence = Some(evidence);

        self
    }
}

/// What an adoption recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptReport {
    /// The incrementals adopted, in version order. None of their bodies ran.
    pub adopted: Vec<Version>,
}

/// Why an adoption was refused. Nothing was recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdoptionError {
    /// The assertion has no reason.
    #[error("adoption requires an audit reason")]
    MissingReason,
    /// The catalog has no incremental at or below the boundary.
    #[error("the catalog has no incremental at or below {0} to adopt")]
    NothingCovered(Version),
    /// The target has no user objects: apply creates its schema instead.
    #[error("the target is fresh; apply creates its schema, adoption cannot")]
    Fresh,
    /// The target already has a journal: it cannot be adopted again.
    #[error("the target is already managed; adoption cannot replace its history")]
    Managed,
}
