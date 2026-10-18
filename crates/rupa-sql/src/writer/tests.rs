use super::expr::{valid_function_name, valid_operator};

#[test]
fn operator_validation() {
    for ok in [
        "ILIKE",
        "NOT ILIKE",
        "IS DISTINCT FROM",
        "@>",
        "->>",
        "?|",
        "<->",
    ] {
        assert!(valid_operator(ok), "{ok}");
    }
    for bad in [
        "", " ILIKE", "ILIKE ", "a;b", "--", "@>--", "/*", "'", "x1", "a  b",
    ] {
        assert!(!valid_operator(bad), "{bad}");
    }
}

#[test]
fn function_name_validation() {
    for ok in ["lower", "LOWER", "pg_catalog.lower", "_f1"] {
        assert!(valid_function_name(ok), "{ok}");
    }

    for bad in ["", "1f", "a.", ".a", "lower()", "a b", "a;drop"] {
        assert!(!valid_function_name(bad), "{bad}");
    }
}
