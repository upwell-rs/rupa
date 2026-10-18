//! Field probes used by `#[derive(Entity)]`, the capability derives and
//! [`col!`]. A derived entity exposes one probe per field; calling
//! `(&&&probe).__rupa_codec()` yields the field's codec.
//!
//! For an inferring [`Probe`], method resolution picks (highest first):
//! 1. `T: ScalarColumn` -> `ScalarCodec<T>`
//! 2. `T = Option<U>`, `U: JsonColumn` -> `NullableJsonCodec<U>`
//! 3. otherwise -> `JsonCodec<T>`, with `T: JsonColumn` required on the
//!    *method* so failures report our diagnostic.
//!
//! Fields with an explicit `#[column(scalar|json)]` get a [`ScalarProbe`],
//! [`JsonProbe`] or [`NullableJsonProbe`] instead. Their inherent
//! `__rupa_codec` is the only candidate, so the override always wins.
//!
//! Resolution happens where the call is written, so it is only meaningful
//! for concrete types; the derive rejects generic entities.

use super::*;

/// A probe for one field of entity `E`.
pub trait FieldProbe: Copy {
    type Entity;
    type Field;
    fn name(&self) -> &'static str;
}

macro_rules! probe {
    ($(#[$m:meta])* $name:ident<$e:ident, $t:ident> => $field:ty) => {
        $(#[$m])*
        pub struct $name<$e, $t> {
            name: &'static str,
            _p: PhantomData<fn() -> ($e, $t)>,
        }
        impl<$e, $t> Clone for $name<$e, $t> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<$e, $t> Copy for $name<$e, $t> {}
        impl<$e, $t> $name<$e, $t> {
            pub const fn new(name: &'static str) -> Self {
                Self { name, _p: PhantomData }
            }
        }
        impl<$e, $t> FieldProbe for $name<$e, $t> {
            type Entity = $e;
            type Field = $field;
            fn name(&self) -> &'static str {
                self.name
            }
        }
    };
}

probe!(
    /// Infers the kind from the field type.
    Probe<E, T> => T
);
probe!(
    /// `#[column(scalar)]`
    ScalarProbe<E, T> => T
);
probe!(
    /// `#[column(json)]` on a non-`Option` field.
    JsonProbe<E, T> => T
);
probe!(
    /// `#[column(json)]` on an `Option<U>` field; `None` is SQL `NULL`.
    NullableJsonProbe<E, U> => Option<U>
);

pub trait ScalarLevel<T> {
    fn __rupa_codec(&self) -> ScalarCodec<T>;
}
impl<E, T: ScalarColumn> ScalarLevel<T> for &&Probe<E, T> {
    fn __rupa_codec(&self) -> ScalarCodec<T> {
        ScalarCodec::new()
    }
}

pub trait NullableJsonLevel<T> {
    fn __rupa_codec(&self) -> NullableJsonCodec<T>;
}
impl<E, T: JsonColumn> NullableJsonLevel<T> for &Probe<E, Option<T>> {
    fn __rupa_codec(&self) -> NullableJsonCodec<T> {
        NullableJsonCodec::new()
    }
}

pub trait JsonLevel<T> {
    fn __rupa_codec(&self) -> JsonCodec<T>
    where
        T: JsonColumn;
}
impl<E, T> JsonLevel<T> for Probe<E, T> {
    fn __rupa_codec(&self) -> JsonCodec<T>
    where
        T: JsonColumn,
    {
        JsonCodec::new()
    }
}

impl<E, T: ScalarColumn> ScalarProbe<E, T> {
    pub fn __rupa_codec(&self) -> ScalarCodec<T> {
        ScalarCodec::new()
    }
}
impl<E, T: JsonColumn> JsonProbe<E, T> {
    pub fn __rupa_codec(&self) -> JsonCodec<T> {
        JsonCodec::new()
    }
}
impl<E, U: JsonColumn> NullableJsonProbe<E, U> {
    pub fn __rupa_codec(&self) -> NullableJsonCodec<U> {
        NullableJsonCodec::new()
    }
}

pub fn column_from_probe<P: FieldProbe, C: Codec<P::Field>>(
    probe: &P,
    _codec: C,
) -> Column<P::Entity, P::Field, C::Kind> {
    Column::with_codec::<C>(probe.name())
}

/// Compile-time check that a companion struct's field of type `T` matches
/// the entity field this probe describes. Implemented per probe type (not
/// via `FieldProbe::Field`) so a mismatch reports this diagnostic.
#[diagnostic::on_unimplemented(
    message = "this field's type `{T}` does not match the entity's field",
    label = "expected the entity field's type",
    note = "the entity field is described by `{Self}`"
)]
pub trait FieldIs<T> {
    /// The entity field type (always `T` when implemented).
    type Field;
    fn cast(value: &T) -> &Self::Field;
}
impl<E, T> FieldIs<T> for Probe<E, T> {
    type Field = T;
    fn cast(value: &T) -> &T {
        value
    }
}
impl<E, T> FieldIs<T> for ScalarProbe<E, T> {
    type Field = T;
    fn cast(value: &T) -> &T {
        value
    }
}
impl<E, T> FieldIs<T> for JsonProbe<E, T> {
    type Field = T;
    fn cast(value: &T) -> &T {
        value
    }
}
impl<E, U> FieldIs<Option<U>> for NullableJsonProbe<E, U> {
    type Field = Option<U>;
    fn cast(value: &Option<U>) -> &Option<U> {
        value
    }
}

pub fn assert_field_type<T, P: FieldIs<T>>(_probe: &P) {}

/// `value` as the entity field type, checked by [`FieldIs`]. When the check
/// fails, only that diagnostic is reported (not a second type mismatch).
pub fn as_field<'a, T, P: FieldIs<T>>(_probe: &P, value: &'a T) -> &'a P::Field {
    P::cast(value)
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }

    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }

        i += 1;
    }

    true
}

/// For an `Insertable` companion: the error message of the first field
/// the entity requires on insert that `provided` lacks. `required` pairs
/// a field with its message. The generated code panics with it in its
/// own `const` item, so the error points at the user's derive rather
/// than into this function (or into `core`'s panic machinery, whose
/// source may not be installed).
pub const fn missing_insert_field(
    required: &'static [(&'static str, &'static str)],
    provided: &[&str],
) -> Option<&'static str> {
    let mut i = 0;
    while i < required.len() {
        let (field, message) = required[i];
        let mut found = false;
        let mut j = 0;
        while j < provided.len() {
            if str_eq(field, provided[j]) {
                found = true;
            }

            j += 1;
        }

        if !found {
            return Some(message);
        }

        i += 1;
    }

    None
}

/// Const string equality, for generated checks.
pub const fn same_str(a: &str, b: &str) -> bool {
    str_eq(a, b)
}
