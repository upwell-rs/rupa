//! `#[derive(Joined)]`: a struct that joined rows decode into, with each field
//! taken from one of the join's entities.
//!
//! ```ignore
//! #[derive(Joined)]
//! struct OrderLine {
//!     #[from(Order::id)] order_id: i64,
//!     #[from(User::name)] customer: String,
//!     #[from(Coupon::code)] coupon: Option<String>, // Coupon is left-joined
//!     #[flatten] user: User,                         // a whole entity
//! }
//! ```
//!
//! Generated: `FromRow` (fields read in order, by position) and
//! `Joined<R, M>`, whose projection selects exactly those columns. The impl
//! is generic over the join's row type `R`: each field's entity is found in
//! `R` by type (`PartOf`), and a field from a left-joined entity must be an
//! `Option` (`Admits`). Nothing is matched by name.
//!
//! An entity that is a source twice (a self-join) is ambiguous by type; pin
//! such fields to a source position: `#[from(User::name, source = 1)]`,
//! `#[flatten(source = 1)]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::parse::ParseStream;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, LitInt, Path, Token, Type};

use crate::model::{self, column_path, option_inner};

enum Source {
    /// `#[from(Entity::field)]`
    Column { entity: Path, field: Ident },
    /// `#[flatten]`: the whole entity (`E`, or `Option<E>` when left-joined).
    Flatten { entity: Type },
}

/// `source = N`: the source position, when the entity is a source twice.
fn parse_source(input: ParseStream) -> syn::Result<LitInt> {
    let key: Ident = input.parse()?;
    if key != "source" {
        return Err(syn::Error::new(key.span(), "expected `source = N`"));
    }

    input.parse::<Token![=]>()?;
    input.parse()
}

struct Part {
    ident: Ident,
    ty: Type,
    source: Source,
    /// `source = N`
    at: Option<LitInt>,
}

fn parts(input: &DeriveInput) -> syn::Result<Vec<Part>> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Joined can only be derived for structs",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Joined requires a struct with named fields",
        ));
    };
    named
        .named
        .iter()
        .map(|f| {
            let ident = f.ident.clone().expect("named field");
            let mut source = None;
            let mut at = None;
            for attr in &f.attrs {
                let found = if attr.path().is_ident("from") {
                    let (mut entity, pos) = attr.parse_args_with(|input: ParseStream| {
                        let entity = input.call(Path::parse_mod_style)?;
                        let pos = if input.peek(Token![,]) {
                            input.parse::<Token![,]>()?;
                            Some(parse_source(input)?)
                        } else {
                            None
                        };
                        Ok((entity, pos))
                    })?;
                    at = pos;
                    if entity.segments.len() < 2 {
                        return Err(syn::Error::new_spanned(
                            &entity,
                            "name the entity and its field: `#[from(Entity::field)]`",
                        ));
                    }

                    let field = entity.segments.pop().expect("two segments").into_value().ident;
                    entity.segments.pop_punct();
                    Source::Column { entity, field }
                } else if attr.path().is_ident("flatten") {
                    if !matches!(attr.meta, syn::Meta::Path(_)) {
                        at = Some(attr.parse_args_with(parse_source)?);
                    }

                    let entity = option_inner(&f.ty).unwrap_or(&f.ty).clone();
                    Source::Flatten { entity }
                } else {
                    continue;
                };
                if source.replace(found).is_some() {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "a field takes one `#[from(..)]` or `#[flatten]`",
                    ));
                }
            }

            let source = source.ok_or_else(|| {
                syn::Error::new(
                    ident.span(),
                    "say where this field comes from: `#[from(Entity::field)]`, or `#[flatten]` for a whole entity",
                )
            })?;
            Ok(Part {
                ident,
                ty: f.ty.clone(),
                source,
                at,
            })
        })
        .collect()
}

/// A pinned position, or one inferred by type.
pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    model::reject_generics(&input, "Joined")?;
    let ms = model::support_path(&input)?;
    let parts = parts(&input)?;
    let ident = &input.ident;
    let row = format_ident!("__R");

    let mut params = Vec::new();
    let mut markers = Vec::new();
    let mut bounds = Vec::new();
    let mut projection = Vec::new();
    let mut reads = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let kind = format_ident!("__N{k}");
        let at = match &part.at {
            Some(n) => quote!(#ms::At<#n>),
            None => {
                let at = format_ident!("__I{k}");
                params.push(quote!(#at));
                quote!(#at)
            }
        };
        let ty = &part.ty;
        let shape = if option_inner(ty).is_some() {
            quote!(#ms::Opt)
        } else {
            quote!(#ms::Plain)
        };
        let (entity, entity_span) = match &part.source {
            Source::Column { entity, .. } => (quote!(#entity), entity.span()),
            Source::Flatten { entity } => (quote!(#entity), entity.span()),
        };
        let part_of = quote!(<#row as #ms::PartOf<#entity, #at, #kind>>);
        bounds.push(quote_spanned!(entity_span=> #row: #ms::PartOf<#entity, #at, #kind>));
        bounds.push(quote_spanned!(ty.span()=> #kind: #ms::Admits<#shape>));
        params.push(quote!(#kind));
        markers.push(quote!((#at, #kind)));

        let fi = &part.ident;
        match &part.source {
            Source::Column { entity, field } => {
                let column = column_path(&quote!(#entity), field);
                projection.push(quote! {
                    columns.push(#ms::ColumnRef::of(#part_of::ID, #column.name()));
                });
                reads.push(quote_spanned! {ty.span()=>
                    let #fi: #ty = #ms::ReadAs::<#ty>::read_as(&#column, row, base)?;
                    base += 1;
                });
            }
            Source::Flatten { entity } => {
                projection.push(quote! {
                    let source = #part_of::ID;
                    columns.extend(
                        <#entity as #ms::Entity>::COLUMNS
                            .iter()
                            .map(|c| #ms::ColumnRef::of(source, c.name)),
                    );
                });
                reads.push(quote_spanned! {ty.span()=>
                    let #fi = <#ty as #ms::JoinPart>::decode(row, base)?;
                    base += <#ty as #ms::JoinPart>::width();
                });
            }
        }
    }

    let idents = parts.iter().map(|p| &p.ident);

    Ok(quote! {
        impl #ms::FromRow for #ident {
            #[allow(unused_assignments, reason = "the last field's width is not needed")]
            fn from_row(row: &dyn #ms::Row) -> #ms::Result<Self, #ms::ResultError> {
                let mut base = 0usize;
                #(#reads)*
                #ms::Result::Ok(Self { #(#idents),* })
            }
        }

        impl<#row, #(#params),*> #ms::Joined<#row, (#(#markers,)*)> for #ident
        where
            #(#bounds,)*
        {
            fn projection() -> #ms::Vec<#ms::ColumnRef> {
                let mut columns = #ms::Vec::new();
                #(#projection)*
                columns
            }
        }
    })
}
