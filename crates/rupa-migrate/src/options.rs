use crate::{Blocker, Version};

/// Whether an apply may run late artifacts: pending versions below the
/// newest installed one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Order {
    /// Refuse late artifacts before anything runs.
    #[default]
    Strict,
    /// Run late artifacts after the installed ones, each with a new
    /// installation rank, and report them as late. The author answers for
    /// both orders: the engine cannot prove that the migrations commute.
    /// Every other blocker still refuses the run.
    AllowLate,
}

impl Order {
    /// Removes the late blockers this order permits from `blockers` and
    /// returns their versions, in catalog order.
    pub(crate) fn permit(self, blockers: &mut Vec<Blocker>) -> Vec<Version> {
        let mut late = Vec::new();

        if self == Self::AllowLate {
            blockers.retain(|blocker| match blocker {
                Blocker::Late { version, .. } => {
                    late.push(*version);

                    false
                }
                _ => true,
            });
        }

        late
    }
}

/// How an apply initializes a target that records no installation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Init {
    /// Run the newest snapshot at or below the apply's target version, which
    /// covers the incrementals up to its version, then the incrementals
    /// above it. Without such a snapshot, replay the incrementals.
    #[default]
    PreferSnapshot,
    /// Replay every incremental, never running a snapshot.
    FullReplay,
}

/// Per-run choices shared by an apply and its plan. The default is strict,
/// applies every pending version and prefers a snapshot on a fresh target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Whether late artifacts may run.
    pub order: Order,
    /// The newest version to apply; `None` applies the whole catalog.
    /// Late versions above it are left pending rather than refused.
    pub through: Option<Version>,
    /// How a target that records no installation is initialized.
    pub init: Init,
}

impl ApplyOptions {
    /// The options `apply` and `plan` use.
    pub const DEFAULT: Self = Self {
        order: Order::Strict,
        through: None,
        init: Init::PreferSnapshot,
    };
}

/// Which installations a revert undoes. Undo always runs newest
/// installation first, and only ever undoes the newest installations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertTarget {
    /// Every installation of a version above this one. They must be the
    /// newest installations: a retained one among them refuses the revert.
    To(Version),
    /// The newest installations, this many, whatever their versions.
    Last(usize),
}
