use rupa::core::tx::TxOptions;
use rupa::{AsyncSecureTransactional, SecureTransactional, SecurityContext};

fn main() {
    // The memory backend has no native row-level security.
    let mut db = rupa_driver_memory::MemoryDb::new();
    let _ = db.begin_with_security(TxOptions::default(), SecurityContext::new());
    let mut db = rupa_driver_memory::AsyncMemoryDb::new();
    let _ = db.begin_with_security(TxOptions::default(), SecurityContext::new());
}
