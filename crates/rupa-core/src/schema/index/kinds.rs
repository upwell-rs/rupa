use super::{IndexKind, IndexMethod, IndexSupport, IndexableBy, JsonIndexKind};
use crate::value::ScalarColumn;

/// The portable ordered index: the default method of every dialect.
#[derive(Debug, Clone, Copy)]
pub struct BTree;

/// A hash index (`USING hash`): equality lookups over one column. PostgreSQL
/// only; InnoDB has no hash indexes and SQLite none at all.
#[derive(Debug, Clone, Copy)]
pub struct Hash;

/// A generalized inverted index (`USING gin`) over JSON columns of any type,
/// using PostgreSQL's default `jsonb_ops` class. PostgreSQL only. Text needs an
/// operator class (`gin_trgm_ops`, `to_tsvector`), so no text type is
/// accepted here.
#[derive(Debug, Clone, Copy)]
pub struct Gin;

/// A generalized search tree (`USING gist`). PostgreSQL only. No built-in
/// column type has a default GiST operator class, so no type is accepted
/// until a column type implements [`IndexableBy`] for it.
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

impl JsonIndexKind for BTree {}

impl JsonIndexKind for Gin {}
