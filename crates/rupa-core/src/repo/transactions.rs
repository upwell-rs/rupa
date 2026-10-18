//! Repository transaction opening delegates to native sources and executors.

use super::{Acquire, Context, ContextMut, ExclusiveSyncMode, Repository, SyncMode};
use crate::dialect::{Supports, caps};
use crate::error::TxError;
use crate::exec::Executor;
use crate::pool::{OwnedTx, Pool};
use crate::security::{SecureTx, SecurityContext, check_native_rls};
use crate::tx::{Transaction, Transactional, TxOptions};

#[cfg(feature = "async")]
use super::{AcquireAsync, AsyncMode, AsyncTransactionSource, ExclusiveAsyncMode};
#[cfg(feature = "async")]
use crate::exec::AsyncExecutor;
#[cfg(feature = "async")]
use crate::tx::AsyncTransactional;

/// A sync source that opens owned native transactions.
pub trait TransactionSource: Acquire {
    /// The native transaction returned by the source.
    type Tx: Transaction<Dialect = Self::Dialect, Error = Self::Error>;

    /// The source's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Opens a transaction with the requested options.
    fn open(&self, options: TxOptions) -> Result<Self::Tx, Self::Error>;
}

impl<P: Pool> TransactionSource for P {
    type Tx = OwnedTx<P::Conn>;

    fn dialect(&self) -> &Self::Dialect {
        Pool::dialect(self)
    }

    fn open(&self, options: TxOptions) -> Result<Self::Tx, Self::Error> {
        Pool::begin_with(self, options)
    }
}

macro_rules! transaction_methods {
    ($mode:ty, $context:path, $source:path, $executor:path, $dialect:path,
     $open:ident, $access:ident, $tx:ty, $apply:path,
     [$($async:tt)*], [$($await:tt)*], [$($mutable:tt)*]) => {
        impl<C: $context, K> Repository<C, K, $mode>
        where
            C::Source: $source,
        {
            /// Opens a native transaction with default options.
            pub $($async)* fn begin(&$($mutable)* self) -> Result<$tx, <C::Source as $executor>::Error> {
                self.begin_with(TxOptions::default()) $($await)*
            }

            /// Opens a native transaction with the requested options.
            pub $($async)* fn begin_with(
                &$($mutable)* self,
                options: TxOptions,
            ) -> Result<$tx, <C::Source as $executor>::Error> {
                <C::Source as $source>::$open(self.$access(), options) $($await)*
            }

            /// Opens a native transaction with statically supported row-level security.
            pub $($async)* fn begin_with_security(
                &$($mutable)* self,
                options: TxOptions,
                context: SecurityContext,
            ) -> Result<SecureTx<$tx>, <C::Source as $executor>::Error>
            where
                <C::Source as $executor>::Dialect: Supports<caps::NativeRls>,
            {
                let tx = self.begin_with(options) $($await)*?;

                $apply(tx, context) $($await)*
            }

            /// Checks native row-level security before opening the transaction.
            pub $($async)* fn try_begin_with_security(
                &$($mutable)* self,
                options: TxOptions,
                context: SecurityContext,
            ) -> Result<SecureTx<$tx>, <C::Source as $executor>::Error>
            where
                <C::Source as $executor>::Error: From<TxError>,
            {
                check_native_rls(<C::Source as $dialect>::dialect(self.source()))?;

                let tx = self.begin_with(options) $($await)*?;

                $apply(tx, context) $($await)*
            }
        }
    };
}

transaction_methods!(
    SyncMode,
    Context,
    TransactionSource,
    Acquire,
    TransactionSource,
    open,
    source,
    <C::Source as TransactionSource>::Tx,
    crate::security::apply,
    [],
    [],
    []
);

transaction_methods!(ExclusiveSyncMode, ContextMut, Transactional, Executor, Executor,
    begin_with, source_mut, <C::Source as Transactional>::Tx<'_>, crate::security::apply,
    [], [], [mut]);

#[cfg(feature = "async")]
transaction_methods!(AsyncMode, Context, AsyncTransactionSource, AcquireAsync, AsyncTransactionSource,
    open, source, <C::Source as AsyncTransactionSource>::Tx, crate::security::apply_security,
    [async], [.await], []);

#[cfg(feature = "async")]
transaction_methods!(ExclusiveAsyncMode, ContextMut, AsyncTransactional, AsyncExecutor, AsyncExecutor,
    begin_with, source_mut, <C::Source as AsyncTransactional>::Tx<'_>, crate::security::apply_security,
    [async], [.await], [mut]);
