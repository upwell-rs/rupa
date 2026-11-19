//! Generating builder expressions from the filter DSL.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Ident, Type};

use crate::dsl::{CmpOp, Expr};
use crate::model::column_path;

/// `tokens` with every span set to `span`, so errors in generated code point
/// at the user's token rather than at the whole attribute.
pub(crate) fn respan(tokens: &TokenStream, span: Span) -> TokenStream {
    tokens
        .clone()
        .into_iter()
        .map(|mut tt| {
            if let proc_macro2::TokenTree::Group(g) = &tt {
                let mut group = proc_macro2::Group::new(g.delimiter(), respan(&g.stream(), span));
                group.set_span(span);
                tt = proc_macro2::TokenTree::Group(group);
            } else {
                tt.set_span(span);
            }

            tt
        })
        .collect()
}

/// Generated tokens for one DSL node, by what it denotes.
pub(crate) enum Gen {
    /// An `Expr<bool>` (or other boolean expression).
    Cond(TokenStream),
    /// A typed column handle.
    Column(TokenStream),
    /// A JSON path builder.
    Json(TokenStream),
    /// A Rust value: parameter or literal.
    Value(TokenStream),
}

/// Where field names resolve.
pub enum Fields<'a> {
    /// `#[query]`: plain fields of one entity.
    Entity(&'a Type),
    /// `on!`: `Entity::field`, found among a join's sources (the closure
    /// argument named here) by entity type.
    Sources(Ident),
    /// A joined `#[query]`: the sources are known, so `alias::field` and an
    /// entity that is a source once resolve to a position. Other spellings of
    /// an entity fall back to the lookup by type.
    Known(Ident, Vec<KnownSource>),
}

/// A source of a joined `#[query]`, in join order.
pub struct KnownSource {
    /// The entity type.
    pub entity: TokenStream,
    /// `entity` as written, for matching `Entity::field` (spaces removed).
    pub key: String,
    pub alias: Option<Ident>,
}

pub(crate) fn path_key(tokens: &TokenStream) -> String {
    tokens.to_string().replace(' ', "")
}

pub struct Ctx<'a> {
    pub ms: &'a TokenStream,
    pub fields: Fields<'a>,
    /// The function's parameters, which `$name` must be one of; `None` in
    /// `on!`, where `$name` is any variable in scope.
    pub params: Option<&'a [(Ident, Type)]>,
    pub used: BTreeSet<String>,
}

impl Ctx<'_> {
    pub(crate) fn param(&mut self, p: &Ident) -> syn::Result<TokenStream> {
        let Some(params) = self.params else {
            let ms = respan(self.ms, p.span());
            return Ok(quote_spanned!(p.span()=> #ms::Clone::clone(&#p)));
        };
        if !params.iter().any(|(name, _)| name == p) {
            return Err(syn::Error::new(
                p.span(),
                format!("unknown parameter `${p}`: it must be an argument of this function"),
            ));
        }

        self.used.insert(p.to_string());
        let ms = respan(self.ms, p.span());
        Ok(quote_spanned!(p.span()=> #ms::Clone::clone(&#p)))
    }

    /// A plain field: a column of the query's entity.
    pub(crate) fn column(&self, field: &Ident) -> syn::Result<TokenStream> {
        match &self.fields {
            Fields::Entity(entity) => Ok(column_path(&quote!(#entity), field)),
            Fields::Sources(_) | Fields::Known(..) => Err(syn::Error::new(
                field.span(),
                format!("name the field's entity: `Entity::{field}`"),
            )),
        }
    }

    /// `Entity::field`: the column of `entity`'s source in a join.
    ///
    /// Every source of this entity has an alias.
    ///
    /// Spelled differently from its join: find it by type.
    pub(crate) fn qualified(&self, entity: &syn::Path, field: &Ident) -> syn::Result<TokenStream> {
        match &self.fields {
            Fields::Sources(sources) => {
                let ms = respan(self.ms, field.span());
                let column = column_path(&quote!(#entity), field);
                Ok(quote_spanned!(field.span()=> #ms::source_col(&#sources, #column)))
            }
            Fields::Known(sources, known) => {
                let key = path_key(&quote!(#entity));
                let by_alias = known
                    .iter()
                    .position(|s| s.alias.as_ref().is_some_and(|a| *a == key));
                let by_entity: Vec<usize> = known
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.alias.is_none() && s.key == key)
                    .map(|(n, _)| n)
                    .collect();
                let at = match (by_alias, by_entity.as_slice()) {
                    (Some(n), _) | (None, &[n]) => n,
                    (None, []) => {
                        let aliases: Vec<String> = known
                            .iter()
                            .filter(|s| s.key == key)
                            .filter_map(|s| s.alias.as_ref().map(|a| format!("`{a}::{field}`")))
                            .collect();
                        if !aliases.is_empty() {
                            return Err(syn::Error::new(
                                field.span(),
                                format!(
                                    "`{key}` is only joined under an alias; write {}",
                                    aliases.join(" or ")
                                ),
                            ));
                        }

                        let ms = respan(self.ms, field.span());
                        let column = column_path(&quote!(#entity), field);
                        return Ok(
                            quote_spanned!(field.span()=> #ms::source_col(&#sources, #column)),
                        );
                    }
                    (None, _) => {
                        return Err(syn::Error::new(
                            field.span(),
                            format!(
                                "`{key}` is a source more than once; give the joins aliases \
                                 (`left_join = {key} as name ..`) and write `name::{field}`"
                            ),
                        ));
                    }
                };
                let source = &known[at];
                let index = syn::Index::from(at);
                let column = column_path(&source.entity, field);
                Ok(quote_spanned!(field.span()=> #sources.#index.col(#column)))
            }
            Fields::Entity(_) => Err(syn::Error::new(
                field.span(),
                "`Entity::field` names a field of a joined entity; write just `field` here",
            )),
        }
    }

    /// A column, or a JSON path into it when `path` has keys.
    pub(crate) fn field(column: TokenStream, path: &[Ident]) -> Gen {
        if path.len() == 1 {
            Gen::Column(column)
        } else {
            let first = path[1].to_string();
            let rest = path[2..].iter().map(|k| k.to_string());
            Gen::Json(quote_spanned!(path[0].span()=> #column.path(#first)#(.key(#rest))*))
        }
    }

    /// Put the value on the right: `$x < a` is `a > $x`.
    ///
    /// Inherent `is_null` on JSON columns, `ExprOps::is_null` on scalars.
    ///
    /// Resolved by Rust path: `#[dsl::function]` generates a
    /// builder function of that name taking `impl IntoExpr<T>`.
    pub(crate) fn lower(&mut self, e: &Expr) -> syn::Result<Gen> {
        let ms = self.ms.clone();
        Ok(match e {
            Expr::Field(path) => Self::field(self.column(&path[0])?, path),
            Expr::Qualified(entity, path) => Self::field(self.qualified(entity, &path[0])?, path),
            Expr::Param(p) => Gen::Value(self.param(p)?),
            Expr::Lit(l) => Gen::Value(quote!(#l)),
            Expr::Not(inner, span) => {
                let c = self.cond(inner)?;
                Gen::Cond(quote_spanned!(*span=> ::core::ops::Not::not(#c)))
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                let (a, b) = (self.cond(a)?, self.cond(b)?);
                if matches!(e, Expr::And(..)) {
                    Gen::Cond(quote!(#a.and(#b)))
                } else {
                    Gen::Cond(quote!(#a.or(#b)))
                }
            }
            Expr::Cmp(op, a, b, span) => {
                let (a, b) = (self.lower(a)?, self.lower(b)?);
                let (op, lhs, rhs) = match (&a, &b) {
                    (Gen::Value(_), Gen::Value(_)) => {
                        return Err(syn::Error::new(
                            *span,
                            "compare a field with a value; both sides are values",
                        ));
                    }
                    (Gen::Value(_), _) => (op.mirrored(), b, a),
                    _ => (*op, a, b),
                };
                let rhs = match rhs {
                    Gen::Json(_) => {
                        return Err(syn::Error::new(
                            *span,
                            "a JSON path can only be compared with a value",
                        ));
                    }
                    Gen::Cond(t) | Gen::Column(t) | Gen::Value(t) => t,
                };
                let (method, binop) = match op {
                    CmpOp::Eq => (quote!(eq), quote!(Eq)),
                    CmpOp::Ne => (quote!(ne), quote!(Ne)),
                    CmpOp::Lt => (quote!(lt), quote!(Lt)),
                    CmpOp::Le => (quote!(le), quote!(Le)),
                    CmpOp::Gt => (quote!(gt), quote!(Gt)),
                    CmpOp::Ge => (quote!(ge), quote!(Ge)),
                };
                let ops = respan(&ms, *span);
                Gen::Cond(match lhs {
                    Gen::Json(path) => {
                        quote_spanned!(*span=> #path.compare(#ops::BinOp::#binop, #rhs))
                    }
                    Gen::Cond(t) | Gen::Column(t) => {
                        quote_spanned!(*span=> #ops::ExprOps::#method(#t, #rhs))
                    }
                    Gen::Value(_) => unreachable!("values were moved to the right"),
                })
            }
            Expr::In(a, b, span) => {
                let list = match self.lower(b)? {
                    Gen::Value(v) => v,
                    _ => {
                        return Err(syn::Error::new(
                            *span,
                            "`in` takes a parameter holding the values: `field in $values`",
                        ));
                    }
                };
                Gen::Cond(match self.lower(a)? {
                    Gen::Json(path) => quote_spanned!(*span=> #path.is_in(#list)),
                    Gen::Cond(t) | Gen::Column(t) => {
                        let ops = respan(&ms, *span);
                        quote_spanned!(*span=> #ops::ExprOps::in_(#t, #list))
                    }
                    Gen::Value(_) => {
                        return Err(syn::Error::new(
                            *span,
                            "the left side of `in` must be a field",
                        ));
                    }
                })
            }
            Expr::Like(a, b, span) => {
                let pattern = match self.lower(b)? {
                    Gen::Value(v) => v,
                    _ => {
                        return Err(syn::Error::new(
                            *span,
                            "`like` takes a pattern value: `field like $pattern`",
                        ));
                    }
                };
                Gen::Cond(match self.lower(a)? {
                    Gen::Json(path) => quote_spanned!(*span=> #path.like(#pattern)),
                    Gen::Cond(t) | Gen::Column(t) => {
                        let ops = respan(&ms, *span);
                        quote_spanned!(*span=> #ops::ExprOps::like(#t, #pattern))
                    }
                    Gen::Value(_) => {
                        return Err(syn::Error::new(
                            *span,
                            "the left side of `like` must be a field",
                        ));
                    }
                })
            }
            Expr::IsNull(a, span) => Gen::Cond(match self.lower(a)? {
                Gen::Json(path) => quote_spanned!(*span=> #path.is_null()),
                Gen::Cond(t) | Gen::Column(t) => quote_spanned!(*span=> {
                    #[allow(unused_imports)]
                    use #ms::ExprOps as _;
                    #t.is_null()
                }),
                Gen::Value(_) => {
                    return Err(syn::Error::new(*span, "`is_null` applies to a field"));
                }
            }),
            Expr::Call(path, args) => {
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(match self.lower(a)? {
                        Gen::Cond(t) | Gen::Column(t) | Gen::Json(t) | Gen::Value(t) => t,
                    });
                }

                Gen::Cond(quote_spanned!(path.segments[0].ident.span()=> #path(#(#lowered),*)))
            }
        })
    }

    /// A node used as a boolean condition.
    pub fn cond(&mut self, e: &Expr) -> syn::Result<TokenStream> {
        let span = e.span();
        let ms = self.ms.clone();
        match self.lower(e)? {
            Gen::Cond(t) | Gen::Column(t) => {
                Ok(quote_spanned!(span=> #ms::IntoCondition::into_condition(#t)))
            }
            Gen::Json(_) => Err(syn::Error::new(
                span,
                "a JSON path is not a condition; compare it with a value",
            )),
            Gen::Value(_) => Err(syn::Error::new(
                span,
                "a value is not a condition; compare a field with it",
            )),
        }
    }
}
