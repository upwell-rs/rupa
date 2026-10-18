//! The `#[query(..)]` attribute grammar.

use std::collections::BTreeSet;

use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, LitStr, Token, Type};

use crate::dsl::{self, Expr};

pub enum Count {
    Param(Ident),
    Lit(LitInt),
}

pub struct OrderKey {
    /// `Entity` of `Entity::field`, in joins.
    pub(crate) entity: Option<syn::Path>,
    pub(crate) field: Ident,
    pub(crate) desc: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
}

/// How a join finds its condition.
pub enum JoinOn {
    /// `on <expr>`
    Expr(Expr),
    /// `via Entity::relation`: a declared relation, by accessor.
    Via(syn::Path),
    /// Neither: the one declared relation to an earlier source.
    Inferred,
}

pub struct JoinSpec {
    pub(crate) kind: JoinKind,
    pub(crate) entity: syn::Path,
    /// `as name`: how fields of this source are written (`name::field`).
    pub(crate) alias: Option<Ident>,
    pub(crate) on: JoinOn,
}

/// A CRUD operation declared with a flag: `#[query(get)]`, `#[query(insert)]` ..
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Crud {
    Get,
    Insert,
    Update,
    Delete,
}

#[derive(Default)]
pub struct QueryAttr {
    pub crud: Option<(Crud, Span)>,
    pub returning: bool,
    pub filter: Option<Expr>,
    pub order_by: Vec<OrderKey>,
    pub limit: Option<Count>,
    pub offset: Option<Count>,
    pub sql: Option<LitStr>,
    pub entity: Option<Type>,
    /// `entity = T as name`: an alias for the first source of a join.
    pub entity_alias: Option<Ident>,
    pub joins: Vec<JoinSpec>,
    /// Span of the whole attribute, for errors about combinations.
    pub span: Option<Span>,
}

/// `as name`, if present.
pub(crate) fn parse_alias(input: ParseStream) -> syn::Result<Option<Ident>> {
    if input.peek(Token![as]) {
        input.parse::<Token![as]>()?;
        Ok(Some(input.parse()?))
    } else {
        Ok(None)
    }
}

pub(crate) fn parse_count(input: ParseStream) -> syn::Result<Count> {
    if input.peek(Token![$]) {
        input.parse::<Token![$]>()?;
        Ok(Count::Param(input.parse()?))
    } else {
        Ok(Count::Lit(input.parse().map_err(|e| {
            syn::Error::new(e.span(), "expected an integer literal or `$param`")
        })?))
    }
}

impl Parse for QueryAttr {
    /// A flag: a CRUD operation or `returning`.
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attr = QueryAttr {
            span: Some(input.span()),
            ..Default::default()
        };
        let mut seen = BTreeSet::new();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let name = key.to_string();
            if !input.peek(Token![=]) {
                let crud = match name.as_str() {
                    "get" => Some(Crud::Get),
                    "insert" => Some(Crud::Insert),
                    "update" => Some(Crud::Update),
                    "delete" => Some(Crud::Delete),
                    "returning" => {
                        attr.returning = true;
                        None
                    }
                    _ => {
                        return Err(syn::Error::new(
                            key.span(),
                            "unknown `query` flag; expected `get`, `insert`, `update`, `delete` or `returning` \
                             (or a `key = value`)",
                        ));
                    }
                };
                if let Some(c) = crud {
                    if attr.crud.is_some() {
                        return Err(syn::Error::new(
                            key.span(),
                            "only one of `get`, `insert`, `update`, `delete`",
                        ));
                    }

                    attr.crud = Some((c, key.span()));
                }

                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }

                continue;
            }

            input.parse::<Token![=]>()?;
            let repeatable = matches!(name.as_str(), "order_by" | "inner_join" | "left_join");
            if !repeatable && !seen.insert(name.clone()) {
                return Err(syn::Error::new(key.span(), format!("`{name}` given twice")));
            }

            match name.as_str() {
                "filter" => attr.filter = Some(dsl::parse_expr(input)?),
                "inner_join" | "left_join" => {
                    let kind = if name == "inner_join" {
                        JoinKind::Inner
                    } else {
                        JoinKind::Left
                    };
                    let entity = input.call(syn::Path::parse_mod_style)?;
                    let alias = parse_alias(input)?;
                    let on = if dsl::peek_keyword(input, "on") {
                        input.call(Ident::parse_any)?;
                        JoinOn::Expr(dsl::parse_expr(input)?)
                    } else if dsl::peek_keyword(input, "via") {
                        input.call(Ident::parse_any)?;
                        JoinOn::Via(input.call(syn::Path::parse_mod_style)?)
                    } else {
                        JoinOn::Inferred
                    };
                    attr.joins.push(JoinSpec {
                        kind,
                        entity,
                        alias,
                        on,
                    });
                }
                "order_by" => {
                    let mut path = input.call(syn::Path::parse_mod_style)?;
                    let field = path.segments.pop().expect("a path").into_value().ident;
                    path.segments.pop_punct();
                    let entity = (!path.segments.is_empty()).then_some(path);
                    let desc = if input.peek(Ident) && !input.peek2(Token![=]) {
                        let dir: Ident = input.parse()?;
                        match dir.to_string().as_str() {
                            "asc" => false,
                            "desc" => true,
                            _ => {
                                return Err(syn::Error::new(
                                    dir.span(),
                                    "expected `asc` or `desc`",
                                ));
                            }
                        }
                    } else {
                        false
                    };
                    attr.order_by.push(OrderKey {
                        entity,
                        field,
                        desc,
                    });
                }
                "limit" => attr.limit = Some(parse_count(input)?),
                "offset" => attr.offset = Some(parse_count(input)?),
                "sql" => attr.sql = Some(input.parse()?),
                "entity" => {
                    attr.entity = Some(input.parse()?);
                    attr.entity_alias = parse_alias(input)?;
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "unknown `query` key; expected `filter`, `inner_join`, `left_join`, `order_by`, `limit`, `offset`, `sql` or `entity`",
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        if attr.sql.is_some()
            && (attr.filter.is_some()
                || !attr.order_by.is_empty()
                || attr.limit.is_some()
                || attr.offset.is_some()
                || !attr.joins.is_empty())
        {
            return Err(syn::Error::new(
                attr.span.unwrap(),
                "`sql` is a complete query; it cannot be combined with `filter`, joins, `order_by`, `limit` or `offset`",
            ));
        }
        if let (Some(alias), true) = (&attr.entity_alias, attr.joins.is_empty()) {
            return Err(syn::Error::new(
                alias.span(),
                "an alias names a source of a join; this query has no joins",
            ));
        }

        if attr.crud.is_some() && !attr.joins.is_empty() {
            return Err(syn::Error::new(
                attr.span.unwrap(),
                "`get`, `insert`, `update` and `delete` work on one entity; joins are for selects",
            ));
        }

        validate_crud(&attr)?;
        Ok(attr)
    }
}

pub(crate) fn validate_crud(attr: &QueryAttr) -> syn::Result<()> {
    let span = attr.span.unwrap();
    let has_select_keys = !attr.order_by.is_empty()
        || attr.limit.is_some()
        || attr.offset.is_some()
        || attr.sql.is_some();
    match attr.crud {
        None if attr.returning => Err(syn::Error::new(span, "`returning` applies to `insert`")),
        None => Ok(()),
        Some((Crud::Delete, s)) => {
            if has_select_keys || attr.returning {
                Err(syn::Error::new(
                    s,
                    "`delete` takes `entity` and optionally `filter`; nothing else",
                ))
            } else if attr.filter.is_some() && attr.entity.is_none() {
                Err(syn::Error::new(
                    s,
                    "a filtered `delete` names its entity: `#[query(delete, entity = T, filter = ..)]`",
                ))
            } else {
                Ok(())
            }
        }
        Some((c, s)) => {
            let name = match c {
                Crud::Get => "get",
                Crud::Insert => "insert",
                Crud::Update => "update",
                Crud::Delete => unreachable!(),
            };
            if has_select_keys || attr.filter.is_some() {
                Err(syn::Error::new(
                    s,
                    format!(
                        "`{name}` cannot be combined with `filter`, `order_by`, `limit`, `offset` or `sql`"
                    ),
                ))
            } else if attr.returning && c != Crud::Insert {
                Err(syn::Error::new(s, "`returning` applies to `insert`"))
            } else if attr.entity.is_some() && c != Crud::Insert {
                Err(syn::Error::new(
                    s,
                    format!("`{name}` takes its entity from the signature"),
                ))
            } else {
                Ok(())
            }
        }
    }
}
