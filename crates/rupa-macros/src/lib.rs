//! Derive macros for RUPA. Use them through the `rupa` crate.
//!
//! - `#[derive(Entity)]`: table mapping (`Entity`, `FromRow`). No write surface.
//! - `#[derive(Gettable)]`, `#[derive(Insertable)]`, `#[derive(Updatable)]`,
//!   `#[derive(Deletable)]`: one capability impl each, opted into per entity.
//!
//! Generated code refers to `::rupa`; override with `#[rupa(crate = path)]`.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod capability;
mod entity;
mod model;

fn run(
    input: TokenStream,
    f: fn(DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    f(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Maps a struct to a table.
///
/// ```ignore
/// #[derive(Entity)]
/// #[entity(table = "users", schema = "app")]
/// struct User {
///     #[id(generated)] id: i64,
///     email: String,
///     #[column(name = "created", generated)] created_at: DateTime<Utc>,
///     prefs: Prefs, // Serialize + DeserializeOwned, not a scalar: a JSON column
/// }
/// ```
///
/// - `#[entity(table = "..", schema = "..")]`: the table is required, never inferred.
/// - `#[id]` / `#[id(generated)]`: exactly one id field.
/// - `#[column(name = "..")]`: column name (defaults to the field name).
/// - `#[column(scalar)]` / `#[column(json)]`: override the inferred column kind.
/// - `#[column(generated)]`: the database supplies the value; skipped by
///   entity-level inserts and updates.
#[proc_macro_derive(Entity, attributes(entity, id, column, rupa))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    run(input, entity::derive)
}

/// `impl Gettable<Self> for Self`: allows `get::<Self>(&id)`.
#[proc_macro_derive(Gettable, attributes(gettable, id, column, rupa))]
pub fn derive_gettable(input: TokenStream) -> TokenStream {
    run(input, capability::gettable)
}

/// On an entity: `impl Insertable<Self> for Self` (generated columns skipped).
/// On a struct with `#[insertable(entity = E)]`: `impl Insertable<E> for Self`,
/// checking that each field exists on `E` with the same type, and that every
/// non-nullable, non-generated field of `E` is present.
#[proc_macro_derive(Insertable, attributes(insertable, id, column, rupa))]
pub fn derive_insertable(input: TokenStream) -> TokenStream {
    run(input, capability::insertable)
}

/// On an entity: `impl Updatable<Self> for Self`, a full update by id.
/// On a patch struct with `#[updatable(entity = E)]`: `impl Updatable<E> for Self`.
/// Patch fields are `Option<_>` (`None` = unchanged); an `#[id]` field makes
/// the patch target one row, without one it applies to a filter.
#[proc_macro_derive(Updatable, attributes(updatable, id, column, rupa))]
pub fn derive_updatable(input: TokenStream) -> TokenStream {
    run(input, capability::updatable)
}

/// `impl Deletable<Self> for Self`: allows deleting rows of this entity.
#[proc_macro_derive(Deletable, attributes(deletable, id, column, rupa))]
pub fn derive_deletable(input: TokenStream) -> TokenStream {
    run(input, capability::deletable)
}
