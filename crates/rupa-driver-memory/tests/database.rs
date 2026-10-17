//! The memory backend as a `Database`: one shared connection, through the
//! registry like any other driver.

use rupa::{DbConfig, Driver, Registry};
use rupa_conformance::{Item, Note, Review};
use rupa_driver_memory::MemoryDriver;

fn registry() -> Registry {
    Registry::new().with(MemoryDriver::new().setup(|db| {
        db.register::<Item>()
            .register::<Note>()
            .register::<Review>();
    }))
}

#[test]
fn pool_cases() {
    pollster::block_on(rupa_conformance::run_pool_cases(async || {
        registry()
            .connect(&DbConfig::new("memory:"))
            .await
            .expect("connect")
    }));
}

#[test]
fn options_for_another_driver_are_refused() {
    let config = DbConfig::new("memory:").driver(rupa::SqliteOptions::default());
    // The registry resolves by name, so it never hands this to the memory driver...
    let err = pollster::block_on(registry().connect(&config)).unwrap_err();
    assert!(
        err.to_string().contains("no driver named `sqlite`"),
        "{err}"
    );
    // ...which refuses it too when called directly.
    let err = pollster::block_on(MemoryDriver::new().connect(&config)).unwrap_err();
    assert!(err.to_string().contains("options for `sqlite`"), "{err}");
}
