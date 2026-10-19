use super::*;

fn ty(source: &str) -> Type {
    syn::parse_str(source).expect("a type")
}

#[test]
fn scalar_types_are_recognized_by_name() {
    for source in [
        "i64",
        "String",
        "std::string::String",
        "Option<bool>",
        "Vec<u8>",
        "uuid::Uuid",
        "chrono::DateTime<chrono::Utc>",
        "Option<NaiveDateTime>",
    ] {
        assert!(known_scalar(&ty(source)), "{source}");
    }
}

#[test]
fn other_types_keep_the_compile_time_selection() {
    for source in [
        "Prefs",
        "Vec<String>",
        "Option<Prefs>",
        "DateTime<FixedOffset>",
        "u32",
        "<T as Trait>::Assoc",
        "String<X>",
    ] {
        assert!(!known_scalar(&ty(source)), "{source}");
    }
}
