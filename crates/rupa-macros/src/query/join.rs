//! Joined queries: the join builder and condition closures.

use std::collections::BTreeSet;

use super::Ctx;
use super::Fields;
use super::JoinKind;
use super::JoinOn;
use super::KnownSource;
use super::QueryAttr;
use super::Shape;
use super::Signature;
use super::check_all_used;
use super::count;
use super::path_key;
use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Ident, Type};

/// `|sources| -> Expr<bool> { cond }`: a condition over a join's sources, as
/// `on!` and joined `#[query]`s write it.
///
/// `$name` clones its value, which may be `Copy`.
pub fn condition_closure(ms: &TokenStream, sources: &Ident, cond: &TokenStream) -> TokenStream {
    quote! {
        |#sources| -> #ms::Expr<bool> {
            let _ = &#sources;
            #[allow(clippy::clone_on_copy)]
            let condition = #cond;
            condition
        }
    }
}

/// The body of a `#[query]` with joins: the join builder, with every
/// condition and sort key a closure over the sources.
pub(crate) fn join_body(
    ms: &TokenStream,
    attr: &QueryAttr,
    sig: &Signature,
    shape: &Shape,
    result: &Type,
) -> syn::Result<TokenStream> {
    let row = match shape {
        Shape::Rows { entity } => Some(&**entity),
        Shape::Bool => None,
        Shape::U64 => {
            return Err(syn::Error::new(
                sig.result_span,
                "`u64` (affected rows) is for raw `sql` statements; joins return rows or `bool`",
            ));
        }
    };
    let tuple = match row {
        Some(Type::Tuple(t)) => Some(t),
        _ => None,
    };
    let base = match (&attr.entity, tuple) {
        (Some(e), _) => quote!(#e),
        (None, Some(t)) if !t.elems.is_empty() => {
            let first = &t.elems[0];
            quote!(#first)
        }
        _ => {
            return Err(syn::Error::new(
                sig.result_span,
                "name the first source with `entity = Type` (the rows aren't a tuple starting with it)",
            ));
        }
    };

    let sources = Ident::new("__rupa_sources", Span::mixed_site());
    let mut known = vec![KnownSource {
        key: path_key(&base),
        entity: base.clone(),
        alias: attr.entity_alias.clone(),
    }];
    for join in &attr.joins {
        let entity = &join.entity;
        known.push(KnownSource {
            entity: quote!(#entity),
            key: path_key(&quote!(#entity)),
            alias: join.alias.clone(),
        });
    }

    let mut aliases = BTreeSet::new();
    for alias in known.iter().filter_map(|s| s.alias.as_ref()) {
        if !aliases.insert(alias.to_string()) || known.iter().any(|s| *alias == s.key) {
            return Err(syn::Error::new(
                alias.span(),
                format!("alias `{alias}` is used twice, or is also an entity's name"),
            ));
        }
    }

    let mut ctx = Ctx {
        ms,
        fields: Fields::Known(sources.clone(), known),
        params: Some(&sig.params),
        used: BTreeSet::new(),
    };
    let mut b = quote!(#ms::select::<#base>());
    for join in &attr.joins {
        let entity = &join.entity;
        let (on_method, finish, via) = match join.kind {
            JoinKind::Inner => (quote!(inner_join_on), quote!(inner), quote!(inner_via)),
            JoinKind::Left => (quote!(left_join_on), quote!(left), quote!(left_via)),
        };
        b = match &join.on {
            JoinOn::Expr(e) => {
                let closure = condition_closure(ms, &sources, &ctx.cond(e)?);
                quote!(#b.#on_method::<#entity>(#closure))
            }
            JoinOn::Via(rel) => quote!(#b.join::<#entity>().#via(#rel())),
            JoinOn::Inferred => quote!(#b.join::<#entity>().#finish()),
        };
    }

    if let Some(f) = &attr.filter {
        let closure = condition_closure(ms, &sources, &ctx.cond(f)?);
        b = quote!(#b.filter(#closure));
    }

    for key in &attr.order_by {
        let Some(entity) = &key.entity else {
            return Err(syn::Error::new(
                key.field.span(),
                format!("name the field's entity: `Entity::{}`", key.field),
            ));
        };
        let column = ctx.qualified(entity, &key.field)?;
        let dir = if key.desc { quote!(desc) } else { quote!(asc) };
        b = quote!(#b.order_by(|#sources| {
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
    Ok(match (shape, tuple, row) {
        (Shape::Bool, ..) => quote!(#b.exists()),
        (_, Some(_), _) => quote_spanned!(sig.result_span=> #b.build::<#result>()),
        (_, None, Some(row)) => {
            quote_spanned!(sig.result_span=> #b.into::<#row>().build::<#result, _>())
        }
        (_, None, None) => unreachable!("rows have a row type"),
    })
}
