//! Parsing shared by all derives: the struct, its fields, and the attributes
//! `#[id]`, `#[column(..)]` and `#[rupa(crate = ..)]`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, GenericArgument, Ident, LitStr, Path, PathArguments, Type};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Infer,
    Scalar,
    Json,
}

pub struct Field {
    pub ident: Ident,
    /// Field name without a raw-identifier prefix.
    pub name: String,
    pub ty: Type,
    pub column: LitStr,
    pub id: bool,
    pub generated: bool,
    pub kind: Kind,
    /// Whether `#[column(..)]` appeared at all (rejected on companion structs).
    pub has_column_attr: bool,
}

impl Field {
    pub fn span(&self) -> Span {
        self.ty.span()
    }
}

/// The path that generated code uses for its support items:
/// `<crate>::__macro_support`, where `<crate>` is `::rupa` unless overridden
/// with `#[rupa(crate = path)]`.
pub fn support_path(input: &DeriveInput) -> syn::Result<TokenStream> {
    let mut krate: Option<Path> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("rupa") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("crate") {
                    krate = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("unknown `rupa` attribute; expected `crate = path`"))
                }
            })?;
        }
    }
    Ok(match krate {
        Some(p) => quote!(#p::__macro_support),
        None => quote!(::rupa::__macro_support),
    })
}

pub fn reject_generics(input: &DeriveInput, what: &str) -> syn::Result<()> {
    if input.generics.params.is_empty() && input.generics.where_clause.is_none() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            &input.generics,
            format!(
                "{what} cannot be derived for generic types: column kinds are resolved per concrete field type"
            ),
        ))
    }
}

pub fn fields(input: &DeriveInput, what: &str) -> syn::Result<Vec<Field>> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            format!("{what} can only be derived for structs"),
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            format!("{what} requires a struct with named fields"),
        ));
    };

    named
        .named
        .iter()
        .map(|f| {
            let ident = f.ident.clone().expect("named field");
            let name = ident.to_string().trim_start_matches("r#").to_owned();
            let mut field = Field {
                column: LitStr::new(&name, ident.span()),
                name,
                ident,
                ty: f.ty.clone(),
                id: false,
                generated: false,
                kind: Kind::Infer,
                has_column_attr: false,
            };
            for attr in &f.attrs {
                if attr.path().is_ident("id") {
                    field.id = true;
                    if !matches!(attr.meta, syn::Meta::Path(_)) {
                        attr.parse_nested_meta(|m| {
                            if m.path.is_ident("generated") {
                                field.generated = true;
                                Ok(())
                            } else {
                                Err(m.error("unknown `id` option; expected `generated`"))
                            }
                        })?;
                    }
                } else if attr.path().is_ident("column") {
                    field.has_column_attr = true;
                    attr.parse_nested_meta(|m| {
                        if m.path.is_ident("name") {
                            field.column = m.value()?.parse()?;
                        } else if m.path.is_ident("scalar") {
                            field.kind = Kind::Scalar;
                        } else if m.path.is_ident("json") {
                            field.kind = Kind::Json;
                        } else if m.path.is_ident("generated") {
                            field.generated = true;
                        } else {
                            return Err(m.error(
                                "unknown `column` option; expected `name = \"..\"`, `scalar`, `json` or `generated`",
                            ));
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(field)
        })
        .collect()
}

/// `Some(U)` if `ty` is syntactically `Option<U>`.
pub fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None;
    }
    let last = p.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(t) if args.args.len() == 1 => Some(t),
        _ => None,
    }
}

/// The hidden struct of field probes a derived entity exposes.
pub fn fields_struct_ident(entity: &Ident) -> Ident {
    format_ident!("__RupaFields{}", entity)
}

/// An expression evaluating to the typed `Column` handle of `entity`'s field
/// `field`, resolved through the entity's probe for that field.
pub fn column_expr(
    ms: &TokenStream,
    entity: &TokenStream,
    field: &Ident,
    span: Span,
) -> TokenStream {
    quote_spanned! {span=> {
        #[allow(unused_imports)]
        use #ms::{JsonLevel as _, NullableJsonLevel as _, ScalarLevel as _};
        let probe = <#entity>::__rupa_fields().#field;
        #ms::column_from_probe(&probe, (&&&probe).__rupa_codec())
    }}
}

/// Reads `#[<name>(entity = Path)]`, used by capability derives on companion structs.
pub fn companion_entity(input: &DeriveInput, name: &str) -> syn::Result<Option<Path>> {
    let mut entity = None;
    for attr in &input.attrs {
        if attr.path().is_ident(name) {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("entity") {
                    entity = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error(format!("unknown `{name}` option; expected `entity = Type`")))
                }
            })?;
        }
    }
    Ok(entity)
}

pub fn reject_column_attrs(fields: &[Field], what: &str) -> syn::Result<()> {
    match fields.iter().find(|f| f.has_column_attr) {
        Some(f) => Err(syn::Error::new(
            f.ident.span(),
            format!("`#[column]` settings belong on the entity, not on this {what} struct"),
        )),
        None => Ok(()),
    }
}
