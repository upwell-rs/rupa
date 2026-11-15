//! Parsing shared by all derives: the struct, its fields, and the attributes
//! `#[id]`, `#[column(..)]` and `#[rupa(crate = ..)]`.

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
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
    /// `#[id(generated)]`: an identity key, rather than a database default.
    pub identity: bool,
    pub kind: Kind,
    /// Whether `#[column(..)]` appeared at all (rejected on companion structs).
    pub has_column_attr: bool,
    /// `#[column(references = E)]`: the entity whose id this column holds.
    pub references: Option<Path>,
    /// `#[column(relation = name)]`: the relation accessor's name.
    pub relation: Option<Ident>,
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
            let mut field = Field::new(ident, f.ty.clone());

            field.apply_attrs(&f.attrs)?;

            Ok(field)
        })
        .collect()
}

impl Field {
    /// A field with no attributes: a plain column named after the field.
    pub fn new(ident: Ident, ty: Type) -> Self {
        let name = ident.to_string().trim_start_matches("r#").to_owned();

        Self {
            column: LitStr::new(&name, ident.span()),
            name,
            ident,
            ty,
            id: false,
            generated: false,
            identity: false,
            kind: Kind::Infer,
            has_column_attr: false,
            references: None,
            relation: None,
        }
    }

    /// Applies the `#[id]` and `#[column(..)]` attributes among `attrs`, the
    /// one grammar of every derive and of `table!`.
    pub fn apply_attrs(&mut self, attrs: &[syn::Attribute]) -> syn::Result<()> {
        for attr in attrs {
            if attr.path().is_ident("id") {
                self.apply_id(attr)?;
            } else if attr.path().is_ident("column") {
                self.apply_column(attr)?;
            }
        }

        Ok(())
    }

    fn apply_id(&mut self, attr: &syn::Attribute) -> syn::Result<()> {
        self.id = true;

        if !matches!(attr.meta, syn::Meta::Path(_)) {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("generated") {
                    self.generated = true;
                    self.identity = true;

                    Ok(())
                } else {
                    Err(m.error("unknown `id` option; expected `generated`"))
                }
            })?;
        }

        Ok(())
    }

    fn apply_column(&mut self, attr: &syn::Attribute) -> syn::Result<()> {
        self.has_column_attr = true;

        attr.parse_nested_meta(|m| {
            if m.path.is_ident("name") {
                self.column = m.value()?.parse()?;
            } else if m.path.is_ident("scalar") {
                self.kind = Kind::Scalar;
            } else if m.path.is_ident("json") {
                self.kind = Kind::Json;
            } else if m.path.is_ident("generated") {
                self.generated = true;
            } else if m.path.is_ident("references") {
                self.references = Some(m.value()?.parse()?);
            } else if m.path.is_ident("relation") {
                self.relation = Some(m.value()?.parse()?);
            } else {
                return Err(m.error(
                    "unknown `column` option; expected `name = \"..\"`, `scalar`, `json`, `generated`, `references = Entity` or `relation = name`",
                ));
            }

            Ok(())
        })?;

        match (&self.relation, &self.references) {
            (Some(relation), None) => Err(syn::Error::new(
                relation.span(),
                "`relation` names a `references` relation; add `references = Entity`",
            )),
            _ => Ok(()),
        }
    }
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

/// Field types `ScalarColumn` is implemented for, recognized by name so the
/// derive can write `Scalar` directly: an exact type that rust-analyzer
/// resolves, unlike a kind selected by evaluating a constant. Other types keep
/// the compile-time selection. A user type shadowing one of these names fails
/// with the scalar codec's "not a scalar column type" error, which
/// `#[column(json)]` resolves. So does `Uuid` or a chrono type in a build
/// without rupa's `uuid`/`chrono` features, which would otherwise have been
/// stored as JSON: enable the feature, or opt into JSON explicitly.
pub fn known_scalar(ty: &Type) -> bool {
    const PLAIN: [&str; 11] = [
        "bool",
        "i16",
        "i32",
        "i64",
        "f32",
        "f64",
        "String",
        "Uuid",
        "NaiveDate",
        "NaiveTime",
        "NaiveDateTime",
    ];

    if let Some(inner) = option_inner(ty) {
        return known_scalar(inner);
    }

    let Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    let name = last.ident.to_string();
    let argument = |expected: &str| match &last.arguments {
        PathArguments::AngleBracketed(args) if args.args.len() == 1 => matches!(
            args.args.first(),
            Some(GenericArgument::Type(Type::Path(arg)))
                if arg.qself.is_none() && arg.path.segments.last().is_some_and(|s| s.ident == expected)
        ),
        _ => false,
    };

    if path.qself.is_some() {
        return false;
    }

    match name.as_str() {
        "Vec" => argument("u8"),
        "DateTime" => argument("Utc"),
        name => PLAIN.contains(&name) && last.arguments.is_none(),
    }
}

/// The name of the column handle constant `#[derive(Entity)]` emits for
/// `field`: `created_at` becomes `CREATED_AT`. It keeps the field's span, so
/// go-to-definition on a use lands on the field.
pub fn const_ident(field: &Ident) -> Ident {
    let name = field.to_string();

    Ident::new(&name.trim_start_matches("r#").to_uppercase(), field.span())
}

/// The typed `Column` handle of `entity`'s field `field`: its constant.
///
/// The constant's name resolves with macro hygiene but keeps the field's
/// location, so a misspelled field is reported there without rustc offering
/// the constant's uppercase name as a replacement for the field. The block
/// carries the field's span, so errors about the handle (a JSON path on a
/// scalar column) point at the field rather than at the macro.
pub fn column_path(entity: &TokenStream, field: &Ident) -> TokenStream {
    let mut handle = const_ident(field);

    handle.set_span(field.span().resolved_at(Span::call_site()));

    quote_spanned!(field.span()=> { <#entity>::#handle })
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

#[cfg(test)]
mod tests;
