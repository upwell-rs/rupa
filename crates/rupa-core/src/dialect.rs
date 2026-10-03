//! Dialects and their capabilities, in two forms:
//!
//! - **Static:** `D: Supports<caps::Ilike>`. A compile-time bound; the error names
//!   the dialect and the missing capability.
//! - **Runtime:** `dialect.supports(Capability::Ilike)` / `dialect.id()`, for code
//!   whose dialect is chosen at run time (e.g. from config, via [`DynDialect`]).
//!
//! Both forms come from one [`dialect!`] declaration per dialect, so they cannot
//! drift. [`DynDialect`] implements no `Supports<_>` at all: statically gated
//! functions are unusable with it by construction, and the runtime form applies.

use std::fmt::Debug;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialectId {
    Postgres,
    MySql,
    Sqlite,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Ilike,
    JsonPath,
    JsonContainment,
    Returning,
    Savepoints,
    NativeRls,
}

/// A dialect, usable both as a generic parameter and as `&dyn Dialect`.
pub trait Dialect: Debug + Send + Sync + 'static {
    fn id(&self) -> DialectId;
    fn supports(&self, cap: Capability) -> bool;
}

/// Type-level counterpart of a [`Capability`] variant; see [`caps`].
pub trait CapabilityMarker: 'static {
    const CAP: Capability;
}

/// Static capability bound.
#[diagnostic::on_unimplemented(
    message = "dialect `{Self}` does not support `{C}`",
    label = "requires `{C}`",
    note = "use a dialect that supports it, or a runtime-checked function returning `Result`"
)]
pub trait Supports<C: CapabilityMarker>: Dialect {}

/// One zero-sized marker per [`Capability`] variant.
pub mod caps {
    use super::{Capability, CapabilityMarker};

    macro_rules! markers {
        ($($name:ident),*) => {$(
            #[derive(Debug, Clone, Copy)]
            pub struct $name;
            impl CapabilityMarker for $name {
                const CAP: Capability = Capability::$name;
            }
        )*};
    }
    markers!(
        Ilike,
        JsonPath,
        JsonContainment,
        Returning,
        Savepoints,
        NativeRls
    );
}

/// Declares a dialect's capabilities once, emitting the runtime table, the
/// `Dialect` impl and every `Supports<_>` impl from the same list.
macro_rules! dialect {
    ($ty:ident => $id:ident [$($cap:ident),* $(,)?]) => {
        impl $ty {
            pub const CAPABILITIES: &'static [Capability] = &[$(Capability::$cap),*];
        }
        impl Dialect for $ty {
            fn id(&self) -> DialectId {
                DialectId::$id
            }
            fn supports(&self, cap: Capability) -> bool {
                Self::CAPABILITIES.contains(&cap)
            }
        }
        $(impl Supports<caps::$cap> for $ty {})*
    };
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Postgres;

dialect!(Postgres => Postgres [Ilike, JsonPath, JsonContainment, Returning, Savepoints, NativeRls]);

/// The capabilities of a known dialect, as declared by its `dialect!` entry.
pub fn capabilities_of(id: DialectId) -> &'static [Capability] {
    match id {
        DialectId::Postgres => Postgres::CAPABILITIES,
        // Declared with their dialect types in milestone 7.
        DialectId::MySql | DialectId::Sqlite => &[],
    }
}

/// A dialect selected at run time. Implements [`Dialect`] but no [`Supports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynDialect(DialectId);

impl DynDialect {
    pub const fn new(id: DialectId) -> Self {
        Self(id)
    }
}

impl Dialect for DynDialect {
    fn id(&self) -> DialectId {
        self.0
    }
    fn supports(&self, cap: Capability) -> bool {
        capabilities_of(self.0).contains(&cap)
    }
}
