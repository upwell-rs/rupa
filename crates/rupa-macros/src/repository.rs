//! `#[repository]`: emits the trait as written (with `#[query]` attributes
//! removed) and one repository type, `{Trait}Repo<C>`, over any
//! [`Connection`](rupa_core::connection::Connection) `C`.
//!
//! The author decides everything visible:
//!
//! - **The connection.** `{Trait}Repo::new(c)` takes a pool, a connection or
//!   a transaction, or any `&`, `&mut`, `Box`, `Rc` or `Arc` of one, and runs
//!   every query on it. Methods take `&self`.
//! - **Sync or async, per method.** If any query method is `async`, the
//!   repository is async: it runs on every connection, sync ones included,
//!   and its plain `fn` methods block in place. Otherwise everything is
//!   synchronous and runs on sync connections.
//! - **Errors.** Methods return `Result<R, E>`; the implementation requires
//!   `E: From<rupa::DynError>`.
//!
//! Every repository is dyn-compatible by default: async methods return boxed
//! futures, so the trait works as `Arc<dyn Trait>` (which Upwell injects),
//! and `dyn Trait` can be built over any matching connection through
//! `DynRepository`. `#[repository(static_dispatch)]` opts into unboxed
//! `impl Future + Send` methods instead; such a trait is used through its
//! generated repository type, not as a trait object.
//!
//! With rupa's `upwell-database` feature, `#[repository(database = Primary)]`
//! registers the repository as an Upwell provider of `Arc<dyn Trait>` over
//! the declared database `Primary`, discovered at link time like a
//! component. `database` may repeat, once per database.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    FnArg, ItemTrait, ReturnType, Signature as FnSignature, TraitItem, TraitItemFn, Type,
    TypeParamBound,
};

use crate::query::{QueryAttr, Signature, called_functions, query_body};

/// One `#[query]` method of the trait.
struct Method {
    item: TraitItemFn,
    attr: QueryAttr,
    is_async: bool,
    /// `R` and `E` of `Result<R, E>`.
    ok: Type,
    err: Type,
}

/// Whether the repository is sync or async: any `async` method makes the
/// whole repository async.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sync,
    Async,
}

impl Mode {
    fn of(methods: &[Method]) -> Self {
        match methods.iter().any(|m| m.is_async) {
            true => Self::Async,
            false => Self::Sync,
        }
    }

    /// The bound on the repository's connection: a sync repository runs on
    /// sync connections, an async one on every connection it can borrow
    /// across `.await`.
    fn bound(self, ms: &TokenStream) -> TokenStream {
        match self {
            Self::Sync => quote!(#ms::Connection<Mode = #ms::Immediate>),
            Self::Async => quote!(#ms::Connection + ::core::marker::Sync),
        }
    }
}

/// `R` and `E` of a `Result<R, E>` type.
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

/// The options of `#[repository(..)]`.
#[derive(Default)]
struct Options {
    /// Native, unboxed futures instead of a dyn-compatible trait.
    static_dispatch: bool,
    /// The Upwell databases the repository is provided over.
    databases: Vec<syn::Path>,
}

impl Options {
    fn parse(args: TokenStream) -> syn::Result<Self> {
        let mut options = Self::default();
        let parser = syn::meta::parser(|m| {
            if m.path.is_ident("static_dispatch") {
                options.static_dispatch = true;

                return Ok(());
            }

            if m.path.is_ident("database") {
                options.databases.push(m.value()?.parse()?);

                return Ok(());
            }

            Err(m.error(
                "unknown `repository` option; expected `static_dispatch` or `database = ..`",
            ))
        });

        syn::parse::Parser::parse2(parser, args)?;

        if let Some(database) = options.databases.first() {
            if !cfg!(feature = "upwell") {
                return Err(syn::Error::new_spanned(
                    database,
                    "`database = ..` provides the repository in an Upwell application; enable rupa's `upwell-database` feature",
                ));
            }

            if options.static_dispatch {
                return Err(syn::Error::new_spanned(
                    database,
                    "`database = ..` provides `Arc<dyn Trait>`, which a `static_dispatch` repository is not",
                ));
            }
        }

        Ok(options)
    }
}

/// The `#[query]` method `f`, with its attribute removed from the trait.
/// `None` for a method with a default body and no `#[query]`.
fn take_method(f: &mut TraitItemFn) -> syn::Result<Option<Method>> {
    let Some(pos) = f.attrs.iter().position(|a| a.path().is_ident("query")) else {
        if f.default.is_none() {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a repository method needs `#[query(..)]`, or a default body",
            ));
        }

        return Ok(None);
    };
    let attr: QueryAttr = f.attrs.remove(pos).parse_args()?;

    if f.default.is_some() {
        return Err(syn::Error::new_spanned(
            &f.sig,
            "a `#[query]` method must not have a body",
        ));
    }

    match f.sig.inputs.first() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
        _ => {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a repository query method takes `&self`; the repository holds its connection",
            ));
        }
    }
    let ReturnType::Type(_, ret) = &f.sig.output else {
        return Err(syn::Error::new_spanned(
            &f.sig,
            "a repository method returns `Result<R, E>`",
        ));
    };
    let (ok, err) = result_parts(ret).ok_or_else(|| {
        syn::Error::new_spanned(
            ret,
            "a repository method returns `Result<R, E>`, with `E: From<executor error>` (e.g. `rupa::DynError`)",
        )
    })?;

    Ok(Some(Method {
        is_async: f.sig.asyncness.is_some(),
        ok,
        err,
        attr,
        item: f.clone(),
    }))
}

/// Every `#[query]` method of `tr`, stripping their attributes.
fn collect_methods(tr: &mut ItemTrait) -> syn::Result<Vec<Method>> {
    let mut methods = Vec::new();

    for it in &mut tr.items {
        let TraitItem::Fn(f) = it else { continue };

        methods.extend(take_method(f)?);
    }

    Ok(methods)
}

/// Rejects async methods without the `async` feature.
fn check_methods(methods: &[Method]) -> syn::Result<()> {
    if !cfg!(feature = "async")
        && let Some(asyncness) = methods.iter().find_map(|m| m.item.sig.asyncness)
    {
        return Err(syn::Error::new_spanned(
            asyncness,
            "async repository methods need the `async` feature of `rupa`",
        ));
    }

    Ok(())
}

/// Rewrites the trait's `async fn`s to return a future: boxed, so the trait
/// is dyn-compatible when requested, or unboxed `impl Future` by default.
fn rewrite_async_methods(
    tr: &mut ItemTrait,
    methods: &[Method],
    ms: &TokenStream,
    static_dispatch: bool,
) {
    for it in &mut tr.items {
        let TraitItem::Fn(f) = it else { continue };
        let Some(m) = methods
            .iter()
            .find(|m| m.is_async && m.item.sig.ident == f.sig.ident)
        else {
            continue;
        };
        let (ok, err) = (&m.ok, &m.err);

        f.sig.asyncness = None;
        f.sig.output = match static_dispatch {
            true => {
                syn::parse_quote!(-> impl #ms::Future<Output = ::core::result::Result<#ok, #err>> + #ms::Send)
            }
            false => {
                syn::parse_quote!(-> #ms::Pin<#ms::Box<dyn #ms::Future<Output = ::core::result::Result<#ok, #err>> + #ms::Send + '_>>)
            }
        };
    }
}

/// `items` without repeats, compared by their tokens.
fn distinct<T: ToTokens>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = Vec::new();
    let mut out = Vec::new();

    for item in items {
        let key = item.to_token_stream().to_string();

        if !seen.contains(&key) {
            seen.push(key);
            out.push(item);
        }
    }

    out
}

/// The where-clause of the implementation: every error type converts from
/// the executor's, and every statically gated DSL function is available on
/// its dialect. The DSL bounds sit on this impl, not on the trait, so
/// hand-written implementors are unaffected.
fn impl_bounds(
    tr: &ItemTrait,
    methods: &[Method],
    ms: &TokenStream,
    impl_type: &TokenStream,
) -> Vec<TokenStream> {
    let errors = distinct(methods.iter().map(|m| &m.err))
        .into_iter()
        .map(|e| quote!(#e: ::core::convert::From<#ms::DynError>));
    let functions = distinct(methods.iter().flat_map(|m| called_functions(&m.attr)))
        .into_iter()
        .map(|p| quote!(#p: #ms::DslAvailable<<__C as #ms::Connection>::Dialect>));
    let supertraits = tr.supertraits.iter().filter_map(|b| match b {
        TypeParamBound::Trait(t) => Some(quote!(#impl_type: #t)),
        _ => None,
    });

    errors.chain(functions).chain(supertraits).collect()
}

/// The signature `m` has in the emitted trait (its `async` rewritten).
fn emitted_signature<'a>(tr: &'a ItemTrait, m: &Method) -> &'a FnSignature {
    tr.items
        .iter()
        .find_map(|it| match it {
            TraitItem::Fn(f) if f.sig.ident == m.item.sig.ident => Some(&f.sig),
            _ => None,
        })
        .expect("method is in the trait")
}

/// The implementation of one method: build the query, then run it on the
/// repository's connection. A plain `fn` in an async repository blocks in
/// place.
fn method_impl(
    tr: &ItemTrait,
    m: &Method,
    mode: Mode,
    ms: &TokenStream,
    static_dispatch: bool,
) -> syn::Result<TokenStream> {
    let mut sig = Signature::from_sig(&m.item.sig)?;
    let err = &m.err;
    let emitted = emitted_signature(tr, m);
    let span = m.item.sig.span();
    let complete = quote!(#ms::complete(&self.0, __query));
    let error = quote!(|e| <#err as ::core::convert::From<#ms::DynError>>::from(#ms::ExecError::into_dyn_error(e)));

    sig.result = m.ok.clone();

    let query = query_body(ms, &m.attr, &sig)?;
    let body = match (mode, m.is_async, static_dispatch) {
        (Mode::Sync, _, _) => quote!(#ms::now(#complete).map_err(#error)),
        (Mode::Async, false, _) => quote!(#ms::block_in_place(#complete).map_err(#error)),
        (Mode::Async, true, true) => quote!(async move { #complete.await.map_err(#error) }),
        (Mode::Async, true, false) => {
            quote!(#ms::Box::pin(async move { #complete.await.map_err(#error) }))
        }
    };

    Ok(quote_spanned! {span=>
        #[allow(clippy::clone_on_copy)]
        #emitted {
            let __query = { #query };
            #body
        }
    })
}

/// Expands `#[repository]` into the trait and its repository type.
pub fn repository(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let ms = quote!(::rupa::__macro_support);
    let options = Options::parse(args)?;
    let static_dispatch = options.static_dispatch;
    let mut tr: ItemTrait = syn::parse2(item)?;

    if !tr.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &tr.generics,
            "generic repository traits are not supported yet",
        ));
    }

    let methods = collect_methods(&mut tr)?;

    if methods.is_empty() {
        return Ok(quote!(#tr));
    }

    check_methods(&methods)?;
    rewrite_async_methods(&mut tr, &methods, &ms, static_dispatch);

    let mode = Mode::of(&methods);
    let bound = mode.bound(&ms);
    let repo = format_ident!("{}Repo", tr.ident);
    let visibility = &tr.vis;
    let trait_ident = &tr.ident;
    let impl_type = quote!(#repo<__C>);
    let bounds = impl_bounds(&tr, &methods, &ms, &impl_type);
    let impl_fns = methods
        .iter()
        .map(|m| method_impl(&tr, m, mode, &ms, static_dispatch))
        .collect::<syn::Result<Vec<_>>>()?;
    let repo_doc = format!(
        "[`{trait_ident}`] over a connection `C`: a pool, a connection or a transaction, or a reference or smart pointer to one. Generated by `#[repository]`."
    );
    let dyn_repository = (!static_dispatch).then(|| {
        quote! {
            impl<__C> #ms::DynRepository<__C> for dyn #trait_ident
            where
                #repo<__C>: #trait_ident + 'static,
            {
                fn over(connection: __C) -> #ms::Arc<Self> {
                    #ms::Arc::new(#repo::new(connection))
                }
            }
        }
    });

    let providers = options
        .databases
        .iter()
        .map(|database| upwell_provider(trait_ident, database));

    Ok(quote! {
        #tr

        #[doc = #repo_doc]
        #[derive(Debug, Clone, Copy, Default)]
        #visibility struct #repo<__C>(__C);

        impl<__C> #repo<__C> {
            /// Runs this repository's queries on `connection`.
            #visibility const fn new(connection: __C) -> Self {
                Self(connection)
            }

            /// The connection this repository runs on.
            #visibility const fn connection(&self) -> &__C {
                &self.0
            }

            /// The connection, giving up the repository.
            #visibility fn into_connection(self) -> __C {
                self.0
            }
        }

        impl<__C> #trait_ident for #impl_type
        where
            __C: #bound,
            #(#bounds,)*
        {
            #(#impl_fns)*
        }

        #dyn_repository

        #(#providers)*
    })
}

/// Registers `dyn #trait_ident` as an Upwell provider over the declared
/// database `database`: built from the injected database component itself.
fn upwell_provider(trait_ident: &syn::Ident, database: &syn::Path) -> TokenStream {
    let up = quote!(::rupa::upwell::__private);
    let name = format!("dyn {trait_ident}");

    quote! {
        impl #up::ProviderOf<dyn #trait_ident> for ::rupa::upwell::databases::Database<#database> {
            fn upcast(this: #up::Arc<Self>) -> #up::Arc<dyn #trait_ident> {
                <dyn #trait_ident as #up::DynRepository<#up::Arc<Self>>>::over(this)
            }
        }

        const _: () = {
            #[#up::linkme::distributed_slice(#up::PROVIDERS)]
            #[linkme(crate = #up::linkme)]
            #[allow(unsafe_code)]
            static PROVIDER: #up::ProviderDescriptor =
                ::rupa::upwell::databases::provider::<#database, dyn #trait_ident>(#name);
        };
    }
}

/// The trait with `#[query]` attributes removed, emitted alongside an error.
pub fn strip_query_attrs(item: TokenStream) -> TokenStream {
    let Ok(mut tr) = syn::parse2::<ItemTrait>(item.clone()) else {
        return item;
    };

    for it in &mut tr.items {
        if let TraitItem::Fn(f) = it {
            f.attrs.retain(|a| !a.path().is_ident("query"));
        }
    }

    quote!(#tr)
}
