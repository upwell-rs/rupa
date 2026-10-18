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
        Expr::Qualified(e, p) => format!(
            "{}::{}",
            quote::quote!(#e).to_string().replace(' ', ""),
            p.iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(".")
        ),
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
    assert_eq!(
        p("Order::user_id == User::id && Order::meta.k == $k"),
        "(and (Eq Order::user_id User::id) (Eq Order::meta.k $k))"
    );
    assert_eq!(
        p("app::model::User::id is_null"),
        "(is_null app::model::User::id)"
    );
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
