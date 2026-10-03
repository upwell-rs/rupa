//! `#[query(..)]`: turns a declared signature into a sans-IO `Query<R>` built
//! with the typed builder. Nothing is inferred from names: the attribute
//! gives the filter, ordering and paging; the signature gives the parameter
//! types and the result type `R`.
//!
//! ```text
//! #[query(filter = <expr>, order_by = field [asc|desc], .., limit = n|$p, offset = n|$p)]
//! #[query(sql = "SELECT .. WHERE email = $email")]
//! #[query(entity = Type, filter = ..)]        // when R does not name the entity (bool)
//! ```
//!
//! Result types: `Vec<T>`, `Option<T>`, `T` (rows of entity `T`), `bool`
//! (whether any row matches; needs `entity`). Raw SQL also takes `u64` and
//! `bool` as affected-row results.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{FnArg, Ident, LitInt, LitStr, Pat, ReturnType, Token, Type};

use crate::dsl::{self, CmpOp, Expr};
use crate::model::column_expr;

pub enum Count {
    Param(Ident),
    Lit(LitInt),
}

pub struct OrderKey {
    field: Ident,
    desc: bool,
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
    /// Span of the whole attribute, for errors about combinations.
    pub span: Option<Span>,
}

fn parse_count(input: ParseStream) -> syn::Result<Count> {
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
                // A flag: a CRUD operation or `returning`.
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
            if name != "order_by" && !seen.insert(name.clone()) {
                return Err(syn::Error::new(key.span(), format!("`{name}` given twice")));
            }
            match name.as_str() {
                "filter" => attr.filter = Some(dsl::parse_expr(input)?),
                "order_by" => {
                    let field: Ident = input.parse()?;
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
                    attr.order_by.push(OrderKey { field, desc });
                }
                "limit" => attr.limit = Some(parse_count(input)?),
                "offset" => attr.offset = Some(parse_count(input)?),
                "sql" => attr.sql = Some(input.parse()?),
                "entity" => attr.entity = Some(input.parse()?),
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "unknown `query` key; expected `filter`, `order_by`, `limit`, `offset`, `sql` or `entity`",
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
                || attr.offset.is_some())
        {
            return Err(syn::Error::new(
                attr.span.unwrap(),
                "`sql` is a complete query; it cannot be combined with `filter`, `order_by`, `limit` or `offset`",
            ));
        }
        validate_crud(&attr)?;
        Ok(attr)
    }
}

fn validate_crud(attr: &QueryAttr) -> syn::Result<()> {
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

/// `Some(T)` if `ty` is `&T`.
fn deref_type(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(r) => Some(&r.elem),
        _ => None,
    }
}

/// A single value parameter of a CRUD method, as a reference expression.
fn value_ref(ident: &Ident, ty: &Type) -> TokenStream {
    if deref_type(ty).is_some() {
        quote!(#ident)
    } else {
        quote!(&#ident)
    }
}

/// Whether `ty` holds several values: `&[V]`, `Vec<V>`, `&Vec<V>`.
fn is_many(ty: &Type) -> bool {
    let inner = deref_type(ty).unwrap_or(ty);
    matches!(inner, Type::Slice(_)) || single_generic(inner, "Vec").is_some()
}

fn crud_body(ms: &TokenStream, attr: &QueryAttr, sig: &Signature) -> syn::Result<TokenStream> {
    let (crud, span) = attr.crud.expect("checked by caller");
    let result = &sig.result;
    let shape = shape_of(result);

    if let (Crud::Delete, Some(filter)) = (crud, &attr.filter) {
        let entity = attr.entity.as_ref().expect("validated");
        if !matches!(shape, Shape::U64 | Shape::Bool) {
            return Err(syn::Error::new(
                sig.result_span,
                "a `delete` returns `u64` (rows deleted) or `bool` (any deleted)",
            ));
        }
        let mut ctx = Ctx {
            ms,
            entity,
            params: &sig.params,
            used: BTreeSet::new(),
        };
        let c = ctx.cond(filter)?;
        check_all_used(&ctx.used, sig)?;
        return Ok(
            quote_spanned!(sig.result_span=> #ms::delete::<#entity>().filter(#c).build::<#result>()),
        );
    }

    let [(param, param_ty)] = sig.params.as_slice() else {
        let what = match crud {
            Crud::Get => "the id",
            Crud::Insert => "the value(s) to insert",
            Crud::Update => "the value describing the update",
            Crud::Delete if attr.entity.is_some() => "the id",
            Crud::Delete => "the value to delete (or name `entity` and take the id)",
        };
        return Err(syn::Error::new(
            span,
            format!("this method takes exactly one parameter: {what}"),
        ));
    };
    let value = value_ref(param, param_ty);

    Ok(match crud {
        Crud::Get => {
            let Some(entity) = single_generic(result, "Option") else {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `get` returns `Option<T>`",
                ));
            };
            quote_spanned!(span=> #ms::get::<#entity>(#value))
        }
        Crud::Insert => {
            let rows = if is_many(param_ty) {
                quote_spanned!(span=> #ms::insert::<_>().values(#value))
            } else {
                quote_spanned!(span=> #ms::insert::<_>().value(#value))
            };
            if attr.returning {
                if matches!(shape, Shape::U64 | Shape::Bool) {
                    return Err(syn::Error::new(
                        sig.result_span,
                        "`insert, returning` returns the stored row(s): `T` or `Vec<T>`",
                    ));
                }
                quote_spanned!(sig.result_span=> #rows.returning::<#result>())
            } else {
                if !matches!(shape, Shape::U64 | Shape::Bool) {
                    return Err(syn::Error::new(
                        sig.result_span,
                        "an `insert` returns `u64` or `bool`; add `returning` to get the stored row(s)",
                    ));
                }
                let entity = attr
                    .entity
                    .as_ref()
                    .map(|e| quote!(#e))
                    .unwrap_or(quote!(_));
                let rows = if is_many(param_ty) {
                    quote_spanned!(span=> #ms::insert::<#entity>().values(#value))
                } else {
                    quote_spanned!(span=> #ms::insert::<#entity>().value(#value))
                };
                quote_spanned!(sig.result_span=> #rows.build::<#result>())
            }
        }
        Crud::Update => {
            if !matches!(shape, Shape::U64 | Shape::Bool) {
                return Err(syn::Error::new(
                    sig.result_span,
                    "an `update` returns `u64` (rows updated) or `bool` (any updated)",
                ));
            }
            quote_spanned!(span=> #ms::update::<_>().one(#value).build::<#result>())
        }
        Crud::Delete => {
            if !matches!(shape, Shape::U64 | Shape::Bool) {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `delete` returns `u64` (rows deleted) or `bool` (any deleted)",
                ));
            }
            match &attr.entity {
                Some(entity) => {
                    quote_spanned!(span=> #ms::delete::<#entity>().by_id(#value).build::<#result>())
                }
                None => quote_spanned!(span=> #ms::delete::<_>().one(#value).build::<#result>()),
            }
        }
    })
}

/// What the declared result type asks for.
enum Shape {
    /// `Vec<T>`, `Option<T>`, `T`: rows of entity `T`.
    Rows {
        entity: Box<Type>,
    },
    Bool,
    U64,
}

fn single_generic<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
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

fn is_ident(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

fn shape_of(ret: &Type) -> Shape {
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

/// `tokens` with every span set to `span`, so errors in generated code point
/// at the user's token rather than at the whole attribute.
fn respan(tokens: &TokenStream, span: Span) -> TokenStream {
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
enum Gen {
    /// An `Expr<bool>` (or other boolean expression).
    Cond(TokenStream),
    /// A typed column handle.
    Column(TokenStream),
    /// A JSON path builder.
    Json(TokenStream),
    /// A Rust value: parameter or literal.
    Value(TokenStream),
}

struct Ctx<'a> {
    ms: &'a TokenStream,
    entity: &'a Type,
    params: &'a [(Ident, Type)],
    used: BTreeSet<String>,
}

impl Ctx<'_> {
    fn param(&mut self, p: &Ident) -> syn::Result<TokenStream> {
        if !self.params.iter().any(|(name, _)| name == p) {
            return Err(syn::Error::new(
                p.span(),
                format!("unknown parameter `${p}`: it must be an argument of this function"),
            ));
        }
        self.used.insert(p.to_string());
        let ms = respan(self.ms, p.span());
        Ok(quote_spanned!(p.span()=> #ms::Clone::clone(&#p)))
    }

    fn column(&self, field: &Ident) -> TokenStream {
        let entity = self.entity;
        column_expr(self.ms, &quote!(#entity), field, field.span())
    }

    fn lower(&mut self, e: &Expr) -> syn::Result<Gen> {
        let ms = self.ms.clone();
        Ok(match e {
            Expr::Field(path) => {
                let column = self.column(&path[0]);
                if path.len() == 1 {
                    Gen::Column(column)
                } else {
                    let first = path[1].to_string();
                    let rest = path[2..].iter().map(|k| k.to_string());
                    Gen::Json(quote_spanned!(path[0].span()=> #column.path(#first)#(.key(#rest))*))
                }
            }
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
                // Put the value on the right: `$x < a` is `a > $x`.
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
                // Inherent `is_null` on JSON columns, `ExprOps::is_null` on scalars.
                Gen::Cond(t) | Gen::Column(t) => quote_spanned!(*span=> {
                    #[allow(unused_imports)]
                    use #ms::ExprOps as _;
                    #t.is_null()
                }),
                Gen::Value(_) => {
                    return Err(syn::Error::new(*span, "`is_null` applies to a field"));
                }
            }),
            Expr::Call(path, _) => {
                return Err(syn::Error::new_spanned(
                    path,
                    "DSL function calls are not supported yet (milestone 6: `#[dsl::function]`)",
                ));
            }
        })
    }

    /// A node used as a boolean condition.
    fn cond(&mut self, e: &Expr) -> syn::Result<TokenStream> {
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

fn count(ctx: &mut Ctx, c: &Count) -> syn::Result<TokenStream> {
    match c {
        Count::Param(p) => ctx.param(p),
        Count::Lit(l) => Ok(quote!(#l)),
    }
}

/// The expression building `Query<R>` for a declared query.
pub fn query_body(ms: &TokenStream, attr: &QueryAttr, sig: &Signature) -> syn::Result<TokenStream> {
    if attr.crud.is_some() {
        return crud_body(ms, attr, sig);
    }
    let result = &sig.result;
    let shape = shape_of(result);

    let body = if let Some(sql) = &attr.sql {
        raw_body(ms, sql, sig, &shape)?
    } else {
        let entity = match (&attr.entity, &shape) {
            (Some(e), _) => e.clone(),
            (None, Shape::Rows { entity }) => (**entity).clone(),
            (None, Shape::Bool) => {
                return Err(syn::Error::new(
                    sig.result_span,
                    "a `bool` query checks whether rows exist; name the entity with `entity = Type`",
                ));
            }
            (None, Shape::U64) => {
                return Err(syn::Error::new(
                    sig.result_span,
                    "`u64` (affected rows) is for raw `sql` statements; filter queries return rows or `bool`",
                ));
            }
        };
        let mut ctx = Ctx {
            ms,
            entity: &entity,
            params: &sig.params,
            used: BTreeSet::new(),
        };
        let mut b = quote!(#ms::select::<#entity>());
        if let Some(f) = &attr.filter {
            let c = ctx.cond(f)?;
            b = quote!(#b.filter(#c));
        }
        for key in &attr.order_by {
            let column = ctx.column(&key.field);
            let dir = if key.desc { quote!(desc) } else { quote!(asc) };
            b = quote!(#b.order_by({
                #[allow(unused_imports)]
                use #ms::ExprOps as _;
                #column.#dir()
            }));
        }
        if let Some(l) = &attr.limit {
            let l = count(&mut ctx, l)?;
            b = quote!(#b.limit(#l));
        }
        if let Some(o) = &attr.offset {
            let o = count(&mut ctx, o)?;
            b = quote!(#b.offset(#o));
        }
        check_all_used(&ctx.used, sig)?;
        match shape {
            Shape::Bool => quote!(#b.exists()),
            _ => quote_spanned!(sig.result_span=> #b.build::<#result>()),
        }
    };
    Ok(body)
}

fn check_all_used(used: &BTreeSet<String>, sig: &Signature) -> syn::Result<()> {
    match sig
        .params
        .iter()
        .find(|(name, _)| !used.contains(&name.to_string()))
    {
        Some((name, _)) => Err(syn::Error::new(
            name.span(),
            format!("parameter `{name}` is not used by the query; reference it as `${name}`"),
        )),
        None => Ok(()),
    }
}

/// Splits raw SQL at `$name` placeholders, outside quotes and comments.
/// Returns literal fragments interleaved with parameter names.
pub fn split_sql(sql: &str) -> Result<Vec<SqlPart>, String> {
    let bytes = sql.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut state = None::<u8>; // b'\'' or b'"' while quoted
    while i < bytes.len() {
        let c = bytes[i];
        match state {
            Some(q) => {
                if c == q {
                    state = None;
                }
                i += 1;
            }
            None if c == b'\'' || c == b'"' => {
                state = Some(c);
                i += 1;
            }
            None if c == b'-' && bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            None if c == b'/' && bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
            }
            None if c == b'$' => {
                let rest = &sql[i + 1..];
                let name_len = rest
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(rest.len());
                let name = &rest[..name_len];
                if name.is_empty() {
                    return Err("`$` must start a named parameter (`$name`)".into());
                }
                if name.as_bytes()[0].is_ascii_digit() {
                    return Err(format!(
                        "positional placeholder `${name}`: use named parameters (`$email`), which are rendered per dialect"
                    ));
                }
                if rest[name_len..].starts_with('$') {
                    return Err("dollar-quoted strings are not supported in `#[query(sql)]`".into());
                }
                if start < i {
                    parts.push(SqlPart::Text(sql[start..i].to_owned()));
                }
                parts.push(SqlPart::Param(name.to_owned()));
                i += 1 + name_len;
                start = i;
            }
            None => i += 1,
        }
    }
    if state.is_some() {
        return Err("unterminated quote in SQL".into());
    }
    if start < bytes.len() {
        parts.push(SqlPart::Text(sql[start..].to_owned()));
    }
    Ok(parts)
}

#[derive(Debug, PartialEq)]
pub enum SqlPart {
    Text(String),
    Param(String),
}

fn raw_body(
    ms: &TokenStream,
    sql: &LitStr,
    sig: &Signature,
    shape: &Shape,
) -> syn::Result<TokenStream> {
    let parts = split_sql(&sql.value()).map_err(|m| syn::Error::new(sql.span(), m))?;
    let mut ctx_used = BTreeSet::new();
    let mut b = quote!(#ms::raw());
    for part in parts {
        match part {
            SqlPart::Text(t) => b = quote!(#b.sql(#t)),
            SqlPart::Param(name) => {
                let Some((ident, _)) = sig.params.iter().find(|(p, _)| *p == name) else {
                    return Err(syn::Error::new(
                        sql.span(),
                        format!("unknown parameter `${name}` in SQL"),
                    ));
                };
                ctx_used.insert(name);
                b = quote!(#b.bind_param(#ms::Clone::clone(&#ident)));
            }
        }
    }
    check_all_used(&ctx_used, sig)?;
    let result = &sig.result;
    Ok(match shape {
        Shape::Rows { .. } => quote_spanned!(sig.result_span=> #b.rows::<#result>()),
        Shape::Bool | Shape::U64 => quote_spanned!(sig.result_span=> #b.execute::<#result>()),
    })
}

/// `#[query]` on a free function: `fn f(args) -> R;` becomes
/// `fn f(args) -> Query<R> { .. }`.
pub fn query_fn(attr: TokenStream, item: TokenStream, ms: TokenStream) -> syn::Result<TokenStream> {
    let attr: QueryAttr = syn::parse2(attr)?;
    let item: syn::ForeignItemFn = syn::parse2(item).map_err(|e| {
        syn::Error::new(
            e.span(),
            "`#[query]` applies to a function declaration without a body: `fn name(args) -> R;`",
        )
    })?;
    let sig_info = Signature::from_sig(&item.sig)?;
    let body = query_body(&ms, &attr, &sig_info)?;
    let syn::ForeignItemFn {
        attrs,
        vis,
        mut sig,
        ..
    } = item;
    let result = &sig_info.result;
    sig.output = syn::parse_quote!(-> #ms::Query<#result>);
    Ok(quote! {
        #(#attrs)*
        // Parameters are cloned so each may be used more than once.
        #[allow(clippy::clone_on_copy)]
        #vis #sig {
            #body
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> SqlPart {
        SqlPart::Text(s.into())
    }
    fn param(s: &str) -> SqlPart {
        SqlPart::Param(s.into())
    }

    #[test]
    fn splits_named_parameters() {
        assert_eq!(
            split_sql("SELECT * FROM t WHERE a = $a AND b > $b_2").unwrap(),
            [
                text("SELECT * FROM t WHERE a = "),
                param("a"),
                text(" AND b > "),
                param("b_2")
            ]
        );
        assert_eq!(split_sql("$x").unwrap(), [param("x")]);
    }

    #[test]
    fn ignores_dollars_in_quotes_and_comments() {
        let sql = "SELECT '$no', \"$col\" -- $c\n /* $d */ FROM t WHERE x = $yes";
        let parts = split_sql(sql).unwrap();
        assert_eq!(
            parts
                .iter()
                .filter(|p| matches!(p, SqlPart::Param(_)))
                .count(),
            1
        );
        assert_eq!(parts.last(), Some(&param("yes")));
    }

    #[test]
    fn rejects_unsupported_placeholders() {
        assert!(
            split_sql("WHERE a = $1")
                .unwrap_err()
                .contains("positional")
        );
        assert!(
            split_sql("SELECT $tag$x$tag$")
                .unwrap_err()
                .contains("dollar-quoted")
        );
        assert!(split_sql("WHERE a = $").is_err());
        assert!(split_sql("WHERE a = 'open").is_err());
    }
}
