//! CRUD flags: `#[query(get)]`, `#[query(insert)]` and friends.

use std::collections::BTreeSet;

use super::Crud;
use super::Ctx;
use super::Fields;
use super::QueryAttr;
use super::Shape;
use super::Signature;
use super::check_all_used;
use super::shape_of;
use super::single_generic;
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Ident, Type};

/// `Some(T)` if `ty` is `&T`.
pub(crate) fn deref_type(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(r) => Some(&r.elem),
        _ => None,
    }
}

/// A single value parameter of a CRUD method, as a reference expression.
pub(crate) fn value_ref(ident: &Ident, ty: &Type) -> TokenStream {
    if deref_type(ty).is_some() {
        quote!(#ident)
    } else {
        quote!(&#ident)
    }
}

/// Whether `ty` holds several values: `&[V]`, `Vec<V>`, `&Vec<V>`.
pub(crate) fn is_many(ty: &Type) -> bool {
    let inner = deref_type(ty).unwrap_or(ty);
    matches!(inner, Type::Slice(_)) || single_generic(inner, "Vec").is_some()
}

pub(crate) fn crud_body(
    ms: &TokenStream,
    attr: &QueryAttr,
    sig: &Signature,
) -> syn::Result<TokenStream> {
    let (crud, span) = attr.crud.expect("checked by caller");
    let result = &sig.result;
    let shape = shape_of(result);

    if let (Crud::Delete, Some(filter)) = (crud, &attr.filter) {
        let entity = attr.entity.as_ref().expect("validated");
        if !matches!(shape, Shape::U64 | Shape::Bool) {
            return Err(syn::Error::new(
                sig.result_span,
                "a `delete` returns `u64` (rows deleted) or `bool` (any deleted)",
            ));
        }

        let mut ctx = Ctx {
            ms,
            fields: Fields::Entity(entity),
            params: Some(&sig.params),
            used: BTreeSet::new(),
        };
        let c = ctx.cond(filter)?;
        check_all_used(&ctx.used, sig)?;
        return Ok(
            quote_spanned!(sig.result_span=> #ms::delete::<#entity>().filter(#c).build::<#result>()),
        );
    }

    let [(param, param_ty)] = sig.params.as_slice() else {
        let what = match crud {
            Crud::Get => "the id",
            Crud::Insert => "the value(s) to insert",
            Crud::Update => "the value describing the update",
            Crud::Delete if attr.entity.is_some() => "the id",
            Crud::Delete => "the value to delete (or name `entity` and take the id)",
        };
        return Err(syn::Error::new(
            span,
            format!("this method takes exactly one parameter: {what}"),
        ));
    };
    let value = value_ref(param, param_ty);

    Ok(match crud {
        Crud::Get => {
            let Some(entity) = single_generic(result, "Option") else {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `get` returns `Option<T>`",
                ));
            };
            quote_spanned!(span=> #ms::get::<#entity>(#value))
        }
        Crud::Insert => {
            let rows = if is_many(param_ty) {
                quote_spanned!(span=> #ms::insert::<_>().values(#value))
            } else {
                quote_spanned!(span=> #ms::insert::<_>().value(#value))
            };
            if attr.returning {
                if matches!(shape, Shape::U64 | Shape::Bool) {
                    return Err(syn::Error::new(
                        sig.result_span,
                        "`insert, returning` returns the stored row(s): `T` or `Vec<T>`",
                    ));
                }

                quote_spanned!(sig.result_span=> #rows.returning::<#result>())
            } else {
                if !matches!(shape, Shape::U64 | Shape::Bool) {
                    return Err(syn::Error::new(
                        sig.result_span,
                        "an `insert` returns `u64` or `bool`; add `returning` to get the stored row(s)",
                    ));
                }

                let entity = attr
                    .entity
                    .as_ref()
                    .map(|e| quote!(#e))
                    .unwrap_or(quote!(_));
                let rows = if is_many(param_ty) {
                    quote_spanned!(span=> #ms::insert::<#entity>().values(#value))
                } else {
                    quote_spanned!(span=> #ms::insert::<#entity>().value(#value))
                };
                quote_spanned!(sig.result_span=> #rows.build::<#result>())
            }
        }
        Crud::Update => {
            if !matches!(shape, Shape::U64 | Shape::Bool) {
                return Err(syn::Error::new(
                    sig.result_span,
                    "an `update` returns `u64` (rows updated) or `bool` (any updated)",
                ));
            }

            quote_spanned!(span=> #ms::update::<_>().one(#value).build::<#result>())
        }
        Crud::Delete => {
            if !matches!(shape, Shape::U64 | Shape::Bool) {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `delete` returns `u64` (rows deleted) or `bool` (any deleted)",
                ));
            }

            match &attr.entity {
                Some(entity) => {
                    quote_spanned!(span=> #ms::delete::<#entity>().by_id(#value).build::<#result>())
                }
                None => quote_spanned!(span=> #ms::delete::<_>().one(#value).build::<#result>()),
            }
        }
    })
}
