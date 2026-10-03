//! `#[dsl::function]`: a dialect-aware SQL function usable from the typed
//! builder and from `#[query]`.
//!
//! ```ignore
//! #[dsl::function]                                  // runtime-checked
//! fn ilike(dialect: &dyn Dialect, s: Expr<String>, p: Expr<String>) -> Result<Expr<bool>, DslError> {
//!     match dialect.id() { .. }
//! }
//!
//! #[dsl::function(requires = JsonContainment, eval = eval_contains)]   // statically gated
//! fn json_contains(doc: Expr<serde_json::Value>, part: Expr<serde_json::Value>) -> Expr<bool> {
//!     Expr::raw_op("@>", doc, part)
//! }
//! ```
//!
//! The body is the function's *lowering*: it runs when a query is rendered,
//! with the target dialect, turning the call into SQL-level IR. Its
//! parameters are expressions (bound parameters when they carry data), so it
//! cannot interpolate values.
//!
//! Generated, all under the function's name:
//! - `fn name(args: impl IntoExpr<T>..) -> Expr<R>`: builds the (un-lowered) call;
//! - `struct name {}`: a marker type (types and values live in separate
//!   namespaces, so `use path::name` imports both). It implements
//!   `DslAvailable<D>` for the dialects the function supports: every dialect,
//!   or those with the `requires` capabilities. `#[repository]` adds that
//!   bound to its implementation, so a statically gated function fails to
//!   compile against a dialect without the capability.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::{FnArg, GenericArgument, Ident, ItemFn, LitStr, Pat, PathArguments, ReturnType, Type};

/// `Some(T)` if `ty` is `Name<T>` (last path segment `Name`).
fn generic_arg<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(a) = &last.arguments else {
        return None;
    };
    match a.args.first()? {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

fn is_dyn_dialect(ty: &Type) -> bool {
    let Type::Reference(r) = ty else { return false };
    matches!(&*r.elem, Type::TraitObject(t) if t.bounds.iter().any(|b| matches!(
        b, syn::TypeParamBound::Trait(tb) if tb.path.segments.last().is_some_and(|s| s.ident == "Dialect")
    )))
}

pub fn function(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let mut requires: Vec<Ident> = Vec::new();
    let mut eval: Option<syn::Path> = None;
    let mut name: Option<LitStr> = None;
    let mut krate: Option<syn::Path> = None;
    let parser = syn::meta::parser(|m| {
        if m.path.is_ident("requires") {
            let v = m.value()?;
            if v.peek(syn::token::Bracket) {
                let content;
                syn::bracketed!(content in v);
                requires.extend(content.parse_terminated(Ident::parse, syn::Token![,])?);
            } else {
                requires.push(v.parse()?);
            }
        } else if m.path.is_ident("eval") {
            eval = Some(m.value()?.parse()?);
        } else if m.path.is_ident("name") {
            name = Some(m.value()?.parse()?);
        } else if m.path.is_ident("crate") {
            krate = Some(m.value()?.parse()?);
        } else {
            return Err(m.error(
                "unknown option; expected `requires = Capability`, `eval = path`, `name = \"..\"` or `crate = path`",
            ));
        }
        Ok(())
    });
    parser.parse2(args)?;
    use syn::parse::Parse as _;
    let ms = match krate {
        Some(p) => quote!(#p::__macro_support),
        None => quote!(::rupa::__macro_support),
    };

    let func: ItemFn = syn::parse2(item)?;
    let sig = &func.sig;
    if !sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "a DSL function is not generic: it is lowered at render time with the target dialect as `&dyn Dialect`; \
             gate it statically with `requires = Capability`",
        ));
    }
    if let Some(a) = &sig.asyncness {
        return Err(syn::Error::new_spanned(
            a,
            "a DSL function's lowering is synchronous",
        ));
    }
    let fname = &sig.ident;
    let display = name.map(|n| n.value()).unwrap_or_else(|| fname.to_string());
    let vis = &func.vis;
    let attrs = &func.attrs;

    // Parameters: optional leading `dialect: &dyn Dialect`, then `name: Expr<T>`.
    let mut inputs = sig.inputs.iter().peekable();
    let mut takes_dialect = false;
    if let Some(FnArg::Typed(pt)) = inputs.peek()
        && is_dyn_dialect(&pt.ty)
    {
        takes_dialect = true;
        inputs.next();
    }
    let mut params: Vec<(Ident, Type)> = Vec::new();
    for arg in inputs {
        let FnArg::Typed(pt) = arg else {
            return Err(syn::Error::new_spanned(
                arg,
                "a DSL function takes no receiver",
            ));
        };
        let Pat::Ident(pi) = &*pt.pat else {
            return Err(syn::Error::new_spanned(
                &pt.pat,
                "parameters must be plain names",
            ));
        };
        let Some(inner) = generic_arg(&pt.ty, "Expr") else {
            return Err(syn::Error::new_spanned(
                &pt.ty,
                "DSL function parameters are expressions: `Expr<T>` (the dialect, if needed, comes first as `&dyn Dialect`)",
            ));
        };
        params.push((pi.ident.clone(), inner.clone()));
    }

    // Return: `Expr<R>` or `Result<Expr<R>, DslError>`.
    let ReturnType::Type(_, ret) = &sig.output else {
        return Err(syn::Error::new_spanned(
            sig,
            "a DSL function returns `Expr<R>` or `Result<Expr<R>, DslError>`",
        ));
    };
    let (out, fallible) = if let Some(r) = generic_arg(ret, "Expr") {
        (r.clone(), false)
    } else if let Some(inner) = generic_arg(ret, "Result").and_then(|ok| generic_arg(ok, "Expr")) {
        (inner.clone(), true)
    } else {
        return Err(syn::Error::new_spanned(
            ret,
            "a DSL function returns `Expr<R>` or `Result<Expr<R>, DslError>`",
        ));
    };

    // The user's body, kept as an inner function with its own signature.
    let body = &func.block;
    let inner_inputs = &sig.inputs;
    let inner_output = &sig.output;
    let arity = params.len();
    let arg_names: Vec<_> = params.iter().map(|(n, _)| n).collect();
    let arg_types: Vec<_> = params.iter().map(|(_, t)| t).collect();
    let dialect_arg = if takes_dialect {
        quote!(dialect,)
    } else {
        quote!()
    };
    let call = quote!(__rupa_body(#dialect_arg #(#arg_names),*));
    let lowered = if fallible {
        quote!(#call.map(#ms::Expr::into_node))
    } else {
        quote!(::core::result::Result::Ok(#call.into_node()))
    };
    let capability_check = requires.iter().map(|cap| {
        quote! {
            if !dialect.supports(<#ms::caps::#cap as #ms::CapabilityMarker>::CAP) {
                return ::core::result::Result::Err(#ms::DslError::unsupported(#display, dialect.id()));
            }
        }
    });
    let eval_field = match &eval {
        Some(p) => quote!(::core::option::Option::Some(#p)),
        None => quote!(::core::option::Option::None),
    };
    let available = if requires.is_empty() {
        quote!(impl<D: #ms::Dialect> #ms::DslAvailable<D> for #fname {})
    } else {
        quote!(impl<D: #ms::Dialect #(+ #ms::Supports<#ms::caps::#requires>)*> #ms::DslAvailable<D> for #fname {})
    };
    let lower_fn = format_ident!("__rupa_lower");
    let builder_args = params
        .iter()
        .map(|(n, t)| quote!(#n: impl #ms::IntoExpr<#t>));
    let builder_nodes = params
        .iter()
        .map(|(n, t)| quote!(<_ as #ms::IntoExpr<#t>>::into_node(#n)));

    Ok(quote! {
        #[doc = concat!("Marker type of the DSL function [`", stringify!(#fname), "()`]; see `DslAvailable`.")]
        #[allow(non_camel_case_types)]
        #vis struct #fname {}

        impl #fname {
            #[doc(hidden)]
            #vis fn __rupa_def() -> &'static #ms::DslFnDef {
                #[allow(unused_variables, clippy::needless_return)]
                fn #lower_fn(
                    dialect: &dyn #ms::Dialect,
                    args: ::std::vec::Vec<#ms::ExprNode>,
                ) -> ::core::result::Result<#ms::ExprNode, #ms::DslError> {
                    #[allow(unused_variables)]
                    fn __rupa_body(#inner_inputs) #inner_output #body

                    #(#capability_check)*
                    if args.len() != #arity {
                        return ::core::result::Result::Err(#ms::DslError::Arity {
                            function: #display,
                            expected: #arity,
                            got: args.len(),
                        });
                    }
                    let mut args = args.into_iter();
                    #(let #arg_names = #ms::Expr::<#arg_types>::from_node(args.next().unwrap());)*
                    #lowered
                }
                static DEF: #ms::DslFnDef = #ms::DslFnDef { name: #display, lower: #lower_fn, eval: #eval_field };
                &DEF
            }
        }

        #available

        #(#attrs)*
        #vis fn #fname(#(#builder_args),*) -> #ms::Expr<#out> {
            #ms::Expr::dsl(#fname::__rupa_def(), ::std::vec![#(#builder_nodes),*])
        }
    })
}
