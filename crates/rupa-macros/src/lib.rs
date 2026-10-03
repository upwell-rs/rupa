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
mod dsl;
mod dsl_fn;
mod entity;
mod model;
mod query;
mod repository;

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

/// A sans-IO query from a declared signature: `fn f(args) -> R;` becomes
/// `fn f(args) -> Query<R>`.
///
/// ```ignore
/// #[query(filter = email == $email)]
/// fn by_email(email: &str) -> Option<User>;
///
/// #[query(filter = active == $active && prefs.theme == $theme,
///         order_by = created_at desc, limit = 50)]
/// fn active_by_theme(active: bool, theme: &str) -> Vec<User>;
///
/// #[query(sql = "SELECT .. FROM users WHERE email = $email")]
/// fn raw_by_email(email: &str) -> Vec<User>;
/// ```
///
/// Generated code refers to `::rupa` (the facade crate).
#[proc_macro_attribute]
pub fn query(attr: TokenStream, item: TokenStream) -> TokenStream {
    query::query_fn(
        attr.into(),
        item.into(),
        quote::quote!(::rupa::__macro_support),
    )
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

/// A repository trait plus its implementation for `Repo<S>`.
///
/// ```ignore
/// #[repository]
/// pub trait UserRepository: Send + Sync {
///     #[query(filter = email == $email)]
///     async fn by_email(&self, email: &str) -> Result<Option<User>, DynError>;
///
///     #[query(filter = active == $active, order_by = id)]
///     fn active(&self, active: bool) -> Result<Vec<User>, DynError>;  // blocks in place
/// }
///
/// let users: Arc<dyn UserRepository> = Arc::new(Repo::shared_async(executor));
/// ```
///
/// See the `rupa_macros::repository` module docs for the rules on receivers,
/// sync/async methods, errors and `static_dispatch`.
#[proc_macro_attribute]
pub fn repository(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item: proc_macro2::TokenStream = item.into();
    repository::repository(attr.into(), item.clone())
        .unwrap_or_else(|e| {
            // Keep the trait so code using it still resolves; only the
            // macro's own error is reported.
            let mut out = e.into_compile_error();
            out.extend(repository::strip_query_attrs(item));
            out
        })
        .into()
}

/// A dialect-aware DSL function; use as `#[rupa::dsl::function]`.
/// See the `dsl_fn` module docs for the generated items.
#[proc_macro_attribute]
pub fn function(attr: TokenStream, item: TokenStream) -> TokenStream {
    dsl_fn::function(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
