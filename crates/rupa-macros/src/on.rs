//! `on!(..)`: a join condition in the `#[query]` expression grammar, with
//! fields written `Entity::field`.
//!
//! It expands to a closure over the join's tuple of source handles, the same
//! closure `inner_join_on`, `left_join_on`, `filter` and `order_by`-style
//! methods take. Each `Entity::field` resolves to that entity's source by
//! type (`rupa::join::Has`), so an entity that is a source twice (a self-join)
//! is a compile error: use the handles there. `$name` binds any variable in
//! scope as a parameter.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use syn::Ident;
use syn::parse::{ParseStream, Parser};

use crate::dsl::parse_expr;
use crate::query::{Ctx, Fields, condition_closure};

/// Mixed-site hygiene: the user's `$name` variables can't shadow it.
pub fn on(input: TokenStream, ms: TokenStream) -> syn::Result<TokenStream> {
    let expr = (|input: ParseStream| {
        let e = parse_expr(input)?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after the condition"));
        }

        Ok(e)
    })
    .parse2(input)?;
    let sources = Ident::new("__rupa_sources", Span::mixed_site());
    let mut ctx = Ctx {
        ms: &ms,
        fields: Fields::Sources(sources.clone()),
        params: None,
        used: BTreeSet::new(),
    };
    let cond = ctx.cond(&expr)?;
    Ok(condition_closure(&ms, &sources, &cond))
}
