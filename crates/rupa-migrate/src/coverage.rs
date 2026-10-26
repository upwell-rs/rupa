use crate::{Checksum, Provenance};

/// A durable claim that a target holds the state of incrementals whose
/// bodies never ran on it individually: an executed snapshot, or an
/// operator's adoption. A target has at most one, and it is never undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// The boundary version, as stored: the snapshot's version or the
    /// version adopted through. Nothing at or below it runs or is undone.
    pub boundary: String,
    /// What made the claim.
    pub source: CoverageSource,
    /// The incrementals the claim stands for, in version order, as stored.
    /// A matching checksum shows which source the claim represents, not that
    /// its body ran on the target.
    pub members: Vec<CoverageMember>,
}

/// What made a coverage claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageSource {
    /// A snapshot ran on a fresh target.
    Snapshot {
        /// The snapshot's installation rank.
        installation_rank: u64,
        /// The snapshot's checksum, its coverage manifest included.
        checksum: Checksum,
        /// Whether the runner observed it complete or an operator asserted it.
        provenance: Provenance,
    },
    /// An operator adopted an unmanaged target, asserting it equivalent.
    Adoption {
        /// The journal id of the adoption event.
        id: i64,
    },
}

/// One incremental a coverage claim stands for, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageMember {
    /// The version, as stored.
    pub version: String,
    /// The checksum the claim recorded for it.
    pub checksum: Checksum,
}
