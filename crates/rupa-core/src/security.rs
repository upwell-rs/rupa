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

use std::collections::BTreeMap;
use std::future::Future;

use crate::dialect::{Supports, caps};
use crate::exec::{AsyncExecutor, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{AsyncTransaction, AsyncTransactional, Transaction, Transactional, TxOptions};

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

    pub fn set(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.insert(key.into(), value.into());
        self
    }

    /// The role to switch to for the transaction (`SET LOCAL ROLE`).
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

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

impl<T: AsyncExecutor> AsyncExecutor for SecureTx<T> {
    type Dialect = T::Dialect;
    type Error = T::Error;

    fn dialect(&self) -> &T::Dialect {
        self.tx.dialect()
    }

    fn execute(
        &mut self,
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
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<T::Tx<'_>, T::Error>> + Send {
        self.tx.begin_with(options)
    }
}

/// Opening transactions with a [`SecurityContext`]. Available on every
/// transactional executor, but only callable where the dialect has native
/// row-level security: elsewhere it does not compile.
pub trait SecureTransactional: Transactional {
    fn begin_with_security(
        &mut self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<Self::Tx<'_>>, Self::Error>
    where
        Self::Dialect: Supports<caps::NativeRls>,
    {
        let mut tx = self.begin_with(options)?;
        if !context.is_empty() {
            // On failure `tx` is dropped, which rolls it back.
            tx.execute(&Statement::ApplySecurity(context.clone()), Expect::Affected)?;
        }
        Ok(SecureTx { tx, context })
    }
}

impl<E: Transactional + ?Sized> SecureTransactional for E {}

/// Async counterpart of [`SecureTransactional`].
pub trait AsyncSecureTransactional: AsyncTransactional {
    fn begin_with_security(
        &mut self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<Self::Tx<'_>>, Self::Error>> + Send
    where
        Self::Dialect: Supports<caps::NativeRls>,
    {
        async move {
            let mut tx = self.begin_with(options).await?;
            if !context.is_empty() {
                tx.execute(&Statement::ApplySecurity(context.clone()), Expect::Affected)
                    .await?;
            }
            Ok(SecureTx { tx, context })
        }
    }
}

impl<E: AsyncTransactional + ?Sized> AsyncSecureTransactional for E {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_validated() {
        assert!(is_valid_key("tenant_id") && is_valid_key("_x1"));
        for bad in ["", "Tenant", "1x", "a.b", "a-b", "a b", "x'"] {
            assert!(!is_valid_key(bad), "{bad}");
        }
        let ctx = SecurityContext::new().set("tenant_id", "1").set("Bad", "2");
        assert_eq!(ctx.invalid_key(), Some("Bad"));
        assert_eq!(
            SecurityContext::new().namespace("my-app").invalid_key(),
            Some("my-app")
        );
    }

    #[test]
    fn settings_are_namespaced() {
        let ctx = SecurityContext::new()
            .set("tenant_id", "42")
            .role("app_user");
        assert_eq!(
            ctx.settings().collect::<Vec<_>>(),
            [("app.tenant_id".to_string(), "42")]
        );
        assert_eq!(ctx.get_role(), Some("app_user"));
    }
}
