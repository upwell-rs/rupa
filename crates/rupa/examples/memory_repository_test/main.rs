//! Unit-testing code that uses a repository, on the in-memory backend: no
//! database server, file or Docker. The service depends on the repository
//! trait; production builds it over a real pool, tests over a `MemoryDb`.
//! The memory backend evaluates the query IR with Postgres semantics, so the
//! same `#[query]` declarations run unchanged.
//!
//! Run: `cargo run -p rupa --example memory_repository_test --features memory`
//! Test: `cargo test -p rupa --example memory_repository_test --features memory`

use std::sync::Arc;

use rupa::drivers::memory::MemoryDb;
use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable)]
#[entity(table = "accounts")]
struct Account {
    #[id(generated)]
    id: i64,
    email: String,
    active: bool,
}

/// A new account: everything but the generated id.
#[derive(Insertable)]
#[insertable(entity = Account)]
struct NewAccount {
    email: String,
    active: bool,
}

#[repository]
trait Accounts: Send + Sync {
    #[query(insert, returning)]
    fn create(&self, account: &NewAccount) -> Result<Account, DynError>;

    #[query(entity = Account, filter = email == $email)]
    fn email_taken(&self, email: &str) -> Result<bool, DynError>;

    #[query(filter = active == true, order_by = id)]
    fn active(&self) -> Result<Vec<Account>, DynError>;
}

/// Why a sign-up was refused.
#[derive(Debug, PartialEq)]
enum SignupError {
    /// The address is already registered.
    Taken,
    /// The repository failed.
    Database(String),
}

impl From<DynError> for SignupError {
    fn from(error: DynError) -> Self {
        Self::Database(error.to_string())
    }
}

/// The application code under test: it only knows the repository trait.
struct Signups {
    accounts: Arc<dyn Accounts>,
}

impl Signups {
    fn sign_up(&self, email: &str) -> Result<Account, SignupError> {
        let email = email.trim().to_lowercase();

        if self.accounts.email_taken(&email)? {
            return Err(SignupError::Taken);
        }

        let account = self.accounts.create(&NewAccount {
            email,
            active: true,
        })?;

        Ok(account)
    }
}

/// A fresh in-memory database with the accounts table: the test fixture.
fn memory_signups() -> Signups {
    let mut db = MemoryDb::new();

    db.register::<Account>();

    Signups {
        accounts: Arc::new(AccountsRepo::new(db)),
    }
}

fn main() {
    let signups = memory_signups();
    let ada = signups.sign_up(" Ada@Example.com ").expect("first sign-up");

    assert_eq!(ada.email, "ada@example.com");
    assert_eq!(signups.sign_up("ada@example.com"), Err(SignupError::Taken));
    assert_eq!(signups.accounts.active().expect("active runs"), [ada]);

    println!("the repository ran on the in-memory backend");
}

#[cfg(test)]
mod tests;
