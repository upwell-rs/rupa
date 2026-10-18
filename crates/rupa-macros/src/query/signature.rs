//! The declared signature: parameters and the result shape.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{FnArg, Ident, Pat, ReturnType, Type};

/// What the declared result type asks for.
pub(crate) enum Shape {
    /// `Vec<T>`, `Option<T>`, `T`: rows of entity `T`.
    Rows {
        entity: Box<Type>,
    },
    Bool,
    U64,
}

pub(crate) fn single_generic<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != name {
        return None;
    }

    let syn::PathArguments::AngleBracketed(a) = &last.arguments else {
        return None;
    };
    match (a.args.len(), a.args.first()?) {
        (1, syn::GenericArgument::Type(t)) => Some(t),
        _ => None,
    }
}

pub(crate) fn is_ident(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

pub(crate) fn shape_of(ret: &Type) -> Shape {
    if is_ident(ret, "bool") {
        Shape::Bool
    } else if is_ident(ret, "u64") {
        Shape::U64
    } else {
        let entity = single_generic(ret, "Vec")
            .or_else(|| single_generic(ret, "Option"))
            .unwrap_or(ret)
            .clone();
        Shape::Rows {
            entity: Box::new(entity),
        }
    }
}

/// A declared query function: its parameters and result type.
pub struct Signature {
    pub params: Vec<(Ident, Type)>,
    /// `R` of `Query<R>`.
    pub result: Type,
    pub result_span: Span,
}

impl Signature {
    /// Reads parameters and the result type. `-> R` and `-> Query<R>` both
    /// mean `Query<R>`. A receiver is skipped (repository methods have one).
    pub fn from_sig(sig: &syn::Signature) -> syn::Result<Self> {
        let mut params = Vec::new();
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(_) => {}
                FnArg::Typed(pt) => match &*pt.pat {
                    Pat::Ident(pi) => params.push((pi.ident.clone(), (*pt.ty).clone())),
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "query parameters must be plain names",
                        ));
                    }
                },
            }
        }

        let ReturnType::Type(_, ret) = &sig.output else {
            return Err(syn::Error::new_spanned(
                sig,
                "a query must declare its result type: `Vec<T>`, `Option<T>`, `T`, `bool` or `u64`",
            ));
        };
        let result = single_generic(ret, "Query").unwrap_or(ret).clone();
        Ok(Signature {
            result_span: ret.span(),
            params,
            result,
        })
    }
}
