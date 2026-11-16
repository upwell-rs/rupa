//! `table!`: a typed table snapshot, with no row type unless declared `as Row`. It emits a unit marker
//! implementing `Relation`, plus the same typed column handle constants and
//! `ColumnMeta` values `#[derive(Entity)]` emits for an equivalent struct.
//!
//! ```ignore
//! rupa::table!(Accounts = "main"."accounts" {
//!     #[id(generated)] id: i64,
//!     nick: Option<VarChar<64>>,
//!     #[column(references = Owner)] owner_id: i64,
//!     prefs: Prefs,
//! });
//! ```
//!
//! The marker may write through `update` and `delete` with explicit `set`
//! calls and filters, and takes inserts of an `Insertable` input struct
//! (`#[insertable(entity = Marker)]`); it has no row to read or look up by id.

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
    row: Option<Ident>,
    fields: Vec<model::Field>,
    raw: Vec<Vec<Attribute>>,
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
        let row = if input.peek(Token![as]) {
            input.parse::<Token![as]>()?;

            Some(input.parse::<Ident>().map_err(|e| {
                syn::Error::new(e.span(), "expected the name of the row type after `as`")
            })?)
        } else {
            None
        };
        let body;

        braced!(body in input);

        let decls = Punctuated::<FieldDecl, Token![,]>::parse_terminated(&body)?;
        let (fields, raw) = decls.into_iter().map(|d| (d.0, d.1)).unzip();

        Ok(Self {
            attrs,
            vis,
            ident,
            schema,
            name,
            row,
            fields,
            raw,
            krate,
        })
    }
}

/// One declared column, parsed with the derive's attribute grammar.
struct FieldDecl(model::Field, Vec<Attribute>);

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

        Ok(Self(field, attrs))
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
        row,
        fields,
        raw,
        krate,
    } = table;
    let (root, ms) = match &krate {
        Some(path) => (quote!(#path), quote!(#path::__macro_support)),
        None => (quote!(::rupa), quote!(::rupa::__macro_support)),
    };
    let schema_lit = schema.clone();
    let schema = match schema {
        Some(schema) => quote!(#ms::Option::Some(#schema)),
        None => quote!(#ms::Option::None),
    };
    let ids: Vec<_> = fields.iter().filter(|f| f.id).map(|f| &f.column).collect();

    column::check_handles(&ident, &fields, "table!")?;

    if let Some(row) = row {
        let rupa_attr = krate.map(|path| quote!(#[rupa(crate = #path)]));

        return Ok(row_form(
            &root, rupa_attr, &attrs, &vis, &ident, &row, schema_lit, &name, &fields, raw,
        ));
    }

    if let Some(relation) = fields.iter().find_map(|f| f.relation.as_ref()) {
        return Err(syn::Error::new(
            relation.span(),
            "`relation = ..` needs a row type: declare the table `as Row`",
        ));
    }

    let consts = fields.iter().map(|f| column::handle(&ms, &ident, &vis, f));
    let metas = fields.iter().map(|f| column::meta(&ms, f));
    let required = column::insert_required(&ident, &vis, &fields);
    let references = fields.iter().enumerate().filter_map(|(position, f)| {
        f.references
            .as_ref()
            .map(|target| references_check(&ms, &ident, position, f, target))
    });

    Ok(quote! {
        #(#attrs)*
        #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy)]
        #vis struct #ident;

        impl #ident {
            #(#consts)*

            #required
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

/// A foreign key must point at a relation with a single id column of the
/// field's SQL type, as the derive checks for entities.
fn references_check(
    ms: &TokenStream,
    owner: &Ident,
    position: usize,
    field: &model::Field,
    target: &Path,
) -> TokenStream {
    let target_name = quote!(#target).to_string().replace(' ', "");
    let arity = format!(
        "`{target_name}` must declare exactly one `#[id]` column for `{}` to reference it; \
         a composite key needs a hand-written `ForeignKey`",
        field.name
    );
    let ty = format!(
        "`{}` does not have the type of the id of `{target_name}`",
        field.name
    );

    quote_spanned! {target.span()=>
        const _: () = ::core::assert!(
            <#target as #ms::Relation>::ID_COLUMNS.len() == 1,
            #arity
        );

        const _: () = ::core::assert!(
            <#target as #ms::Relation>::ID_COLUMNS.len() != 1
                || #ms::references_id_type(
                <#target as #ms::Relation>::COLUMNS,
                <#target as #ms::Relation>::ID_COLUMNS,
                &<#owner as #ms::Relation>::COLUMNS[#position],
            ),
            #ty
        );
    }
}

/// The `as Row` form: a row struct carrying the derives, and the marker name
/// as an alias of it, so there is one type and one set of handles.
#[allow(clippy::too_many_arguments)]
fn row_form(
    root: &TokenStream,
    rupa_attr: Option<TokenStream>,
    docs: &[Attribute],
    vis: &Visibility,
    marker: &Ident,
    row: &Ident,
    schema: Option<LitStr>,
    name: &LitStr,
    fields: &[model::Field],
    raw: Vec<Vec<Attribute>>,
) -> TokenStream {
    let entity_attr = match schema {
        Some(schema) => quote!(#[entity(table = #name, schema = #schema)]),
        None => quote!(#[entity(table = #name)]),
    };
    let members = fields.iter().zip(raw).map(|(f, attrs)| {
        let (ident, ty) = (&f.ident, &f.ty);

        quote!(#(#attrs)* #vis #ident: #ty)
    });

    quote! {
        #(#docs)*
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            ::core::cmp::PartialEq,
            #root::Entity,
            #root::Gettable,
            #root::Insertable,
            #root::Updatable,
            #root::Deletable,
        )]
        #entity_attr
        #rupa_attr
        #vis struct #row {
            #(#members,)*
        }

        #[doc = "The table, as the row type declared beside it."]
        #vis type #marker = #row;
    }
}
