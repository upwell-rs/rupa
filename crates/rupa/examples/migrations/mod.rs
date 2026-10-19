mod normalize;

use normalize::Normalize;
use rupa::migrate::Planned;

rupa::migrate::sql_migration! {
    /// Creates notes using an embedded SQL source.
    pub struct CreateNotes {
        version: "1",
        description: "create notes",
        up: "001.sql",
    }
}

rupa::migrate::migration_catalog! {
    /// The application's statically dispatched SQL/Rust catalog.
    pub enum Migrations, error MigrationsError {
        Schema(Planned<CreateNotes>),
        Rust(Normalize),
    }
}

pub fn entries() -> [Migrations; 2] {
    [
        Migrations::Rust(Normalize),
        Migrations::Schema(Planned(CreateNotes)),
    ]
}
