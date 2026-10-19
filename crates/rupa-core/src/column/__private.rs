//! Compile-time codec selection for `#[derive(Entity)]`, plus checks used by
//! the capability derives.
//!
//! For each field type `T` the derive evaluates, in a constant:
//! 1. `<ScalarSelect<T>>::ENCODING`: `Some` when `T: ScalarColumn`;
//! 2. `<NullableJsonSelect<T>>::ENCODING`: `Some` when `T = Option<U>` with
//!    `U: JsonColumn` (`None` is SQL `NULL`, not JSON `null`);
//! 3. `<JsonSelect<T>>::ENCODING`: `Some` when `T: JsonColumn`;
//!
//! and [`select_encoding`] takes the first `Some`. Each selector has an inherent
//! constant that only exists when its bound holds; otherwise path resolution
//! falls back to the `None` of a blanket trait impl. That only decides for
//! concrete types, which is why the derive rejects generic entities.

use super::*;

/// Picks the scalar encoding of `T`, if it has one.
pub struct ScalarSelect<T>(PhantomData<fn() -> T>);

/// Picks the nullable JSON encoding of `T`, if it has one.
pub struct NullableJsonSelect<T>(PhantomData<fn() -> T>);

/// Picks the JSON encoding of `T`, if it has one.
pub struct JsonSelect<T>(PhantomData<fn() -> T>);

/// The fallback of every selector: no encoding.
pub trait NoEncoding<T> {
    /// The selector's bound does not hold.
    const ENCODING: Option<Encoding<T>> = None;
}

impl<T> NoEncoding<T> for ScalarSelect<T> {}

impl<T> NoEncoding<T> for NullableJsonSelect<T> {}

impl<T> NoEncoding<T> for JsonSelect<T> {}

impl<T: ScalarColumn> ScalarSelect<T> {
    /// `T` is a scalar column.
    pub const ENCODING: Option<Encoding<T>> = Some(Encoding::of::<ScalarCodec<T>>());
}

impl<U: JsonColumn> NullableJsonSelect<Option<U>> {
    /// `Option<U>` of a JSON type.
    pub const ENCODING: Option<Encoding<Option<U>>> = Some(Encoding::of::<NullableJsonCodec<U>>());
}

impl<T: JsonColumn> JsonSelect<T> {
    /// `T` is a JSON document.
    pub const ENCODING: Option<Encoding<T>> = Some(Encoding::of::<JsonCodec<T>>());
}

/// The first available encoding, in selector order, or `None` when the type
/// can be stored neither way.
pub const fn select_encoding<T>(
    scalar: Option<Encoding<T>>,
    nullable_json: Option<Encoding<T>>,
    json: Option<Encoding<T>>,
) -> Option<Encoding<T>> {
    match (scalar, nullable_json, json) {
        (Some(encoding), _, _) | (None, Some(encoding), _) | (None, None, Some(encoding)) => {
            Some(encoding)
        }
        (None, None, None) => None,
    }
}

/// Whether a selected encoding stores JSON; `false` when none was selected,
/// so only the handle's value reports the unstorable type.
pub const fn selects_json<T>(encoding: Option<Encoding<T>>) -> bool {
    match encoding {
        Some(encoding) => encoding.is_json(),
        None => false,
    }
}

/// Whether `field` is the entity's (single) primary key field.
pub const fn is_id_field(columns: &[ColumnMeta], field: &str) -> bool {
    let mut i = 0;
    while i < columns.len() {
        if columns[i].primary_key && str_eq(columns[i].field, field) {
            return true;
        }

        i += 1;
    }

    false
}

/// Compile-time check that a companion struct's field of type `T` matches
/// the entity field `Self` handles. Implemented for [`Column`] so a mismatch
/// reports this diagnostic.
#[diagnostic::on_unimplemented(
    message = "this field's type `{T}` does not match the entity's field",
    label = "expected the entity field's type",
    note = "the entity field is `{Self}`"
)]
pub trait FieldIs<T> {
    /// The entity field type (always `T` when implemented).
    type Field;

    /// `value`, as the entity field type.
    fn cast(value: &T) -> &Self::Field;
}

impl<E, T, K> FieldIs<T> for Column<E, T, K> {
    type Field = T;

    fn cast(value: &T) -> &T {
        value
    }
}

/// Fails to compile unless `column` handles a field of type `T`.
pub fn assert_field_type<T, C: FieldIs<T>>(_column: &C) {}

/// `value` as the entity field type, checked by [`FieldIs`]. When the check
/// fails, only that diagnostic is reported (not a second type mismatch).
pub fn as_field<'a, T, C: FieldIs<T>>(_column: &C, value: &'a T) -> &'a C::Field {
    C::cast(value)
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
