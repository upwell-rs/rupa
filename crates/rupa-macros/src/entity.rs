//! `#[derive(Entity)]`: the `Entity` and `FromRow` impls, plus one typed
//! column handle constant per field (`User::EMAIL`), through which `#[query]`,
//! `on!`, `Joined` and the capability derives resolve columns.
//!
//! It grants no insert, update or delete surface; those are separate derives.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{DeriveInput, Ident, LitStr};

use crate::model::{self, Kind, const_ident, known_scalar, option_inner};

/// Prefix of the hidden associated constants the derive emits next to the
/// column handles.
const RESERVED: &str = "__RUPA_";

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

    let mut handles = HashMap::new();
    for f in &fields {
        let handle = const_ident(&f.ident).to_string();

        if handle.starts_with(RESERVED) {
            return Err(syn::Error::new(
                f.ident.span(),
                format!(
                    "field `{}` would get the column handle `{ident}::{handle}`, which `#[derive(Entity)]` reserves; rename it",
                    f.ident
                ),
            ));
        }

        if let Some(earlier) = handles.insert(handle.clone(), &f.ident) {
            return Err(syn::Error::new(
                f.ident.span(),
                format!(
                    "fields `{earlier}` and `{}` would both get the column handle `{ident}::{handle}`; rename one",
                    f.ident
                ),
            ));
        }
    }

    let consts = fields.iter().map(|f| handle(&ms, ident, vis, f));
    let metas = fields.iter().map(|f| {
        let (handle, name) = (const_ident(&f.ident), &f.name);
        let mut meta = quote_spanned!(f.span()=> Self::#handle.meta(#name));

        if f.id {
            meta = quote!(#meta.primary_key());
        }

        if f.generated {
            let generated = if f.identity {
                quote!(#ms::Generated::Identity)
            } else {
                quote!(#ms::Generated::Database)
            };
            meta = quote!(#meta.generated(#generated));
        }

        if let Some(target) = &f.references {
            meta = quote_spanned! {target.span()=>
                #meta.references(#ms::Reference {
                    table: <#target as #ms::Entity>::TABLE,
                    columns: <#target as #ms::Entity>::ID_COLUMNS,
                })
            };
        }

        meta
    });
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

        impl #ms::Entity for #ident {
            type Id = #id_ty;
            const TABLE: #ms::TableRef = #ms::TableRef { schema: #schema, name: #table };
            const ID_COLUMNS: &'static [&'static str] = &[#id_column];
            const COLUMNS: &'static [#ms::ColumnMeta] = &[#(#metas),*];

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

/// The typed column handle constant of field `f`, with the kind its type
/// selects at compile time, or the one `#[column(scalar|json)]` forces.
fn handle(
    ms: &TokenStream,
    entity: &Ident,
    vis: &syn::Visibility,
    f: &model::Field,
) -> TokenStream {
    let (ty, column, name) = (&f.ty, &f.column, &f.name);
    let handle = const_ident(&f.ident);
    let doc = format!("The `{name}` column.");
    let scalar = (
        quote!(#ms::Scalar),
        quote!(#ms::Encoding::of::<#ms::ScalarCodec<#ty>>()),
    );
    let mut selection = TokenStream::new();
    let (kind, encoding) = match (f.kind, option_inner(ty)) {
        (Kind::Scalar, _) => scalar,
        (Kind::Infer, _) if known_scalar(ty) => scalar,
        (Kind::Json, Some(inner)) => (
            quote!(#ms::Json),
            quote!(#ms::Encoding::of::<#ms::NullableJsonCodec<#inner>>()),
        ),
        (Kind::Json, None) => (
            quote!(#ms::Json),
            quote!(#ms::Encoding::of::<#ms::JsonCodec<#ty>>()),
        ),
        (Kind::Infer, _) => {
            let selected = format_ident!("{RESERVED}{handle}_ENCODING", span = f.ident.span());
            let message = format!(
                "field `{name}` of `{entity}` can't be stored in a column: its type must implement \
                 `ScalarColumn` (a scalar column) or `Serialize + DeserializeOwned` (a JSON column)"
            );

            selection = quote_spanned! {ty.span()=>
                #[doc(hidden)]
                #[allow(dead_code, reason = "read by the handle's type and value")]
                #vis const #selected: #ms::Option<#ms::Encoding<#ty>> = {
                    #[allow(unused_imports)]
                    use #ms::NoEncoding as _;

                    #ms::select_encoding(
                        <#ms::ScalarSelect<#ty>>::ENCODING,
                        <#ms::NullableJsonSelect<#ty>>::ENCODING,
                        <#ms::JsonSelect<#ty>>::ENCODING,
                    )
                };
            };

            (
                quote_spanned!(ty.span()=> <#ms::Stored<{ #ms::is_json_encoding(#entity::#selected) }> as #ms::Kinded>::Kind),
                quote_spanned! {ty.span()=>
                    match #entity::#selected {
                        #ms::Option::Some(encoding) => encoding,
                        #ms::Option::None => ::core::panic!(#message),
                    }
                },
            )
        }
    };

    quote_spanned! {f.ident.span()=>
        #selection

        #[doc = #doc]
        #[allow(dead_code, reason = "a derived handle exists for every field, used or not")]
        #vis const #handle: #ms::Column<#entity, #ty, #kind> = #ms::Column::with_encoding(#column, #encoding);
    }
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
                    <#target as #ms::Entity>::ID_COLUMNS,
                )
            }
        }

        impl #ms::JoinTo<#ident, #ms::Rev, #marker> for #target {
            fn on(new: #ms::SourceId, earlier: #ms::SourceId) -> #ms::ExprNode {
                #ms::relation_on(
                    #ms::ColumnRef::of(earlier, #column),
                    new,
                    <#target as #ms::Entity>::ID_COLUMNS,
                )
            }
        }

        impl #ident {
            #[doc = #doc]
            #vis const fn #name() -> #ms::Rel<#marker> {
                #ms::Rel::new()
            }
        }
    })
}
