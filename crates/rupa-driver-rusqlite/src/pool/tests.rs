use super::*;
use std::path::PathBuf;

#[test]
fn urls() {
    assert_eq!(
        SqliteTarget::parse("sqlite::memory:"),
        Ok(SqliteTarget::Memory)
    );
    assert_eq!(
        SqliteTarget::parse("sqlite://:memory:"),
        Ok(SqliteTarget::Memory)
    );
    let file = |p: &str, mode| SqliteTarget::File {
        path: PathBuf::from(p),
        mode,
    };
    assert_eq!(
        SqliteTarget::parse("sqlite://data/app.db"),
        Ok(file("data/app.db", OpenMode::ReadWriteCreate))
    );
    assert_eq!(
        SqliteTarget::parse("SQLITE:app.db?mode=ro"),
        Ok(file("app.db", OpenMode::ReadOnly))
    );
    assert_eq!(
        SqliteTarget::parse("sqlite:///var/app.db?mode=rw"),
        Ok(file("/var/app.db", OpenMode::ReadWrite))
    );
    assert!(SqliteTarget::parse("sqlite://").is_err());
    assert!(SqliteTarget::parse("sqlite://x.db?cache=shared").is_err());
    assert!(SqliteTarget::parse("postgres://x").is_err());
}
