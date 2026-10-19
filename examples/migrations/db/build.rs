fn main() -> Result<(), rupa_build::Error> {
    rupa_build::Migrations::new("primary").compile()
}
