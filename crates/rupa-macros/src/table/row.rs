//! The `as Row` form of `table!`: a row struct carrying the derives, with
//! the marker name as an alias of it.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, LitStr, Visibility};

use crate::model;

/// The parts of the `as Row` form: a row struct carrying the derives, and the
/// marker name as an alias of it, so there is one type and one set of handles.
pub(super) struct RowForm<'a> {
    pub(super) root: TokenStream,
    pub(super) rupa_attr: Option<TokenStream>,
    pub(super) docs: &'a [Attribute],
    pub(super) derives: &'a [Attribute],
    pub(super) vis: &'a Visibility,
    pub(super) marker: &'a Ident,
    pub(super) row: &'a Ident,
    pub(super) schema: Option<&'a LitStr>,
    pub(super) name: &'a LitStr,
    pub(super) fields: &'a [model::Field],
    pub(super) raw: Vec<Vec<Attribute>>,
}

impl RowForm<'_> {
    pub(super) fn expand(self) -> TokenStream {
        let Self {
            root,
            rupa_attr,
            docs,
            derives,
            vis,
            marker,
            row,
            schema,
            name,
            fields,
            raw,
        } = self;
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
            #(#derives)*
            #[derive(
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
}
