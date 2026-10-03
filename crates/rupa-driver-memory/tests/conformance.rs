use rupa_conformance::{Blocking, Harness, Item, Note};
use rupa_driver_memory::{AsyncMemoryDb, MemoryDb};

struct AsyncMemory(AsyncMemoryDb);

impl Harness for AsyncMemory {
    type Exec = AsyncMemoryDb;

    async fn reset(&mut self) {
        self.0 = AsyncMemoryDb::new();
        self.0.register::<Item>().register::<Note>();
    }

    fn exec(&mut self) -> &mut AsyncMemoryDb {
        &mut self.0
    }
}

struct SyncMemory(Blocking<MemoryDb>);

impl Harness for SyncMemory {
    type Exec = Blocking<MemoryDb>;

    async fn reset(&mut self) {
        self.0 = Blocking(MemoryDb::new());
        self.0.0.register::<Item>().register::<Note>();
    }

    fn exec(&mut self) -> &mut Blocking<MemoryDb> {
        &mut self.0
    }
}

#[test]
fn conformance_async() {
    pollster::block_on(rupa_conformance::run(
        &mut AsyncMemory(AsyncMemoryDb::new()),
    ));
}

#[test]
fn conformance_sync() {
    let mut h = SyncMemory(Blocking(MemoryDb::new()));
    pollster::block_on(rupa_conformance::run(&mut h));
    pollster::block_on(rupa_conformance::run_transactions(&mut h));
}
