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

/// Per-run choices shared by an apply and its plan. The default is strict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Whether late artifacts may run.
    pub order: Order,
}

impl ApplyOptions {
    /// The options `apply` and `plan` use.
    pub const DEFAULT: Self = Self {
        order: Order::Strict,
    };
}

/// Which installations a revert undoes. Undo always runs newest
/// installation first, and only ever undoes the newest installations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertTarget {
    /// Every installation of a version above this one. They must be the
    /// newest installations: a retained one among them refuses the revert.
    To(Version),
    /// The newest `0` installations, whatever their versions.
    Last(usize),
}
