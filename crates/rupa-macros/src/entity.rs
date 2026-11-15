//! `#[derive(Entity)]`: the `Relation`, `Entity` and `FromRow` impls, plus one typed
//! column handle constant per field (`User::EMAIL`), through which `#[query]`,
//! `on!`, `Joined` and the capability derives resolve columns.
//!
//! It grants no insert, update or delete surface; those are separate derives.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{DeriveInput, Ident, LitStr};

use crate::column;
use crate::model::{self, const_ident, option_inner};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Entity")?;
    let ms = model::support_path(&input)?;
    let fields = model::fields(&input, "Entity")?;
    let ident = &input.ident;
    let vis = &input.vis;
    let (table, schema) = table(&input)?;

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
    let (id_ident, id_ty, id_column) = (&id.ident, &id.ty, &id.column);

    column::check_handles(ident, &fields, "#[derive(Entity)]")?;

    let consts = fields.iter().map(|f| column::handle(&ms, ident, vis, f));
    let metas = fields.iter().map(|f| column::meta(&ms, f));
    let reads = fields.iter().enumerate().map(|(i, f)| {
        let (fi, handle) = (&f.ident, const_ident(&f.ident));
        quote!(#fi: Self::#handle.read(row, #i)?)
    });
    let encodes = fields.iter().map(|f| {
        let (fi, handle) = (&f.ident, const_ident(&f.ident));
        quote!(Self::#handle.encode(&self.#fi))
    });

    let required = fields.iter().filter(|f| !f.generated && option_inner(&f.ty).is_none()).map(|f| {
        let name = &f.name;
        let message = format!(
            "this Insertable struct is missing field `{name}` of `{ident}`, which is neither nullable nor generated; \
             add the field, or mark it `#[column(generated)]` on `{ident}` if the database supplies it"
        );
        quote!((#name, #message))
    });

    let relations = fields
        .iter()
        .filter_map(|f| {
            f.references
                .as_ref()
                .map(|target| relation(&ms, &input, f, target))
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #(#relations)*

        impl #ident {
            #(#consts)*

            #[doc(hidden)]
            #vis const __RUPA_INSERT_REQUIRED: &'static [(&'static str, &'static str)] = &[#(#required,)*];
        }

        impl #ms::FromRow for #ident {
            fn from_row(row: &dyn #ms::Row) -> #ms::Result<Self, #ms::ResultError> {
                #ms::Result::Ok(Self { #(#reads,)* })
            }
        }

        impl #ms::Relation for #ident {
            const TABLE: #ms::TableRef = #ms::TableRef { schema: #schema, name: #table };
            const ID_COLUMNS: &'static [&'static str] = &[#id_column];
            const COLUMNS: &'static [#ms::ColumnMeta] = &[#(#metas),*];
        }

        impl #ms::Entity for #ident {
            type Id = #id_ty;

            fn id(&self) -> &#id_ty {
                &self.#id_ident
            }

            fn to_values(&self) -> #ms::Vec<#ms::Value> {
                ::std::vec![#(#encodes),*]
            }
        }
    })
}

/// `#[entity(table = "..", schema = "..")]`: the table name and the schema
/// expression (`Option<&str>`).
fn table(input: &DeriveInput) -> syn::Result<(LitStr, TokenStream)> {
    let ms = model::support_path(input)?;
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
            &input.ident,
            "missing `#[entity(table = \"...\")]`: the table name is never inferred",
        )
    })?;
    let schema = match schema {
        Some(s) => quote!(#ms::Option::Some(#s)),
        None => quote!(#ms::Option::None),
    };

    Ok((table, schema))
}

/// A `#[column(references = Target)]` field: a type check, a marker type, the
/// two `JoinTo` impls (this entity joined onto `Target`, and the reverse), and
/// the relation accessor (`relation = name`, or the field name without `_id`).
fn relation(
    ms: &TokenStream,
    input: &DeriveInput,
    f: &model::Field,
    target: &syn::Path,
) -> syn::Result<TokenStream> {
    let (ident, vis) = (&input.ident, &input.vis);
    let (ty, column) = (&f.ty, &f.column);
    let name = match &f.relation {
        Some(name) => name.clone(),
        None => {
            let base = f
                .name
                .strip_suffix("_id")
                .filter(|b| !b.is_empty())
                .unwrap_or(&f.name);
            // `type_id` would make `fn type()`: keywords can't be accessor names.
            syn::parse_str::<Ident>(base).map_err(|_| {
                syn::Error::new(
                    f.ident.span(),
                    format!(
                        "the relation accessor would be named `{base}`, a keyword; \
                         name it: `#[column(references = .., relation = name)]`"
                    ),
                )
            })?;
            Ident::new(base, f.ident.span())
        }
    };
    let marker = format_ident!("__RupaRel{}_{}", ident, name);
    let doc = format!(
        "The relation through `{}`, for `join::<..>().inner_via(..)`.",
        f.name
    );
    let span = target.span();
    Ok(quote_spanned! {span=>
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #vis struct #marker;

        const _: () = {
            fn check<F: #ms::RefersTo<<#target as #ms::Entity>::Id>>() {}
            let _ = check::<#ty>;
        };

        impl #ms::JoinTo<#target, #ms::Fwd, #marker> for #ident {
            fn on(new: #ms::SourceId, earlier: #ms::SourceId) -> #ms::ExprNode {
                #ms::relation_on(
                    #ms::ColumnRef::of(new, #column),
                    earlier,
                    <#target as #ms::Relation>::ID_COLUMNS,
                )
            }
        }

        impl #ms::JoinTo<#ident, #ms::Rev, #marker> for #target {
            fn on(new: #ms::SourceId, earlier: #ms::SourceId) -> #ms::ExprNode {
                #ms::relation_on(
                    #ms::ColumnRef::of(earlier, #column),
                    new,
                    <#target as #ms::Relation>::ID_COLUMNS,
                )
            }
        }

        impl #ident {
            #[doc = #doc]
            #[allow(dead_code, reason = "a derived accessor exists for every relation, used or not")]
            #vis const fn #name() -> #ms::Rel<#marker> {
                #ms::Rel::new()
            }
        }
    })
}
