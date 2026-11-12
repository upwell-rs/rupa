//! The example's unit tests: the service on a fresh in-memory database.

use super::*;

#[test]
fn sign_up_normalizes_the_address() {
    let signups = memory_signups();

    let account = signups
        .sign_up(" Ada@Example.com ")
        .expect("sign-up succeeds");

    assert_eq!(account.email, "ada@example.com");
    assert_eq!(account.id, 1, "the backend generates ids");
}

#[test]
fn a_taken_address_is_refused() {
    let signups = memory_signups();

    signups.sign_up("ada@example.com").expect("first sign-up");

    assert_eq!(signups.sign_up("ADA@example.com"), Err(SignupError::Taken));
    assert_eq!(signups.accounts.active().expect("active runs").len(), 1);
}
