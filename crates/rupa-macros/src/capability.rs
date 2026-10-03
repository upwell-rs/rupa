//! Capability derives. Each emits exactly one trait impl (plus, on companion
//! structs, compile-time checks against the entity's fields).
//!
//! On an entity (`#[derive(Entity, Insertable)] struct User`), the impl is
//! `Insertable<User> for User`. On a companion struct
//! (`#[insertable(entity = User)] struct NewUser`) it is
//! `Insertable<User> for NewUser`, and every field is checked to exist on
//! `User` with the same type.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{DeriveInput, Path};

use crate::model::{self, Field, column_expr, option_inner};

/// A field's `(column name, encoded value)` pair, for entity `entity`.
fn assignment(
    ms: &TokenStream,
    entity: &TokenStream,
    f: &Field,
    value: TokenStream,
) -> TokenStream {
    let c = column_expr(ms, entity, &f.ident, f.span());
    quote!({ let column = #c; (column.name(), column.encode(#value)) })
}

/// Like [`assignment`], for a companion struct: the value is checked to have
/// the entity field's type (with a targeted diagnostic) before encoding.
fn checked_assignment(
    ms: &TokenStream,
    entity: &TokenStream,
    f: &Field,
    ty: &syn::Type,
    value: TokenStream,
) -> TokenStream {
    let c = column_expr(ms, entity, &f.ident, f.span());
    let fi = &f.ident;
    // `T` is given explicitly so a mismatch fails the `FieldIs` bound
    // (our diagnostic) instead of being inferred from the probe's impl.
    let checked =
        quote_spanned!(f.span()=> #ms::as_field::<#ty, _>(&<#entity>::__rupa_fields().#fi, #value));
    quote!({ let column = #c; (column.name(), column.encode(#checked)) })
}

/// Compile-time check that `f` (of type `ty`) is a field of `entity` with that type.
fn field_check(ms: &TokenStream, entity: &Path, f: &Field, ty: &syn::Type) -> TokenStream {
    let fi = &f.ident;
    quote_spanned!(f.span()=> #ms::assert_field_type::<#ty, _>(&<#entity>::__rupa_fields().#fi);)
}

pub fn gettable(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Gettable")?;
    let ms = model::support_path(&input)?;
    if let Some(p) = model::companion_entity(&input, "gettable")? {
        return Err(syn::Error::new_spanned(
            p,
            "Gettable projections (`entity = ..`) are not supported yet",
        ));
    }
    let ident = &input.ident;
    Ok(quote!(impl #ms::Gettable<#ident> for #ident {}))
}

pub fn deletable(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Deletable")?;
    let ms = model::support_path(&input)?;
    if let Some(p) = model::companion_entity(&input, "deletable")? {
        return Err(syn::Error::new_spanned(
            p,
            "Deletable on companion structs (`entity = ..`) is not supported yet",
        ));
    }
    let ident = &input.ident;
    Ok(quote! {
        impl #ms::Deletable<#ident> for #ident {
            fn delete_key(&self) -> #ms::Vec<#ms::Value> {
                #ms::key_of(self)
            }
        }
    })
}

pub fn insertable(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Insertable")?;
    let ms = model::support_path(&input)?;
    let fields = model::fields(&input, "Insertable")?;
    let ident = &input.ident;

    let Some(entity) = model::companion_entity(&input, "insertable")? else {
        // The entity inserts itself; generated columns are left to the database.
        let me = quote!(#ident);
        let pairs = fields.iter().filter(|f| !f.generated).map(|f| {
            let fi = &f.ident;
            assignment(&ms, &me, f, quote!(&self.#fi))
        });
        return Ok(quote! {
            impl #ms::Insertable<#ident> for #ident {
                fn insert_values(&self) -> #ms::Vec<(&'static str, #ms::Value)> {
                    ::std::vec![#(#pairs),*]
                }
            }
        });
    };

    model::reject_column_attrs(&fields, "Insertable")?;
    if let Some(f) = fields.iter().find(|f| f.id) {
        return Err(syn::Error::new(
            f.ident.span(),
            "`#[id]` has no meaning on an Insertable struct; include the id field like any other, or leave it out if generated",
        ));
    }
    let target = quote!(#entity);
    let pairs = fields.iter().map(|f| {
        let fi = &f.ident;
        checked_assignment(&ms, &target, f, &f.ty, quote!(&self.#fi))
    });
    let names = fields.iter().map(|f| &f.name);

    Ok(quote! {
        impl #ms::Insertable<#entity> for #ident {
            fn insert_values(&self) -> #ms::Vec<(&'static str, #ms::Value)> {
                ::std::vec![#(#pairs),*]
            }
        }

        const _: () = match #ms::missing_insert_field(<#entity>::__RUPA_INSERT_REQUIRED, &[#(#names),*]) {
            ::core::option::Option::Some(message) => ::core::panic!("{}", message),
            ::core::option::Option::None => {}
        };
    })
}

pub fn updatable(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Updatable")?;
    let ms = model::support_path(&input)?;
    let fields = model::fields(&input, "Updatable")?;
    let ident = &input.ident;

    let Some(entity) = model::companion_entity(&input, "updatable")? else {
        // Full update by id: every column except the id and generated ones.
        let id = fields.iter().find(|f| f.id).ok_or_else(|| {
            syn::Error::new_spanned(ident, "Updatable on an entity needs its `#[id]` field")
        })?;
        let id_ident = &id.ident;
        let me = quote!(#ident);
        let pairs = fields.iter().filter(|f| !f.id && !f.generated).map(|f| {
            let fi = &f.ident;
            assignment(&ms, &me, f, quote!(&self.#fi))
        });
        return Ok(quote! {
            impl #ms::Updatable<#ident> for #ident {
                type Key = #ms::Keyed<<#ident as #ms::Entity>::Id>;

                fn key(&self) -> Self::Key {
                    #ms::Keyed(#ms::Clone::clone(&self.#id_ident))
                }

                fn update_values(&self) -> #ms::Vec<(&'static str, #ms::Value)> {
                    ::std::vec![#(#pairs),*]
                }
            }
        });
    };

    model::reject_column_attrs(&fields, "Updatable")?;
    let target = quote!(#entity);
    let mut checks = Vec::new();
    let mut pushes = Vec::new();
    let mut key = None;
    for f in &fields {
        let fi = &f.ident;
        if f.id {
            if key.is_some() {
                return Err(syn::Error::new(
                    f.ident.span(),
                    "composite ids are not supported yet",
                ));
            }
            checks.push(field_check(&ms, &entity, f, &f.ty));
            key = Some(f);
            continue;
        }
        let Some(inner) = option_inner(&f.ty) else {
            return Err(syn::Error::new(
                f.span(),
                "patch fields must be `Option<_>`: `None` leaves the column unchanged, `Some(v)` sets it \
                 (for a nullable column use `Option<Option<_>>`)",
            ));
        };
        let pair = checked_assignment(&ms, &target, f, inner, quote!(value));
        pushes.push(quote! {
            if let #ms::Option::Some(value) = &self.#fi {
                values.push(#pair);
            }
        });
    }

    let (key_ty, key_fn, id_assert) = match key {
        Some(f) => {
            let (fi, name) = (&f.ident, &f.name);
            let message = format!(
                "the `#[id]` field `{name}` of `{ident}` must have the same name as the id field of the entity"
            );
            (
                quote!(#ms::Keyed<<#entity as #ms::Entity>::Id>),
                quote!(#ms::Keyed(#ms::Clone::clone(&self.#fi))),
                quote!(if !#ms::same_str(<#entity>::__RUPA_ID_FIELD, #name) { ::core::panic!(#message) }),
            )
        }
        None => (quote!(#ms::NoKey), quote!(#ms::NoKey), quote!()),
    };

    Ok(quote! {
        impl #ms::Updatable<#entity> for #ident {
            type Key = #key_ty;

            fn key(&self) -> Self::Key {
                #key_fn
            }

            fn update_values(&self) -> #ms::Vec<(&'static str, #ms::Value)> {
                let mut values = #ms::Vec::new();
                #(#pushes)*
                values
            }
        }

        const _: () = {
            #[allow(dead_code)]
            fn __rupa_check_fields() {
                #(#checks)*
            }
            #id_assert
        };
    })
}
