use super::*;
use serde_json::json;

#[test]
fn like_matches_postgres_semantics() {
    let cases = [
        ("abc", "abc", true),
        ("abc", "a%", true),
        ("abc", "%c", true),
        ("abc", "a_c", true),
        ("abc", "a_", false),
        ("", "%", true),
        ("", "_", false),
        ("a%c", r"a\%c", true),
        ("abc", r"a\%c", false),
        ("mississippi", "%iss%ppi", true),
        ("ABC", "abc", false),
    ];
    for (s, p, expect) in cases {
        assert_eq!(like(s, p).unwrap(), expect, "{s:?} LIKE {p:?}");
    }

    assert!(like("a", "a\\").is_err());
}

#[test]
fn json_containment_matches_postgres() {
    let doc = json!({"a": 1, "tags": ["x", "y"], "n": {"k": true, "z": null}});
    assert!(json_contains(&doc, &json!({})));
    assert!(json_contains(&doc, &json!({"a": 1})));
    assert!(json_contains(&doc, &json!({"a": 1.0})));
    assert!(json_contains(&doc, &json!({"tags": ["y"]})));
    assert!(json_contains(&doc, &json!({"n": {"k": true}})));
    assert!(json_contains(&doc, &json!({"n": {"z": null}})));
    assert!(!json_contains(&doc, &json!({"a": 2})));
    assert!(!json_contains(&doc, &json!({"tags": ["q"]})));
    assert!(!json_contains(&doc, &json!({"missing": null})));
    assert!(json_contains(&json!([1, 2, [3]]), &json!([[3], 1])));
    assert!(json_contains(&json!(["a", "b"]), &json!("a")));
    assert!(!json_contains(&json!({"a": 1}), &json!([])));
}
