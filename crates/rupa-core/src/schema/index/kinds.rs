use super::{IndexKind, IndexMethod, IndexSupport, IndexableBy};
use crate::value::ScalarColumn;
use crate::varchar::VarChar;

/// The portable ordered index: the default method of every dialect.
#[derive(Debug, Clone, Copy)]
pub struct BTree;

/// A hash index (`USING hash`): equality lookups over one column. PostgreSQL
/// only; InnoDB has no hash indexes and SQLite none at all.
#[derive(Debug, Clone, Copy)]
pub struct Hash;

/// A generalized inverted index (`USING gin`), for full-text search and
/// containment over documents. PostgreSQL only.
#[derive(Debug, Clone, Copy)]
pub struct Gin;

/// A generalized search tree (`USING gist`), for trigram and similarity
/// search over text. PostgreSQL only.
#[derive(Debug, Clone, Copy)]
pub struct Gist;

impl IndexKind for BTree {
    const METHOD: IndexMethod = IndexMethod::new("btree")
        .postgres(IndexSupport::Default)
        .mysql(IndexSupport::Default)
        .sqlite(IndexSupport::Default)
        .supports_unique();
}

impl IndexKind for Hash {
    const METHOD: IndexMethod = IndexMethod::new("hash")
        .postgres(IndexSupport::Using("hash"))
        .mysql(IndexSupport::Unsupported("InnoDB has no hash indexes"))
        .sqlite(IndexSupport::Unsupported("SQLite has no hash indexes"))
        .single_column();
}

impl IndexKind for Gin {
    const METHOD: IndexMethod = IndexMethod::new("gin")
        .postgres(IndexSupport::Using("gin"))
        .mysql(IndexSupport::Unsupported("MySQL has no GIN indexes"))
        .sqlite(IndexSupport::Unsupported("SQLite has no GIN indexes"));
}

impl IndexKind for Gist {
    const METHOD: IndexMethod = IndexMethod::new("gist")
        .postgres(IndexSupport::Using("gist"))
        .mysql(IndexSupport::Unsupported("MySQL has no GiST indexes"))
        .sqlite(IndexSupport::Unsupported("SQLite has no GiST indexes"));
}

impl<T: ScalarColumn> IndexableBy<BTree> for T {}

impl<T: ScalarColumn> IndexableBy<Hash> for T {}

macro_rules! text_kinds {
    ($($kind:ty),*) => {$(
        impl IndexableBy<$kind> for String {}
        impl IndexableBy<$kind> for Option<String> {}
        impl<const N: usize> IndexableBy<$kind> for VarChar<N> {}
        impl<const N: usize> IndexableBy<$kind> for Option<VarChar<N>> {}
    )*};
}

text_kinds!(Gin, Gist);

impl IndexableBy<Gin> for serde_json::Value {}

impl IndexableBy<Gin> for Option<serde_json::Value> {}
