use super::*;

#[test]
fn pending_rollbacks_merge_into_the_outermost() {
    let p = PendingRollback::default();
    assert_eq!(p.statement(), None);
    p.defer(2);
    p.defer(1);
    p.defer(3);
    assert_eq!(p.statement(), Some(TxStatement::RollbackToSavepoint(1)));
    p.defer(0);
    assert_eq!(p.statement(), Some(TxStatement::Rollback));
    p.clear(TxStatement::Rollback);
    assert!(!p.is_pending());
}

#[test]
fn clearing_a_sent_rollback_keeps_one_deferred_meanwhile() {
    let p = PendingRollback::default();

    p.defer(2);
    let sent = p.statement().expect("a rollback to the savepoint is owed");
    p.defer(0);
    p.clear(sent);

    assert_eq!(p.statement(), Some(TxStatement::Rollback));

    p.clear(TxStatement::Rollback);

    assert!(!p.is_pending());
}

#[test]
fn a_slot_holds_one_top_level_transaction() {
    let slot = TxSlot::default();
    let claim = slot.claim(0).expect("a free slot");

    assert!(slot.is_open());
    assert!(matches!(slot.claim(0), Err(TxError::AlreadyOpen)));

    drop(slot.claim(1).expect("a savepoint needs no slot"));

    assert!(slot.is_open(), "a savepoint's claim frees nothing");

    drop(claim);

    assert!(!slot.is_open(), "an unkept claim frees the slot");

    slot.claim(0).expect("a freed slot").keep();

    assert!(slot.is_open(), "a kept claim holds the slot");

    slot.release();

    assert!(!slot.is_open());
}

#[test]
fn nested_transactions_refuse_options() {
    assert_eq!(
        open_statement::<TxError>(1, TxOptions::default().read_only()),
        Err(TxError::OptionsOnNested)
    );
    assert_eq!(
        open_statement::<TxError>(2, TxOptions::default()),
        Ok(TxStatement::Savepoint(2))
    );
}
