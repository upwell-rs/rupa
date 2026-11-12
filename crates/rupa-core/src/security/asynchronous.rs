//! Row-level security for async transactions.

use std::future::Future;

use super::{SecureTx, SecurityContext, check_native_rls};
use crate::dialect::{Supports, caps};
use crate::error::TxError;
use crate::exec::{AsyncExecutor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{AsyncTransaction, AsyncTransactional, TxOptions};

impl<T: AsyncExecutor> AsyncExecutor for SecureTx<T> {
    type Dialect = T::Dialect;
    type Error = T::Error;

    fn dialect(&self) -> &T::Dialect {
        self.tx.dialect()
    }

    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, T::Error>> + Send {
        self.tx.execute(statement, expect)
    }
}

impl<T: AsyncTransaction> AsyncTransaction for SecureTx<T> {
    fn commit(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        self.tx.commit()
    }

    fn rollback(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        self.tx.rollback()
    }
}

impl<T: AsyncTransactional> AsyncTransactional for SecureTx<T> {
    type Tx<'s>
        = T::Tx<'s>
    where
        Self: 's;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<T::Tx<'_>, T::Error>> + Send {
        self.tx.begin_with(options)
    }
}

/// Async counterpart of [`SecureTransactional`](crate::security::SecureTransactional).
pub trait AsyncSecureTransactional: AsyncTransactional {
    /// Opens a transaction with `context` applied; see
    /// [`SecureTransactional::begin_with_security`](crate::security::SecureTransactional::begin_with_security).
    fn begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<Self::Tx<'_>>, Self::Error>> + Send
    where
        Self::Dialect: Supports<caps::NativeRls>,
    {
        async move {
            let tx = self.begin_with(options).await?;
            apply_security(tx, context).await
        }
    }

    /// Async counterpart of
    /// [`SecureTransactional::try_begin_with_security`](crate::security::SecureTransactional::try_begin_with_security): checked at run time.
    fn try_begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<Self::Tx<'_>>, Self::Error>> + Send
    where
        Self::Error: From<TxError>,
    {
        async move {
            check_native_rls(self.dialect())?;
            let tx = self.begin_with(options).await?;
            apply_security(tx, context).await
        }
    }
}

impl<E: AsyncTransactional + ?Sized> AsyncSecureTransactional for E {}

/// Applies `context` inside the freshly opened `tx`. On failure `tx` is
/// dropped, which rolls it back.
pub async fn apply_security<T: AsyncExecutor>(
    tx: T,
    context: SecurityContext,
) -> Result<SecureTx<T>, T::Error> {
    if !context.is_empty() {
        tx.execute(&Statement::ApplySecurity(context.clone()), Expect::Affected)
            .await?;
    }

    Ok(SecureTx::from_parts(tx, context))
}
