//! Derive macros for RUPA. Use them through the `rupa` crate.
//!
//! - `#[derive(Entity)]`: table mapping (`Entity`, `FromRow`). No write surface.
//! - `#[derive(Gettable)]`, `#[derive(Insertable)]`, `#[derive(Updatable)]`,
//!   `#[derive(Deletable)]`: one capability impl each, opted into per entity.
//!
//! - `table!`: a typed table snapshot with no row type (`Relation`).
//!
//! Generated code refers to `::rupa`; override with `#[rupa(crate = path)]`.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod capability;
mod column;
mod dsl;
mod dsl_fn;
mod entity;
mod joined;
mod model;
mod on;
mod query;
mod repository;
mod table;

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

/// A repository trait, and one repository type over any connection.
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
/// let users = UserRepositoryRepo::new(&pool);
/// let tx = users.open_tx(None).await?;   // the same repository, in a transaction
/// tx.by_email("ada@x").await?;
/// tx.commit().await?;
/// ```
///
/// See the `rupa_macros::repository` module docs for the rules on receivers,
/// sync/async methods, errors and the options `static_dispatch` and
/// `database = ..` (an Upwell provider). Async methods
/// return boxed futures by default, so the trait is dyn-compatible.
///
/// Keep the trait so code using it still resolves; only the
/// macro's own error is reported.
#[proc_macro_attribute]
pub fn repository(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item: proc_macro2::TokenStream = item.into();
    repository::repository(attr.into(), item.clone())
        .unwrap_or_else(|e| {
            let mut out = e.into_compile_error();
            out.extend(repository::strip_query_attrs(item));
            out
        })
        .into()
}

/// A struct that joined rows decode into, with fields from several entities.
///
/// ```ignore
/// #[derive(Joined)]
/// struct OrderLine {
///     #[from(Order::id)] order_id: i64,
///     #[from(Coupon::code)] coupon: Option<String>, // left-joined: an Option
///     #[flatten] user: User,                         // a whole entity
/// }
///
/// let lines: Vec<OrderLine> = select::<Order>()
///     .join::<User>().inner()
///     .join::<Coupon>().left()
///     .into::<OrderLine>()
///     .all()
///     .execute(&db)?;
/// ```
///
/// Every field says where it comes from; nothing is matched by name. Only
/// these columns are selected. It is read-only: not an `Entity`.
#[proc_macro_derive(Joined, attributes(from, flatten, rupa))]
pub fn derive_joined(input: TokenStream) -> TokenStream {
    run(input, joined::derive)
}

/// A join condition: `on!(Order::user_id == User::id)`.
///
/// ```ignore
/// select::<User>()
///     .left_join_on::<Order>(on!(Order::user_id == User::id))
///     .filter(on!(Order::total > $min && User::active))
/// ```
///
/// The grammar is `#[query(filter = ..)]`'s, with fields written
/// `Entity::field` (`Entity::field.key` for JSON paths). It expands to a
/// closure over the join's source handles; each entity must be a source
/// exactly once. `$name` binds a variable in scope.
#[proc_macro]
pub fn on(input: TokenStream) -> TokenStream {
    on::on(input.into(), quote::quote!(::rupa::__macro_support))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// A typed table snapshot with no row type: a unit marker implementing
/// `Relation`, and one typed column handle constant per field, built exactly
/// as `#[derive(Entity)]` builds them.
///
/// ```ignore
/// rupa::table!(Accounts = "accounts" {
///     #[id] id: i64,
///     nick: Option<VarChar<64>>,
///     prefs: Prefs,
/// });
///
/// update::<Accounts>().set(Accounts::NICK, &None).filter(Accounts::ID.gt(10))
/// ```
///
/// The grammar is `[#[doc]]* [vis] Marker = ["schema" .] "table" [as Row] { field, .. }`,
/// where a field is `[attribute]* name: Type` and the attributes are the
/// derive's `#[id]`, `#[id(generated)]` and `#[column(name, scalar, json,
/// generated, references)]`, parsed by the same code. More than one `#[id]`
/// declares a composite key. `relation = ..` is refused without `as Row`.
/// Handle constants are the upper-cased field names (`Accounts::NICK`).
///
/// The marker implements `Updatable` and `Deletable` for itself, with nothing
/// to apply and no row key: `update::<Accounts>()` and `delete::<Accounts>()`
/// write through `set` calls and filters, and `#[derive(Insertable)]` input
/// structs can target it. It has no row to read, so selects, `get`,
/// `update(..).one(..)` and `delete(..).one(..)` need an `Entity`. A
/// `references` target must have exactly one id of the field's SQL type.
///
/// With `as Row` the macro instead emits a struct `Row` with the fields (at
/// the table's visibility), the `Entity`, `Gettable`, `Insertable`, `Updatable`
/// and `Deletable` derives, and `type Marker = Row;`: one type, with selects,
/// `get` and keyed writes. Use it to read rows in a data-shifting migration;
/// plain snapshots stay a zero-sized marker.
#[proc_macro]
pub fn table(input: TokenStream) -> TokenStream {
    table::table(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
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
