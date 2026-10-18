use super::*;

#[test]
fn keys_are_validated() {
    assert!(is_valid_key("tenant_id") && is_valid_key("_x1"));
    for bad in ["", "Tenant", "1x", "a.b", "a-b", "a b", "x'"] {
        assert!(!is_valid_key(bad), "{bad}");
    }

    let ctx = SecurityContext::new().set("tenant_id", "1").set("Bad", "2");
    assert_eq!(ctx.invalid_key(), Some("Bad"));
    assert_eq!(
        SecurityContext::new().namespace("my-app").invalid_key(),
        Some("my-app")
    );
}

#[test]
fn settings_are_namespaced() {
    let ctx = SecurityContext::new()
        .set("tenant_id", "42")
        .role("app_user");
    assert_eq!(
        ctx.settings().collect::<Vec<_>>(),
        [("app.tenant_id".to_string(), "42")]
    );
    assert_eq!(ctx.get_role(), Some("app_user"));
}
