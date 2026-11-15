//! `table!`: a typed table snapshot with no row type. It emits a unit marker
//! implementing `Relation`, plus the same typed column handle constants and
//! `ColumnMeta` values `#[derive(Entity)]` emits for an equivalent struct.
//!
//! ```ignore
//! rupa::table!(Accounts = "main"."accounts" {
//!     #[id(generated)] id: i64,
//!     nick: Option<VarChar<64>>,
//!     #[column(references = Owner)] owner_id: i64,
//!     prefs: Json<Prefs>,
//! });
//! ```
//!
//! The marker may write through `update` and `delete` with explicit `set`
//! calls and filters; it has no row to read, insert or look up by id.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Ident, LitStr, Path, Token, Type, Visibility, braced};

use crate::column;
use crate::model;

const SUPPORTED: &str =
    "`#[id]`, `#[id(generated)]` and `#[column(name, scalar, json, generated, references)]`";

/// A parsed `table!` invocation.
struct Table {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    schema: Option<LitStr>,
    name: LitStr,
    fields: Vec<model::Field>,
    krate: Option<Path>,
}

impl Parse for Table {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let all = input.call(Attribute::parse_outer)?;
        let (attrs, krate) = marker_attrs(all)?;
        let vis = input.parse()?;
        let ident = input.parse()?;

        input.parse::<Token![=]>()?;

        let first: LitStr = input.parse()?;
        let (schema, name) = if input.peek(Token![.]) {
            input.parse::<Token![.]>()?;

            (Some(first), input.parse()?)
        } else {
            (None, first)
        };
        let body;

        braced!(body in input);

        let fields = Punctuated::<FieldDecl, Token![,]>::parse_terminated(&body)?
            .into_iter()
            .map(|field| field.0)
            .collect();

        Ok(Self {
            attrs,
            vis,
            ident,
            schema,
            name,
            fields,
            krate,
        })
    }
}

/// One declared column, parsed with the derive's attribute grammar.
struct FieldDecl(model::Field);

impl Parse for FieldDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;

        input.parse::<Token![:]>()?;

        let ty: Type = input.parse()?;
        let mut field = model::Field::new(ident, ty);

        for attr in &attrs {
            if !["id", "column", "doc"]
                .iter()
                .any(|name| attr.path().is_ident(name))
            {
                return Err(syn::Error::new_spanned(
                    attr,
                    format!(
                        "unsupported attribute in `table!`; the supported ones are {SUPPORTED}"
                    ),
                ));
            }
        }

        field.apply_attrs(&attrs)?;

        if let Some(relation) = &field.relation {
            return Err(syn::Error::new(
                relation.span(),
                "`table!` declares no relation accessors; drop `relation = ..`",
            ));
        }

        Ok(Self(field))
    }
}

fn marker_attrs(all: Vec<Attribute>) -> syn::Result<(Vec<Attribute>, Option<Path>)> {
    let mut krate = None;
    let mut kept = Vec::new();

    for attr in all {
        if attr.path().is_ident("doc") {
            kept.push(attr);
        } else if attr.path().is_ident("rupa") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("crate") {
                    krate = Some(m.value()?.parse()?);

                    Ok(())
                } else {
                    Err(m.error("unknown `rupa` attribute; expected `crate = path`"))
                }
            })?;
        } else {
            return Err(syn::Error::new_spanned(
                &attr,
                "`table!` takes doc comments and `#[rupa(crate = path)]` on the table",
            ));
        }
    }

    Ok((kept, krate))
}

/// Expands a `table!` invocation.
pub fn table(input: TokenStream) -> syn::Result<TokenStream> {
    let table: Table = syn::parse2(input)?;
    let Table {
        attrs,
        vis,
        ident,
        schema,
        name,
        fields,
        krate,
    } = table;
    let ms = match krate {
        Some(path) => quote!(#path::__macro_support),
        None => quote!(::rupa::__macro_support),
    };
    let schema = match schema {
        Some(schema) => quote!(#ms::Option::Some(#schema)),
        None => quote!(#ms::Option::None),
    };
    let ids: Vec<_> = fields.iter().filter(|f| f.id).map(|f| &f.column).collect();

    column::check_handles(&ident, &fields, "table!")?;

    let consts = fields.iter().map(|f| column::handle(&ms, &ident, &vis, f));
    let metas = fields.iter().map(|f| column::meta(&ms, f));
    let references = fields
        .iter()
        .filter_map(|f| f.references.as_ref())
        .map(|target| references_check(&ms, target));

    Ok(quote! {
        #(#attrs)*
        #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy)]
        #vis struct #ident;

        impl #ident {
            #(#consts)*
        }

        impl #ms::Relation for #ident {
            const TABLE: #ms::TableRef = #ms::TableRef { schema: #schema, name: #name };
            const ID_COLUMNS: &'static [&'static str] = &[#(#ids),*];
            const COLUMNS: &'static [#ms::ColumnMeta] = &[#(#metas),*];
        }

        impl #ms::Updatable<#ident> for #ident {
            type Key = #ms::NoKey;

            fn key(&self) -> #ms::NoKey {
                #ms::NoKey
            }

            fn update_values(&self) -> #ms::Vec<(&'static str, #ms::Value)> {
                #ms::Vec::new()
            }
        }

        impl #ms::Deletable<#ident> for #ident {
            fn delete_key(&self) -> #ms::Vec<#ms::Value> {
                #ms::Vec::new()
            }
        }

        #(#references)*
    })
}

/// A referenced relation must declare an id to point at.
fn references_check(ms: &TokenStream, target: &Path) -> TokenStream {
    let message = format!(
        "`{}` declares no `#[id]` column, so a foreign key cannot point at it",
        quote!(#target).to_string().replace(' ', "")
    );

    quote_spanned! {target.span()=>
        const _: () = ::core::assert!(
            !<#target as #ms::Relation>::ID_COLUMNS.is_empty(),
            #message
        );
    }
}
