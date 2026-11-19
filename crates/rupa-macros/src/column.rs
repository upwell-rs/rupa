//! The column items shared by `#[derive(Entity)]` and `table!`: one typed
//! handle constant per field, and the [`ColumnMeta`](rupa_core::ColumnMeta)
//! expression describing it, so both produce the same columns.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::Ident;
use syn::spanned::Spanned;

use crate::model::{self, Kind, const_ident, known_scalar, option_inner};

/// Prefix of the hidden associated constants the macros emit next to the
/// column handles.
pub const RESERVED: &str = "__RUPA_";

/// Rejects fields whose handle constants would collide with each other or
/// with the hidden ones.
pub fn check_handles(owner: &Ident, fields: &[model::Field], what: &str) -> syn::Result<()> {
    let mut handles = HashMap::new();

    for f in fields {
        let handle = const_ident(&f.ident).to_string();

        if handle.starts_with(RESERVED) {
            return Err(syn::Error::new(
                f.ident.span(),
                format!(
                    "field `{}` would get the column handle `{owner}::{handle}`, which `{what}` reserves; rename it",
                    f.ident
                ),
            ));
        }

        if let Some(earlier) = handles.insert(handle.clone(), &f.ident) {
            return Err(syn::Error::new(
                f.ident.span(),
                format!(
                    "fields `{earlier}` and `{}` would both get the column handle `{owner}::{handle}`; rename one",
                    f.ident
                ),
            ));
        }
    }

    Ok(())
}

/// The `ColumnMeta` of field `f`, built from its handle constant on `Self`.
pub fn meta(ms: &TokenStream, f: &model::Field) -> TokenStream {
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
                table: <#target as #ms::Relation>::TABLE,
                columns: <#target as #ms::Relation>::ID_COLUMNS,
            })
        };
    }

    meta
}

/// The typed column handle constant of field `f`, with the kind its type
/// selects at compile time, or the one `#[column(scalar|json)]` forces.
pub fn handle(
    ms: &TokenStream,
    owner: &Ident,
    vis: &syn::Visibility,
    f: &model::Field,
) -> TokenStream {
    let (ty, column, name) = (&f.ty, &f.column, &f.name);
    let handle = const_ident(&f.ident);
    let doc = format!("The `{name}` column.");
    let docs = &f.docs;
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
                "field `{name}` of `{owner}` can't be stored in a column: its type must implement \
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
                quote_spanned!(ty.span()=> <#ms::Stored<{ #ms::is_json_encoding(#owner::#selected) }> as #ms::Kinded>::Kind),
                quote_spanned! {ty.span()=>
                    match #owner::#selected {
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
        #(#docs)*
        #[allow(dead_code, reason = "a derived handle exists for every field, used or not")]
        #vis const #handle: #ms::Column<#owner, #ty, #kind> = #ms::Column::with_encoding_in(<#owner as #ms::Relation>::TABLE, #column, #encoding);
    }
}

/// The `__RUPA_INSERT_REQUIRED` constant: the fields an `Insertable` struct
/// must provide, each with the message reported when it does not.
pub fn insert_required(
    owner: &Ident,
    vis: &syn::Visibility,
    fields: &[model::Field],
) -> TokenStream {
    let required = fields
        .iter()
        .filter(|f| !f.generated && option_inner(&f.ty).is_none())
        .map(|f| {
            let name = &f.name;
            let message = format!(
                "this Insertable struct is missing field `{name}` of `{owner}`, which is neither nullable nor generated; \
                 add the field, or mark it `#[column(generated)]` on `{owner}` if the database supplies it"
            );

            quote!((#name, #message))
        });

    quote! {
        #[doc(hidden)]
        #vis const __RUPA_INSERT_REQUIRED: &'static [(&'static str, &'static str)] = &[#(#required,)*];
    }
}
