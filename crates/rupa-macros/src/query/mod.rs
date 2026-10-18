//! `#[query(..)]`: turns a declared signature into a sans-IO `Query<R>` built
//! with the typed builder. Nothing is inferred from names: the attribute
//! gives the filter, ordering and paging; the signature gives the parameter
//! types and the result type `R`.
//!
//! ```text
//! #[query(filter = <expr>, order_by = field [asc|desc], .., limit = n|$p, offset = n|$p)]
//! #[query(sql = "SELECT .. WHERE email = $email")]
//! #[query(entity = Type, filter = ..)]        // when R does not name the entity (bool)
//! #[query(left_join = Order on Order::user_id == User::id, filter = User::active)]
//! #[query(inner_join = Order, inner_join = Line via Line::order)]  // declared relations
//! ```
//!
//! Joins (`inner_join` / `left_join`, repeatable, in order) take a condition
//! (`on <expr>`), a relation (`via Entity::relation`), or neither: then the one
//! declared relation to an earlier source. With joins, every field is written
//! `Entity::field`, and the result is a tuple (`Vec<(User, Option<Order>)>`,
//! whose first element is the first source) or a `#[derive(Joined)]` struct
//! (which needs `entity = First`).
//!
//! Result types: `Vec<T>`, `Option<T>`, `T` (rows of entity `T`), `bool`
//! (whether any row matches; needs `entity`). Raw SQL also takes `u64` and
//! `bool` as affected-row results.

mod attr;
mod crud;
mod ctx;
mod join;
mod raw;
mod signature;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};

use crate::dsl::Expr;

pub(crate) use attr::*;
pub(crate) use crud::*;
pub(crate) use ctx::*;
pub(crate) use join::*;
pub(crate) use raw::*;
pub(crate) use signature::*;

pub(crate) fn count(ctx: &mut Ctx, c: &Count) -> syn::Result<TokenStream> {
    match c {
        Count::Param(p) => ctx.param(p),
        Count::Lit(l) => Ok(quote!(#l)),
    }
}

/// The expression building `Query<R>` for a declared query.
pub fn query_body(ms: &TokenStream, attr: &QueryAttr, sig: &Signature) -> syn::Result<TokenStream> {
    if attr.crud.is_some() {
        return crud_body(ms, attr, sig);
    }

    let result = &sig.result;
    let shape = shape_of(result);

    let body = if let Some(sql) = &attr.sql {
        raw_body(ms, sql, sig, &shape)?
    } else {
        let entity = match (&attr.entity, &shape) {
            (Some(e), _) => e.clone(),
            (None, Shape::Rows { entity }) => (**entity).clone(),
            (None, Shape::Bool) => {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `bool` query checks whether rows exist; name the entity with `entity = Type`",
                ));
            }
            (None, Shape::U64) => {
                return Err(syn::Error::new(
                    sig.result_span,
                    "`u64` (affected rows) is for raw `sql` statements; filter queries return rows or `bool`",
                ));
            }
        };
        if !attr.joins.is_empty() {
            return join_body(ms, attr, sig, &shape, result);
        }

        let mut ctx = Ctx {
            ms,
            fields: Fields::Entity(&entity),
            params: Some(&sig.params),
            used: BTreeSet::new(),
        };
        let mut b = quote!(#ms::select::<#entity>());
        if let Some(f) = &attr.filter {
            let c = ctx.cond(f)?;
            b = quote!(#b.filter(#c));
        }

        for key in &attr.order_by {
            let column = match &key.entity {
                None => ctx.column(&key.field)?,
                Some(entity) => ctx.qualified(entity, &key.field)?,
            };
            let dir = if key.desc { quote!(desc) } else { quote!(asc) };
            b = quote!(#b.order_by({
                #[allow(unused_imports)]
                use #ms::ExprOps as _;
                #column.#dir()
            }));
        }

        if let Some(l) = &attr.limit {
            let l = count(&mut ctx, l)?;
            b = quote!(#b.limit(#l));
        }

        if let Some(o) = &attr.offset {
            let o = count(&mut ctx, o)?;
            b = quote!(#b.offset(#o));
        }

        check_all_used(&ctx.used, sig)?;
        match shape {
            Shape::Bool => quote!(#b.exists()),
            _ => quote_spanned!(sig.result_span=> #b.build::<#result>()),
        }
    };
    Ok(body)
}

pub(crate) fn check_all_used(used: &BTreeSet<String>, sig: &Signature) -> syn::Result<()> {
    match sig
        .params
        .iter()
        .find(|(name, _)| !used.contains(&name.to_string()))
    {
        Some((name, _)) => Err(syn::Error::new(
            name.span(),
            format!("parameter `{name}` is not used by the query; reference it as `${name}`"),
        )),
        None => Ok(()),
    }
}

/// `#[query]` on a free function: `fn f(args) -> R;` becomes
/// `fn f(args) -> Query<R> { .. }`.
///
/// Parameters are cloned so each may be used more than once.
pub fn query_fn(attr: TokenStream, item: TokenStream, ms: TokenStream) -> syn::Result<TokenStream> {
    let attr: QueryAttr = syn::parse2(attr)?;
    let item: syn::ForeignItemFn = syn::parse2(item).map_err(|e| {
        syn::Error::new(
            e.span(),
            "`#[query]` applies to a function declaration without a body: `fn name(args) -> R;`",
        )
    })?;
    let sig_info = Signature::from_sig(&item.sig)?;
    let body = query_body(&ms, &attr, &sig_info)?;
    let syn::ForeignItemFn {
        attrs,
        vis,
        mut sig,
        ..
    } = item;
    let result = &sig_info.result;
    sig.output = syn::parse_quote!(-> #ms::Query<#result>);
    Ok(quote! {
        #(#attrs)*
        #[allow(clippy::clone_on_copy)]
        #vis #sig {
            #body
        }
    })
}

/// The DSL function paths called in `attr`, for availability bounds.
pub fn called_functions(attr: &QueryAttr) -> Vec<syn::Path> {
    fn walk(e: &Expr, out: &mut Vec<syn::Path>) {
        match e {
            Expr::Or(a, b) | Expr::And(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Cmp(_, a, b, _) | Expr::In(a, b, _) | Expr::Like(a, b, _) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Not(a, _) | Expr::IsNull(a, _) => walk(a, out),
            Expr::Call(p, args) => {
                if !out
                    .iter()
                    .any(|q| quote!(#q).to_string() == quote!(#p).to_string())
                {
                    out.push(p.clone());
                }

                for a in args {
                    walk(a, out);
                }
            }
            Expr::Field(_) | Expr::Qualified(..) | Expr::Param(_) | Expr::Lit(_) => {}
        }
    }
    let mut out = Vec::new();
    for join in &attr.joins {
        if let JoinOn::Expr(e) = &join.on {
            walk(e, &mut out);
        }
    }

    if let Some(f) = &attr.filter {
        walk(f, &mut out);
    }

    out
}
