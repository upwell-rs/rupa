//! Executors and transaction openers forwarded through references and smart
//! pointers: `&E`, `&mut E`, `Box<E>`, `Rc<E>` and `Arc<E>` run statements and
//! open transactions exactly as `E` does.

#[cfg(feature = "async")]
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use super::{Engine, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{Transactional, TxOptions};
#[cfg(feature = "async")]
use {super::AsyncExecutor, crate::tx::AsyncTransactional};

macro_rules! forward {
    ($($wrapper:ty),* $(,)?) => {$(
        impl<E: Engine + ?Sized> Engine for $wrapper {
            type Completion = E::Completion;
        }

        impl<E: Executor + ?Sized> Executor for $wrapper {
            type Dialect = E::Dialect;
            type Error = E::Error;

            fn dialect(&self) -> &E::Dialect {
                (**self).dialect()
            }

            fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, E::Error> {
                (**self).execute(statement, expect)
            }
        }

        impl<E: Transactional + ?Sized> Transactional for $wrapper {
            type Tx<'t>
                = E::Tx<'t>
            where
                Self: 't;

            fn begin_with(&self, options: TxOptions) -> Result<E::Tx<'_>, E::Error> {
                (**self).begin_with(options)
            }
        }

        #[cfg(feature = "async")]
        impl<E: AsyncExecutor + ?Sized> AsyncExecutor for $wrapper
        where
            Self: Send + Sync,
        {
            type Dialect = E::Dialect;
            type Error = E::Error;

            fn dialect(&self) -> &E::Dialect {
                (**self).dialect()
            }

            fn execute(
                &self,
                statement: &Statement,
                expect: Expect,
            ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
                (**self).execute(statement, expect)
            }
        }

        #[cfg(feature = "async")]
        impl<E: AsyncTransactional + ?Sized> AsyncTransactional for $wrapper
        where
            Self: Send + Sync,
        {
            type Tx<'t>
                = E::Tx<'t>
            where
                Self: 't;

            fn begin_with(
                &self,
                options: TxOptions,
            ) -> impl Future<Output = Result<E::Tx<'_>, E::Error>> + Send {
                (**self).begin_with(options)
            }
        }
    )*};
}

forward!(&E, &mut E, Box<E>, Rc<E>, Arc<E>);
