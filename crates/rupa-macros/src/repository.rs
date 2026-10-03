//! `#[repository]`: emits the trait as written (with `#[query]` attributes
//! removed) plus one implementation for `Repo<S>`.
//!
//! The author decides everything visible:
//!
//! - **Receivers.** All query methods take `&self` (`S` is a connection
//!   source: `Acquire` / `AcquireAsync`, e.g. `Shared`, a pool), or all take
//!   `&mut self` (`S` is the executor, e.g. `&mut tx`).
//! - **Sync or async, per method.** If any query method is `async`, the
//!   repository runs on an async executor, and its plain `fn` methods block
//!   in place. Otherwise everything is synchronous.
//! - **Errors.** Methods return `Result<R, E>`; the implementation requires
//!   `E: From<executor error>`.
//!
//! By default the trait is dyn-compatible (`Arc<dyn Trait>`): `async fn`s
//! become methods returning `Pin<Box<dyn Future + Send + '_>>`.
//! `#[repository(static_dispatch)]` keeps them unboxed as
//! `impl Future + Send`, which is zero-cost but not dyn-compatible.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{FnArg, ItemTrait, ReturnType, TraitItem, TraitItemFn, Type, TypeParamBound};

use crate::query::{QueryAttr, Signature, called_functions, query_body};

struct Method {
    item: TraitItemFn,
    attr: QueryAttr,
    is_async: bool,
    mut_receiver: bool,
    /// `R` and `E` of `Result<R, E>`.
    ok: Type,
    err: Type,
}

fn result_parts(ty: &Type) -> Option<(Type, Type)> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let mut types = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    });
    match (types.next(), types.next(), types.next()) {
        (Some(ok), Some(err), None) => Some((ok, err)),
        _ => None,
    }
}

pub fn repository(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let ms = quote!(::rupa::__macro_support);
    let mut static_dispatch = false;
    let parser = syn::meta::parser(|m| {
        if m.path.is_ident("static_dispatch") {
            static_dispatch = true;
            Ok(())
        } else {
            Err(m.error("unknown `repository` option; expected `static_dispatch`"))
        }
    });
    syn::parse::Parser::parse2(parser, args)?;

    let mut tr: ItemTrait = syn::parse2(item)?;
    if !tr.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &tr.generics,
            "generic repository traits are not supported yet",
        ));
    }

    // Collect query methods, stripping their `#[query]` attributes.
    let mut methods = Vec::new();
    for it in &mut tr.items {
        let TraitItem::Fn(f) = it else { continue };
        let Some(pos) = f.attrs.iter().position(|a| a.path().is_ident("query")) else {
            if f.default.is_none() {
                return Err(syn::Error::new_spanned(
                    &f.sig,
                    "a repository method needs `#[query(..)]`, or a default body",
                ));
            }
            continue;
        };
        let attr_meta = f.attrs.remove(pos);
        let attr: QueryAttr = attr_meta.parse_args()?;
        if f.default.is_some() {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a `#[query]` method must not have a body",
            ));
        }
        let mut_receiver = match f.sig.inputs.first() {
            Some(FnArg::Receiver(r)) if r.reference.is_some() => r.mutability.is_some(),
            _ => {
                return Err(syn::Error::new_spanned(
                    &f.sig,
                    "a repository method takes `&self` (shared, e.g. `Arc<dyn Trait>`) or `&mut self` (one executor)",
                ));
            }
        };
        let ReturnType::Type(_, ret) = &f.sig.output else {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a repository method returns `Result<R, E>`",
            ));
        };
        let (ok, err) = result_parts(ret).ok_or_else(|| {
            syn::Error::new_spanned(
                ret,
                "a repository method returns `Result<R, E>`, with `E: From<executor error>` (e.g. `rupa::core::exec::DynError`)",
            )
        })?;
        methods.push(Method {
            is_async: f.sig.asyncness.is_some(),
            mut_receiver,
            ok,
            err,
            attr,
            item: f.clone(),
        });
    }

    if methods.is_empty() {
        return Ok(quote!(#tr));
    }
    if methods.iter().any(|m| m.mut_receiver) && methods.iter().any(|m| !m.mut_receiver) {
        let m = methods
            .iter()
            .find(|m| m.mut_receiver != methods[0].mut_receiver)
            .unwrap();
        return Err(syn::Error::new_spanned(
            &m.item.sig,
            "all query methods of a repository take the same receiver: `&self` (through a connection source) \
             or `&mut self` (through one executor)",
        ));
    }
    let is_async = methods.iter().any(|m| m.is_async);
    let mut_receiver = methods[0].mut_receiver;

    // Rewrite async query methods in the emitted trait.
    for it in &mut tr.items {
        let TraitItem::Fn(f) = it else { continue };
        let Some(m) = methods.iter().find(|m| m.item.sig.ident == f.sig.ident) else {
            continue;
        };
        if !m.is_async {
            continue;
        }
        let (ok, err) = (&m.ok, &m.err);
        f.sig.asyncness = None;
        f.sig.output = if static_dispatch {
            syn::parse_quote!(-> impl #ms::Future<Output = ::core::result::Result<#ok, #err>> + #ms::Send)
        } else {
            syn::parse_quote!(-> #ms::Pin<#ms::Box<dyn #ms::Future<Output = ::core::result::Result<#ok, #err>> + #ms::Send + '_>>)
        };
    }

    // The implementation for `Repo<__S>`.
    let source_bound = match (is_async, mut_receiver) {
        (true, false) => quote!(#ms::AcquireAsync),
        (false, false) => quote!(#ms::Acquire),
        (true, true) => quote!(#ms::AsyncExecutor),
        (false, true) => quote!(#ms::Executor),
    };
    let mut errors: Vec<&Type> = Vec::new();
    for m in &methods {
        if !errors.iter().any(|e| {
            quote!(#e).to_string() == {
                let me = &m.err;
                quote!(#me).to_string()
            }
        }) {
            errors.push(&m.err);
        }
    }
    let error_bounds = errors
        .iter()
        .map(|e| quote!(#e: ::core::convert::From<<__S as #source_bound>::Error>));
    // Statically gated DSL functions: the implementation requires each used
    // function to be available on the source's dialect. The bound sits on
    // this impl, not on the trait: hand-written implementors are unaffected.
    let mut functions: Vec<syn::Path> = Vec::new();
    for m in &methods {
        for p in called_functions(&m.attr) {
            if !functions
                .iter()
                .any(|q| quote!(#q).to_string() == quote!(#p).to_string())
            {
                functions.push(p);
            }
        }
    }
    let dsl_bounds = functions
        .iter()
        .map(|p| quote!(#p: #ms::DslAvailable<<__S as #source_bound>::Dialect>));
    let super_bounds = tr.supertraits.iter().filter_map(|b| match b {
        TypeParamBound::Trait(t) => Some(quote!(#ms::Repo<__S>: #t)),
        _ => None,
    });

    let mut impl_fns = Vec::new();
    for m in &methods {
        let sig_info = Signature::from_sig(&m.item.sig)?;
        let mut sig_for_query = sig_info;
        sig_for_query.result = m.ok.clone();
        let query = query_body(&ms, &m.attr, &sig_for_query)?;
        let (ok, err) = (&m.ok, &m.err);

        let run = match (is_async, mut_receiver) {
            (true, false) => quote! {
                let mut __conn = #ms::AcquireAsync::acquire(self.source()).await?;
                ::core::result::Result::Ok(#ms::AsyncExecutor::run(&mut __conn, __query).await?)
            },
            (true, true) => quote! {
                ::core::result::Result::Ok(#ms::AsyncExecutor::run(self.source_mut(), __query).await?)
            },
            (false, false) => quote! {
                let mut __conn = #ms::Acquire::acquire(self.source())?;
                ::core::result::Result::Ok(#ms::Executor::run(&mut __conn, __query)?)
            },
            (false, true) => quote! {
                ::core::result::Result::Ok(#ms::Executor::run(self.source_mut(), __query)?)
            },
        };
        let result_ty = quote!(::core::result::Result<#ok, #err>);
        let body = if is_async {
            let fut = quote!(async move { let __out: #result_ty = (async { #run }).await; __out });
            if m.is_async {
                if static_dispatch {
                    fut
                } else {
                    quote!(#ms::Box::pin(#fut))
                }
            } else {
                // A plain `fn` in an async repository blocks in place.
                quote!(#ms::block_in_place(#fut))
            }
        } else {
            quote!({ let __out: #result_ty = (|| { #run })(); __out })
        };

        // Signature as emitted in the trait (async rewritten), with
        // parameters bound so the query can clone them.
        let emitted = tr
            .items
            .iter()
            .find_map(|it| match it {
                TraitItem::Fn(f) if f.sig.ident == m.item.sig.ident => Some(f.sig.clone()),
                _ => None,
            })
            .expect("method is in the trait");
        let span = m.item.sig.span();
        impl_fns.push(quote_spanned! {span=>
            #[allow(clippy::clone_on_copy, clippy::needless_question_mark)]
            #emitted {
                let __query = { #query };
                #body
            }
        });
    }

    let trait_ident = &tr.ident;
    Ok(quote! {
        #tr

        impl<__S> #trait_ident for #ms::Repo<__S>
        where
            __S: #source_bound,
            #(#error_bounds,)*
            #(#dsl_bounds,)*
            #(#super_bounds,)*
        {
            #(#impl_fns)*
        }
    })
}

/// The trait with `#[query]` attributes removed, emitted alongside an error.
pub fn strip_query_attrs(item: TokenStream) -> TokenStream {
    match syn::parse2::<ItemTrait>(item.clone()) {
        Ok(mut tr) => {
            for it in &mut tr.items {
                if let TraitItem::Fn(f) = it {
                    f.attrs.retain(|a| !a.path().is_ident("query"));
                }
            }
            quote!(#tr)
        }
        Err(_) => item,
    }
}
