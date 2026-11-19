use rupa_build::{Migrations, VersionScheme};

fn main() -> Result<(), rupa_build::Error> {
    Migrations::new("primary").compile()?;
    Migrations::new("reporting")
        .dir("reporting")
        .scheme(VersionScheme::UtcTimestamp)
        .compile()
}
