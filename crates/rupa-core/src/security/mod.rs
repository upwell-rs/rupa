//! Row-level security, delegated to the database's own policies.
//!
//! Only dialects with native row-level security (`Capability::NativeRls`:
//! Postgres) are supported. Emulating policies by rewriting queries is
//! deliberately not offered: it is hard to make airtight and slow at scale.
//!
//! A [`SecurityContext`] holds the values the database's policies read
//! (`current_setting('app.tenant_id')`) and, optionally, a role to switch to
//! (superusers bypass row-level security, so applications usually do).
//! [`SecureTransactional::begin_with_security`] opens a transaction and applies
//! the context inside it in one step, returning a [`SecureTx`] that carries
//! the context with it: a transaction and its context never travel separately.
//!
//! Settings are transaction-local (`set_config(.., true)`): they end with the
//! transaction, so a pooled connection never leaks one request's context into
//! the next.

#[cfg(feature = "async")]
mod asynchronous;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::dialect::{Capability, Dialect, Supports, caps};
use crate::error::TxError;
use crate::exec::{Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{Transaction, Transactional, TxOptions};

#[cfg(feature = "async")]
pub use asynchronous::{AsyncSecureTransactional, apply_security};

/// Values for the database's row-level security policies.
///
/// ```ignore
/// let ctx = SecurityContext::new().set("tenant_id", "42").set("user_id", "7").role("app_user");
/// // Postgres: set_config('app.tenant_id', '42', true), .., set_config('role', 'app_user', true)
/// ```
///
/// Keys are setting names under a namespace (`app` by default): lowercase
/// letters, digits and `_`, starting with a letter or `_`. Keys, values and
/// the role are all sent as bound parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityContext {
    namespace: String,
    entries: BTreeMap<String, String>,
    role: Option<String>,
}

impl Default for SecurityContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `s` is a valid setting-name segment.
pub fn is_valid_key(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl SecurityContext {
    /// An empty context in the `app` namespace.
    pub fn new() -> Self {
        Self {
            namespace: "app".into(),
            entries: BTreeMap::new(),
            role: None,
        }
    }

    /// The setting namespace (`app` in `app.tenant_id`).
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Sets `key` (read by policies as `current_setting('<namespace>.<key>')`).
    pub fn set(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.insert(key.into(), value.into());
        self
    }

    /// The role to switch to for the transaction (`SET LOCAL ROLE`).
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// The value set for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// The role the transaction switches to, if any.
    pub fn get_role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    /// `(full setting name, value)` pairs, e.g. `("app.tenant_id", "42")`.
    pub fn settings(&self) -> impl Iterator<Item = (String, &str)> {
        self.entries
            .iter()
            .map(|(k, v)| (format!("{}.{k}", self.namespace), v.as_str()))
    }

    /// The first invalid key or namespace, if any.
    pub fn invalid_key(&self) -> Option<&str> {
        if !is_valid_key(&self.namespace) {
            return Some(&self.namespace);
        }

        self.entries
            .keys()
            .find(|k| !is_valid_key(k))
            .map(String::as_str)
    }

    /// Whether the context sets nothing (applying it is a no-op).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.role.is_none()
    }
}

/// A transaction opened with a [`SecurityContext`] applied.
///
/// It is an executor and a transaction like any other (repositories run in
/// it unchanged). Nested transactions are savepoints of the same
/// transaction and see the same context.
#[derive(Debug)]
pub struct SecureTx<T> {
    tx: T,
    context: SecurityContext,
}

impl<T> SecureTx<T> {
    /// The context this transaction runs under.
    pub fn context(&self) -> &SecurityContext {
        &self.context
    }

    /// The transaction, without its context.
    pub fn inner(&mut self) -> &mut T {
        &mut self.tx
    }
}

impl<T: Executor> Executor for SecureTx<T> {
    type Dialect = T::Dialect;
    type Error = T::Error;

    fn dialect(&self) -> &T::Dialect {
        self.tx.dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, T::Error> {
        self.tx.execute(statement, expect)
    }
}

impl<T: Transaction> Transaction for SecureTx<T> {
    fn commit(self) -> Result<(), T::Error> {
        self.tx.commit()
    }

    fn rollback(self) -> Result<(), T::Error> {
        self.tx.rollback()
    }
}

impl<T: Transactional> Transactional for SecureTx<T> {
    type Tx<'s>
        = T::Tx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<T::Tx<'_>, T::Error> {
        self.tx.begin_with(options)
    }
}

/// Opening transactions with a [`SecurityContext`]. Available on every
/// transactional executor, but only callable where the dialect has native
/// row-level security: elsewhere it does not compile.
pub trait SecureTransactional: Transactional {
    /// Opens a transaction with `context` applied: one step, so the
    /// transaction and its context cannot drift apart. Only compiles for
    /// dialects with native row-level security; see
    /// [`try_begin_with_security`](Self::try_begin_with_security) for erased
    /// executors.
    fn begin_with_security(
        &mut self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<Self::Tx<'_>>, Self::Error>
    where
        Self::Dialect: Supports<caps::NativeRls>,
    {
        apply(self.begin_with(options)?, context)
    }

    /// [`begin_with_security`](Self::begin_with_security), checked at run
    /// time instead: for executors whose dialect is only known at run time
    /// ([`BoxExecutor`](crate::erased::BoxExecutor)). Fails with
    /// [`TxError::Unsupported`] unless the dialect has native row-level
    /// security.
    fn try_begin_with_security(
        &mut self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<Self::Tx<'_>>, Self::Error>
    where
        Self::Error: From<TxError>,
    {
        check_native_rls(self.dialect())?;

        apply(self.begin_with(options)?, context)
    }
}

impl<E: Transactional + ?Sized> SecureTransactional for E {}

/// `Ok` if `dialect` enforces row-level security natively.
pub fn check_native_rls(dialect: &dyn Dialect) -> Result<(), TxError> {
    if dialect.supports(Capability::NativeRls) {
        Ok(())
    } else {
        Err(TxError::Unsupported {
            dialect: dialect.id(),
            capability: Capability::NativeRls,
        })
    }
}

/// Applies `context` inside the freshly opened `tx`. On failure `tx` is
/// dropped, which rolls it back.
pub(crate) fn apply<T: Executor>(
    mut tx: T,
    context: SecurityContext,
) -> Result<SecureTx<T>, T::Error> {
    if !context.is_empty() {
        tx.execute(&Statement::ApplySecurity(context.clone()), Expect::Affected)?;
    }

    Ok(SecureTx { tx, context })
}

impl<T> SecureTx<T> {
    /// Wraps a transaction on which `context` has already been applied.
    /// Prefer `begin_with_security`, which applies it; this is for pools
    /// and adapters that open the transaction themselves.
    pub fn from_parts(tx: T, context: SecurityContext) -> Self {
        Self { tx, context }
    }

    /// The transaction, without its context.
    pub fn into_inner(self) -> T {
        self.tx
    }
}
