use rupa_conformance::{Blocking, Harness, Item};
use rupa_driver_memory::MemoryDb;

struct AsyncMemory(MemoryDb);

impl Harness for AsyncMemory {
    type Exec = MemoryDb;

    async fn reset(&mut self) {
        self.0 = MemoryDb::new();
        self.0.register::<Item>();
    }

    fn exec(&mut self) -> &mut MemoryDb {
        &mut self.0
    }
}

struct SyncMemory(Blocking<MemoryDb>);

impl Harness for SyncMemory {
    type Exec = Blocking<MemoryDb>;

    async fn reset(&mut self) {
        self.0 = Blocking(MemoryDb::new());
        self.0.0.register::<Item>();
    }

    fn exec(&mut self) -> &mut Blocking<MemoryDb> {
        &mut self.0
    }
}

#[test]
fn conformance_async() {
    pollster::block_on(rupa_conformance::run(&mut AsyncMemory(MemoryDb::new())));
}

#[test]
fn conformance_sync() {
    pollster::block_on(rupa_conformance::run(&mut SyncMemory(Blocking(
        MemoryDb::new(),
    ))));
}
