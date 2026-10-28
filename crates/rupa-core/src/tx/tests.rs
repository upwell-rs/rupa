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
    p.clear();
    assert!(!p.is_pending());
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
