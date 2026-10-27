//! `#[repository]`: emits the trait as written (with `#[query]` attributes
//! removed), a repository key, a borrowed view, and an owned repository type.
//!
//! The author decides everything visible:
//!
//! - **Receivers.** `&self` acquires from a source; `&mut self` borrows a
//!   native executor directly, without a synchronization wrapper.
//! - **Sync or async, per method.** If any query method is `async`, the
//!   repository runs on an async executor, and its plain `fn` methods block
//!   in place. Otherwise everything is synchronous.
//! - **Errors.** Methods return `Result<R, E>`; the implementation requires
//!   `E: From<rupa::DynError>`.
//!
//! Every repository is dyn-compatible by default: async methods return boxed
//! futures, so the trait works as `Arc<dyn Trait>` (which Upwell injects),
//! and `dyn Trait` can be built over any matching source through
//! `DynRepository`. `#[repository(static_dispatch)]` opts into unboxed
//! `impl Future + Send` methods instead; such a trait is used through its
//! generated `Repo`/`View` types, not as a trait object.

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
    mut_receiver: bool,
    /// `R` and `E` of `Result<R, E>`.
    ok: Type,
    err: Type,
}

/// How the implementation reaches an executor: through a connection source
/// (`&self`), synchronously or asynchronously.
#[derive(Clone, Copy)]
enum Access {
    SharedSync,
    SharedAsync,
    ExclusiveSync,
    ExclusiveAsync,
}

impl Access {
    /// The access every method of the trait shares. Any `async` method makes
    /// the whole repository async.
    fn of(methods: &[Method]) -> Self {
        let is_async = methods.iter().any(|m| m.is_async);

        match (methods[0].mut_receiver, is_async) {
            (false, false) => Access::SharedSync,
            (false, true) => Access::SharedAsync,
            (true, false) => Access::ExclusiveSync,
            (true, true) => Access::ExclusiveAsync,
        }
    }

    fn is_async(self) -> bool {
        matches!(self, Access::SharedAsync | Access::ExclusiveAsync)
    }

    /// The bound on the repository source.
    fn bound(self, ms: &TokenStream) -> TokenStream {
        match self {
            Access::SharedSync => quote!(#ms::Acquire),
            Access::SharedAsync => quote!(#ms::AcquireAsync),
            Access::ExclusiveSync => quote!(#ms::Executor),
            Access::ExclusiveAsync => quote!(#ms::AsyncExecutor),
        }
    }

    fn mode(self, ms: &TokenStream) -> TokenStream {
        let name = match self {
            Self::SharedSync => "SyncMode",
            Self::SharedAsync => "AsyncMode",
            Self::ExclusiveSync => "ExclusiveSyncMode",
            Self::ExclusiveAsync => "ExclusiveAsyncMode",
        };

        let name = format_ident!("{name}");

        quote!(#ms::#name)
    }

    fn context(self, ms: &TokenStream) -> TokenStream {
        match self {
            Self::SharedSync | Self::SharedAsync => {
                quote!(#ms::Context<Source = __S> + ::core::marker::Send + ::core::marker::Sync)
            }
            Self::ExclusiveSync => quote!(#ms::ContextMut<Source = __S>),
            Self::ExclusiveAsync => quote!(#ms::ContextMut<Source = __S> + ::core::marker::Send),
        }
    }

    fn view(self, ms: &TokenStream) -> TokenStream {
        match self {
            Self::SharedSync | Self::SharedAsync => quote!(#ms::RepoView),
            Self::ExclusiveSync | Self::ExclusiveAsync => quote!(#ms::RepoMut),
        }
    }

    /// Runs a query through acquisition or an exclusive native executor.
    /// Async methods complete it unboxed, so their futures stay unboxed.
    fn execute(self, ms: &TokenStream) -> TokenStream {
        let wait = self.is_async().then(|| quote!(.await));
        let (acquire, connection) = match self {
            Self::SharedSync | Self::SharedAsync => {
                let source = self.bound(ms);

                (
                    quote! {
                        let mut __conn = #source::acquire(self.source()) #wait
                            .map_err(#ms::ExecError::into_dyn_error)?;
                    },
                    quote!(&mut __conn),
                )
            }
            Self::ExclusiveSync | Self::ExclusiveAsync => (quote!(), quote!(self.source_mut())),
        };

        let execute = if self.is_async() {
            quote!(#ms::complete_unboxed(#connection, __query).await)
        } else {
            quote!(__query.execute(#connection))
        };

        quote! {
            #acquire
            ::core::result::Result::Ok(#execute.map_err(#ms::ExecError::into_dyn_error)?)
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

/// Whether the trait uses native, unboxed futures.
fn parse_options(args: TokenStream) -> syn::Result<bool> {
    let mut static_dispatch = false;
    let parser = syn::meta::parser(|m| {
        if m.path.is_ident("static_dispatch") {
            static_dispatch = true;

            return Ok(());
        }

        Err(m.error("unknown `repository` option; expected `static_dispatch`"))
    });

    syn::parse::Parser::parse2(parser, args)?;

    Ok(static_dispatch)
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

    let mut_receiver = match f.sig.inputs.first() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() => r.mutability.is_some(),
        _ => {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a repository query method takes `&self` for a source or `&mut self` for a native executor",
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
            "a repository method returns `Result<R, E>`, with `E: From<executor error>` (e.g. `rupa::DynError`)",
        )
    })?;

    Ok(Some(Method {
        is_async: f.sig.asyncness.is_some(),
        mut_receiver,
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
    if let Some(method) = methods
        .iter()
        .find(|method| method.mut_receiver != methods[0].mut_receiver)
    {
        return Err(syn::Error::new_spanned(
            &method.item.sig,
            "all repository query methods take the same receiver: `&self` or `&mut self`",
        ));
    }

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
    source_bound: &TokenStream,
    impl_type: &TokenStream,
) -> Vec<TokenStream> {
    let errors = distinct(methods.iter().map(|m| &m.err))
        .into_iter()
        .map(|e| quote!(#e: ::core::convert::From<#ms::DynError>));
    let functions = distinct(methods.iter().flat_map(|m| called_functions(&m.attr)))
        .into_iter()
        .map(|p| quote!(#p: #ms::DslAvailable<<__S as #source_bound>::Dialect>));
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

/// The implementation of one method: build the query, then run it. A plain
/// `fn` in an async repository blocks in place.
fn method_impl(
    tr: &ItemTrait,
    m: &Method,
    access: Access,
    ms: &TokenStream,
    static_dispatch: bool,
) -> syn::Result<TokenStream> {
    let mut sig = Signature::from_sig(&m.item.sig)?;
    let (ok, err) = (&m.ok, &m.err);
    let result_ty = quote!(::core::result::Result<#ok, #err>);
    let execute = access.execute(ms);
    let emitted = emitted_signature(tr, m);
    let span = m.item.sig.span();

    sig.result = m.ok.clone();

    let query = query_body(ms, &m.attr, &sig)?;
    let body = match (access.is_async(), m.is_async, static_dispatch) {
        (false, _, _) => quote!({ let __out: #result_ty = (|| { #execute })(); __out }),
        (true, async_method, static_dispatch) => {
            let fut =
                quote!(async move { let __out: #result_ty = (async { #execute }).await; __out });

            match (async_method, static_dispatch) {
                (false, _) => quote!(#ms::block_in_place(#fut)),
                (true, true) => fut,
                (true, false) => quote!(#ms::Box::pin(#fut)),
            }
        }
    };

    Ok(quote_spanned! {span=>
        #[allow(clippy::clone_on_copy, clippy::needless_question_mark)]
        #emitted {
            let __query = { #query };
            #body
        }
    })
}

/// Expands `#[repository]` into the trait and its distinct bound types.
pub fn repository(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let ms = quote!(::rupa::__macro_support);
    let static_dispatch = parse_options(args)?;
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

    let access = Access::of(&methods);
    let source_bound = access.bound(&ms);
    let key = format_ident!("{}Key", tr.ident);
    let view = format_ident!("{}View", tr.ident);
    let owned = format_ident!("{}Repo", tr.ident);
    let visibility = &tr.vis;
    let mode = access.mode(&ms);
    let context = access.context(&ms);
    let view_base = access.view(&ms);
    let impl_type = quote!(#ms::Repository<__C, #key, #mode>);
    let bounds = impl_bounds(&tr, &methods, &ms, &source_bound, &impl_type);
    let impl_fns = methods
        .iter()
        .map(|m| method_impl(&tr, m, access, &ms, static_dispatch))
        .collect::<syn::Result<Vec<_>>>()?;
    let trait_ident = &tr.ident;
    let dyn_repository = (!static_dispatch).then(|| {
        quote! {
            impl<__S> #ms::DynRepository<__S> for dyn #trait_ident
            where
                #ms::Repo<__S, #key, #mode>: #trait_ident + 'static,
            {
                fn over(source: __S) -> ::std::sync::Arc<Self> {
                    ::std::sync::Arc::new(#ms::Repo::<__S, #key, #mode>::new(source))
                }
            }
        }
    });

    Ok(quote! {
        #tr

        #[doc = "Repository identity generated by `#[repository]`. "]
        #visibility struct #key;

        #[doc = "A borrowed view of this repository over one execution context. "]
        #visibility type #view<'__rupa, __S> = #view_base<'__rupa, #key, __S, #mode>;

        #[doc = "An owned instance of this repository. "]
        #visibility type #owned<__S> = #ms::Repo<__S, #key, #mode>;

        impl<__S, __C> #trait_ident for #impl_type
        where
            __C: #context,
            __S: #source_bound,
            #(#bounds,)*
        {
            #(#impl_fns)*
        }

        #dyn_repository
    })
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
