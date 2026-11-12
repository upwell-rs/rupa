use super::*;

#[test]
fn a_url_password_is_masked_as_written_decoded_and_escaped() {
    let url = "postgres://app:p%40ss\"w0rd@db.internal:5432/app?sslmode=require";
    let redactor = Redactor::new().url(url);
    let text = format!("connect {url} failed: password p@ss\"w0rd rejected; json \"p@ss\\\"w0rd\"");
    let redacted = redactor.redact(&text);

    assert!(!redacted.contains("w0rd"), "{redacted}");
    assert!(
        redacted.contains("postgres://app:***@db.internal:5432/app"),
        "{redacted}"
    );
}

#[test]
fn password_parameters_are_masked() {
    for url in [
        "mysql://db/app?user=app&password=hunter2&ssl-mode=required",
        "host=db user=app password=hunter2 dbname=app",
        "host=db user=app password='hunter2' dbname=app",
        "postgres://db/app?sslpassword=hunter2",
    ] {
        let redacted = redact_url(url);

        assert!(!redacted.contains("hunter2"), "{redacted}");
    }
}

#[test]
fn urls_without_passwords_are_kept() {
    for url in ["sqlite://app.db", "app.db", "postgres://app@db/app"] {
        assert_eq!(redact_url(url), url);
    }
}

#[test]
fn explicit_secrets_are_masked_and_empty_ones_ignored() {
    let redactor = Redactor::new().secret("token-123").secret("");

    assert_eq!(redactor.redact("bearer token-123"), "bearer ***");
    assert_eq!(redactor.redact("nothing"), "nothing");
}

#[test]
fn quoted_keyword_passwords_are_masked_whole() {
    for url in [
        "host=db password='two words' dbname=app",
        "host=db password='it\\'s two' dbname=app",
    ] {
        let redacted = redact_url(url);

        assert!(!redacted.contains("two"), "{redacted}");
        assert!(redacted.ends_with(" dbname=app"), "{redacted}");
    }
}
