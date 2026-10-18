//! Raw SQL with `$name` placeholders.

use std::collections::BTreeSet;

use super::Shape;
use super::Signature;
use super::check_all_used;
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::LitStr;

/// Splits raw SQL at `$name` placeholders, outside quotes and comments.
/// Returns literal fragments interleaved with parameter names.
/// Backslashes immediately before quotes are rejected: their interpretation
/// depends on the database and session settings, unknown during expansion.
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
                if c == b'\\' && bytes.get(i + 1) == Some(&q) {
                    return Err(
                        "backslash before a quote is dialect-dependent in `#[query(sql)]`; bind the value as a parameter or use doubled SQL quotes".into(),
                    );
                }

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

pub(crate) fn raw_body(
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
