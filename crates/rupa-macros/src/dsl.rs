//! The `#[query(filter = ..)]` expression grammar: a Pratt parser over `syn`
//! tokens.
//!
//! ```text
//! expr    := expr '||' expr | expr '&&' expr | '!' expr
//!          | operand ('==' | '!=' | '<' | '<=' | '>' | '>=') operand
//!          | operand 'in' operand | operand 'like' operand | operand 'is_null'
//!          | '(' expr ')' | operand
//! operand := field ('.' key)* | '$' param | literal | path '(' expr,* ')'
//! ```
//!
//! Precedence, loosest first: `||`, `&&`, `!`, comparisons / `in` / `like`
//! (non-associative), postfix `is_null`. Field paths with more than one
//! segment are JSON paths: `prefs.theme` is key `theme` of column `prefs`.

use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Peek};
use syn::{Ident, Lit, Path, Token, parenthesized, token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    /// The operator with its operands swapped: `a < b` == `b > a`.
    pub fn mirrored(self) -> Self {
        match self {
            CmpOp::Eq | CmpOp::Ne => self,
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
        }
    }
}

#[derive(Clone)]
pub enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>, Span),
    Cmp(CmpOp, Box<Expr>, Box<Expr>, Span),
    In(Box<Expr>, Box<Expr>, Span),
    Like(Box<Expr>, Box<Expr>, Span),
    IsNull(Box<Expr>, Span),
    /// `column` or `column.key.key`: the first segment is an entity field.
    Field(Vec<Ident>),
    Param(Ident),
    Lit(Lit),
    /// Lowered in milestone 6 (`#[dsl::function]`); parsed now for errors.
    Call(Path, #[allow(dead_code)] Vec<Expr>),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Or(a, _) | Expr::And(a, _) => a.span(),
            Expr::Not(_, s)
            | Expr::Cmp(.., s)
            | Expr::In(.., s)
            | Expr::Like(.., s)
            | Expr::IsNull(_, s) => *s,
            Expr::Field(path) => path[0].span(),
            Expr::Param(p) => p.span(),
            Expr::Lit(l) => l.span(),
            Expr::Call(p, _) => p.segments[0].ident.span(),
        }
    }
}

mod bp {
    pub const OR: u8 = 1;
    pub const AND: u8 = 2;
    pub const NOT: u8 = 3;
    pub const CMP: u8 = 4;
    pub const POSTFIX: u8 = 5;
}

/// `in` is a Rust keyword, so words are read with `Ident::parse_any`.
fn peek_keyword(input: ParseStream, word: &str) -> bool {
    input.fork().call(Ident::parse_any).is_ok_and(|i| i == word)
}

pub fn parse_expr(input: ParseStream) -> syn::Result<Expr> {
    parse_bp(input, 0)
}

fn parse_bp(input: ParseStream, min_bp: u8) -> syn::Result<Expr> {
    let mut lhs = parse_prefix(input)?;
    loop {
        if input.peek(Token![||]) && bp::OR > min_bp {
            input.parse::<Token![||]>()?;
            let rhs = parse_bp(input, bp::OR)?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        } else if input.peek(Token![&&]) && bp::AND > min_bp {
            input.parse::<Token![&&]>()?;
            let rhs = parse_bp(input, bp::AND)?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        } else if let Some(op) = peek_cmp(input) {
            if bp::CMP <= min_bp {
                break;
            }
            let span = input.span();
            consume_cmp(input, op)?;
            let rhs = parse_bp(input, bp::CMP)?;
            reject_chained_comparison(input)?;
            lhs = Expr::Cmp(op, Box::new(lhs), Box::new(rhs), span);
        } else if peek_keyword(input, "in") || peek_keyword(input, "like") {
            if bp::CMP <= min_bp {
                break;
            }
            let word = input.call(Ident::parse_any)?;
            let rhs = parse_bp(input, bp::CMP)?;
            reject_chained_comparison(input)?;
            lhs = if word == "in" {
                Expr::In(Box::new(lhs), Box::new(rhs), word.span())
            } else {
                Expr::Like(Box::new(lhs), Box::new(rhs), word.span())
            };
        } else if peek_keyword(input, "is_null") {
            if bp::POSTFIX <= min_bp {
                break;
            }
            let word = input.call(Ident::parse_any)?;
            lhs = Expr::IsNull(Box::new(lhs), word.span());
        } else {
            break;
        }
    }
    Ok(lhs)
}

fn reject_chained_comparison(input: ParseStream) -> syn::Result<()> {
    if peek_cmp(input).is_some() || peek_keyword(input, "in") || peek_keyword(input, "like") {
        Err(input.error("comparisons do not chain; add parentheses or `&&`"))
    } else {
        Ok(())
    }
}

fn peek_cmp(input: ParseStream) -> Option<CmpOp> {
    fn p<T: Peek>(input: ParseStream, t: T) -> bool {
        input.peek(t)
    }
    if p(input, Token![==]) {
        Some(CmpOp::Eq)
    } else if p(input, Token![!=]) {
        Some(CmpOp::Ne)
    } else if p(input, Token![<=]) {
        Some(CmpOp::Le)
    } else if p(input, Token![>=]) {
        Some(CmpOp::Ge)
    } else if p(input, Token![<]) {
        Some(CmpOp::Lt)
    } else if p(input, Token![>]) {
        Some(CmpOp::Gt)
    } else {
        None
    }
}

fn consume_cmp(input: ParseStream, op: CmpOp) -> syn::Result<()> {
    match op {
        CmpOp::Eq => input.parse::<Token![==]>().map(drop),
        CmpOp::Ne => input.parse::<Token![!=]>().map(drop),
        CmpOp::Le => input.parse::<Token![<=]>().map(drop),
        CmpOp::Ge => input.parse::<Token![>=]>().map(drop),
        CmpOp::Lt => input.parse::<Token![<]>().map(drop),
        CmpOp::Gt => input.parse::<Token![>]>().map(drop),
    }
}

fn parse_prefix(input: ParseStream) -> syn::Result<Expr> {
    if input.peek(Token![!]) && !input.peek(Token![!=]) {
        let bang: Token![!] = input.parse()?;
        let inner = parse_bp(input, bp::NOT)?;
        return Ok(Expr::Not(Box::new(inner), bang.span));
    }
    if input.peek(token::Paren) {
        let content;
        parenthesized!(content in input);
        let e = parse_expr(&content)?;
        if !content.is_empty() {
            return Err(content.error("unexpected tokens"));
        }
        return Ok(e);
    }
    if input.peek(Token![$]) {
        input.parse::<Token![$]>()?;
        let name: Ident = input
            .parse()
            .map_err(|e| syn::Error::new(e.span(), "expected a parameter name after `$`"))?;
        return Ok(Expr::Param(name));
    }
    if input.peek(Lit) {
        return Ok(Expr::Lit(input.parse()?));
    }
    if input.peek(Ident) || input.peek(Token![::]) {
        // A call (`path(args)`) or a field path (`a.b.c`).
        let fork = input.fork();
        let path: Path = fork.call(Path::parse_mod_style)?;
        if fork.peek(token::Paren) {
            input.call(Path::parse_mod_style)?;
            let content;
            parenthesized!(content in input);
            let args = content.parse_terminated(parse_expr, Token![,])?;
            return Ok(Expr::Call(path, args.into_iter().collect()));
        }
        let mut segments = vec![input.parse::<Ident>()?];
        if input.peek(Token![::]) {
            return Err(input.error("expected a field, or a function call `path(..)`"));
        }
        while input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            segments.push(
                input
                    .parse::<Ident>()
                    .map_err(|e| syn::Error::new(e.span(), "expected a JSON key"))?,
            );
        }
        for word in ["in", "like", "is_null"] {
            if segments[0] == word {
                return Err(syn::Error::new(
                    segments[0].span(),
                    format!("`{word}` needs an operand before it"),
                ));
            }
        }
        return Ok(Expr::Field(segments));
    }
    Err(input.error("expected a field, `$param`, literal, `!`, `(` or function call"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse::Parser;

    fn parse(s: &str) -> syn::Result<Expr> {
        let tokens: proc_macro2::TokenStream = s.parse().unwrap();
        (|input: ParseStream| {
            let e = parse_expr(input)?;
            if !input.is_empty() {
                return Err(input.error("trailing tokens"));
            }
            Ok(e)
        })
        .parse2(tokens)
    }

    /// A compact rendering of the tree, for precedence assertions.
    fn show(e: &Expr) -> String {
        match e {
            Expr::Or(a, b) => format!("(or {} {})", show(a), show(b)),
            Expr::And(a, b) => format!("(and {} {})", show(a), show(b)),
            Expr::Not(a, _) => format!("(not {})", show(a)),
            Expr::Cmp(op, a, b, _) => format!("({op:?} {} {})", show(a), show(b)),
            Expr::In(a, b, _) => format!("(in {} {})", show(a), show(b)),
            Expr::Like(a, b, _) => format!("(like {} {})", show(a), show(b)),
            Expr::IsNull(a, _) => format!("(is_null {})", show(a)),
            Expr::Field(p) => p
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("."),
            Expr::Param(p) => format!("${p}"),
            Expr::Lit(_) => "lit".into(),
            Expr::Call(p, args) => format!(
                "{}({})",
                quote::quote!(#p).to_string().replace(' ', ""),
                args.iter().map(show).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    fn p(s: &str) -> String {
        show(&parse(s).unwrap())
    }

    #[test]
    fn precedence() {
        assert_eq!(p("a == $x"), "(Eq a $x)");
        assert_eq!(
            p("a == $x && b != 3 || c"),
            "(or (and (Eq a $x) (Ne b lit)) c)"
        );
        assert_eq!(p("a || b && c"), "(or a (and b c))");
        assert_eq!(p("!a == $x && b"), "(and (not (Eq a $x)) b)");
        assert_eq!(p("!(a || b)"), "(not (or a b))");
        assert_eq!(p("(a || b) && c"), "(and (or a b) c)");
        assert_eq!(p("a <= $x && a >= $y"), "(and (Le a $x) (Ge a $y))");
        assert_eq!(p("a < $x || a > $y"), "(or (Lt a $x) (Gt a $y))");
    }

    #[test]
    fn operators_and_operands() {
        assert_eq!(p("prefs.theme == $theme"), "(Eq prefs.theme $theme)");
        assert_eq!(p("id in $ids"), "(in id $ids)");
        assert_eq!(
            p("email like $pattern && active"),
            "(and (like email $pattern) active)"
        );
        assert_eq!(
            p("nickname is_null || !nickname is_null"),
            "(or (is_null nickname) (not (is_null nickname)))"
        );
        assert_eq!(p("ilike(email, $p)"), "ilike(email, $p)");
        assert_eq!(
            p("rupa::dsl::ilike(email, $p) && a"),
            "(and rupa::dsl::ilike(email, $p) a)"
        );
        assert_eq!(p("a.b.c == 1"), "(Eq a.b.c lit)");
    }

    #[test]
    fn errors() {
        for bad in [
            "a == b == c",
            "a ==",
            "== a",
            "$",
            "a in",
            "a.",
            "a b",
            "a == $x &&",
            "in x",
            "like",
        ] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }
}
