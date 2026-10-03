//! `#[derive(Entity)]`: the `Entity` and `FromRow` impls, plus hidden field
//! probes that `col!` and the capability derives resolve columns through.
//!
//! It grants no insert, update or delete surface; those are separate derives.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{DeriveInput, LitStr};

use crate::model::{self, Kind, column_expr, fields_struct_ident, option_inner};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Entity")?;
    let ms = model::support_path(&input)?;
    let fields = model::fields(&input, "Entity")?;
    let ident = &input.ident;
    let vis = &input.vis;
    let entity = quote!(#ident);

    let (mut table, mut schema): (Option<LitStr>, Option<LitStr>) = (None, None);
    for attr in &input.attrs {
        if attr.path().is_ident("entity") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("table") {
                    table = Some(m.value()?.parse()?);
                } else if m.path.is_ident("schema") {
                    schema = Some(m.value()?.parse()?);
                } else {
                    return Err(m.error(
                        "unknown `entity` option; expected `table = \"..\"` or `schema = \"..\"`",
                    ));
                }
                Ok(())
            })?;
        }
    }
    let table = table.ok_or_else(|| {
        syn::Error::new_spanned(
            ident,
            "missing `#[entity(table = \"...\")]`: the table name is never inferred",
        )
    })?;
    let schema = match schema {
        Some(s) => quote!(#ms::Option::Some(#s)),
        None => quote!(#ms::Option::None),
    };

    let ids: Vec<_> = fields.iter().filter(|f| f.id).collect();
    let id = match ids.as_slice() {
        [id] => *id,
        [] => {
            return Err(syn::Error::new_spanned(
                ident,
                "an entity needs exactly one `#[id]` field",
            ));
        }
        [_, second, ..] => {
            return Err(syn::Error::new(
                second.ident.span(),
                "composite ids (several `#[id]` fields) are not supported yet",
            ));
        }
    };
    let (id_ident, id_ty, id_column, id_name) = (&id.ident, &id.ty, &id.column, &id.name);

    // Hidden field probes: one per field, typed by how its kind is decided.
    let fields_ty = fields_struct_ident(ident);
    let mut probe_fields = Vec::new();
    let mut probe_inits = Vec::new();
    for f in &fields {
        let (fi, ty, col) = (&f.ident, &f.ty, &f.column);
        let probe_ty = match (f.kind, option_inner(ty)) {
            (Kind::Infer, _) => quote!(#ms::Probe<#ident, #ty>),
            (Kind::Scalar, _) => quote!(#ms::ScalarProbe<#ident, #ty>),
            (Kind::Json, Some(inner)) => quote!(#ms::NullableJsonProbe<#ident, #inner>),
            (Kind::Json, None) => quote!(#ms::JsonProbe<#ident, #ty>),
        };
        probe_fields.push(quote!(pub #fi: #probe_ty));
        probe_inits.push(quote!(#fi: <#probe_ty>::new(#col)));
    }

    let column = |f: &model::Field| column_expr(&ms, &entity, &f.ident, f.span());

    let metas = fields.iter().map(|f| {
        let (c, name) = (column(f), &f.name);
        let meta = quote_spanned!(f.span()=> #c.meta(#name));
        if f.generated {
            quote!(#meta.generated())
        } else {
            meta
        }
    });
    let reads = fields.iter().enumerate().map(|(i, f)| {
        let (fi, c) = (&f.ident, column(f));
        quote!(#fi: #c.read(row, #i)?)
    });
    let encodes = fields.iter().map(|f| {
        let (fi, c) = (&f.ident, column(f));
        quote!(#c.encode(&self.#fi))
    });

    // Fields an `Insertable` companion must provide: neither nullable nor generated.
    let required = fields.iter().filter(|f| !f.generated && option_inner(&f.ty).is_none()).map(|f| {
        let name = &f.name;
        let message = format!(
            "this Insertable struct is missing field `{name}` of `{ident}`, which is neither nullable nor generated; \
             add the field, or mark it `#[column(generated)]` on `{ident}` if the database supplies it"
        );
        quote!((#name, #message))
    });

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types, non_snake_case)]
        #vis struct #fields_ty {
            #(#probe_fields,)*
        }

        impl #ident {
            #[doc(hidden)]
            #vis const fn __rupa_fields() -> #fields_ty {
                #fields_ty { #(#probe_inits,)* }
            }

            #[doc(hidden)]
            #vis const __RUPA_INSERT_REQUIRED: &'static [(&'static str, &'static str)] = &[#(#required,)*];

            #[doc(hidden)]
            #vis const __RUPA_ID_FIELD: &'static str = #id_name;
        }

        impl #ms::FromRow for #ident {
            fn from_row(row: &dyn #ms::Row) -> #ms::Result<Self, #ms::ResultError> {
                #ms::Result::Ok(Self { #(#reads,)* })
            }
        }

        impl #ms::Entity for #ident {
            type Id = #id_ty;
            const TABLE: #ms::TableRef = #ms::TableRef { schema: #schema, name: #table };
            const ID_COLUMNS: &'static [&'static str] = &[#id_column];

            fn columns() -> &'static [#ms::ColumnMeta] {
                static COLUMNS: #ms::OnceLock<#ms::Vec<#ms::ColumnMeta>> = #ms::OnceLock::new();
                COLUMNS.get_or_init(|| ::std::vec![#(#metas),*])
            }

            fn id(&self) -> &#id_ty {
                &self.#id_ident
            }

            fn to_values(&self) -> #ms::Vec<#ms::Value> {
                ::std::vec![#(#encodes),*]
            }
        }
    })
}
