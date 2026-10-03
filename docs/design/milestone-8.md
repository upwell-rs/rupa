# Milestone 8: row-level security (native only)

Status: **implemented.**

## Scope: native policies only (decided 2026-10-03)

Row-level security is delegated to databases that enforce it natively; today
that's Postgres (`Capability::NativeRls`). The spec's emulated mode (a
`Policy<E>` that rewrites queries) is dropped. It is hard to make airtight
(every query shape, joins later, raw SQL) and slow at scale, and a database
policy is enforced no matter how a query reaches it.

## API (`rupa_core::security`, re-exported by `rupa`)

```rust
let ctx = SecurityContext::new()
    .set("tenant_id", tenant.to_string())   // read by policies as current_setting('app.tenant_id')
    .role("app_user");                       // SET LOCAL ROLE: superusers bypass RLS

let mut tx = db.begin_with_security(TxOptions::default(), ctx).await?;   // sync: SecureTransactional
tx.context();                                // the context travels with the transaction
Repo::new(&mut tx).after(0).await?;          // repositories run in it unchanged
tx.commit().await?;
```

- **One step.** `begin_with_security` opens the transaction and applies the
  context inside it, returning a `SecureTx<Tx>`. That's an executor and a
  transaction like any other; nested `begin` opens savepoints of the same
  transaction, under the same context. A transaction and its context can't
  drift apart: there is no separate context to hold (relevant for
  `rupa-upwell`).
- **Gated at compile time.** The method exists on every transactional
  executor but carries `where Self::Dialect: Supports<caps::NativeRls>`.
  Against memory, MySQL or SQLite it doesn't compile: *"dialect `Memory` does
  not support `NativeRls`"*. This is the spec's "setup-time error, never
  silently ignored", moved to compile time. The renderer also refuses
  `ApplySecurity` for other dialects, as a second line of defence.
- **Rendering (Postgres).** One statement,
  `SELECT set_config($1, $2, true), …, set_config($n, $m, true)`:
  - Every setting name (`app.tenant_id`) and value is a bound parameter.
  - So is the role, set through the `role` setting, which is the bindable
    equivalent of `SET LOCAL ROLE`.
  - `true` makes each setting transaction-local.
- **Validation.** Keys and the namespace must be lowercase setting names
  (`[a-z_][a-z0-9_]*`), otherwise `RenderError::InvalidSecurityKey`. An
  invalid context fails at `begin_with_security`, and the transaction is
  rolled back.
- **Pooled connections are safe.** Settings end with the transaction, so
  nothing leaks to the next transaction or the next user of the connection.
  A test checks this explicitly.

## Writing the policies (recipe)

```sql
ALTER TABLE docs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON docs
    USING      (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::bigint)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::bigint);
CREATE ROLE app_user NOLOGIN;
GRANT SELECT, INSERT, UPDATE, DELETE ON docs TO app_user;
```

- `current_setting(.., true)` returns NULL instead of failing when the setting
  was never set.
- `NULLIF(.., '')` is needed because a custom setting that was set in an
  earlier, finished transaction reads back as `''`, not NULL.
- Superusers and table owners bypass RLS, unless the table uses
  `FORCE ROW LEVEL SECURITY` (owners) or the session switches to an
  unprivileged role. Hence `.role(..)`.

## Tests

- **Real Postgres, async driver:**
  - tenant isolation;
  - `WITH CHECK` refusing another tenant's row;
  - savepoints and a `&mut self` repository inside a secure transaction;
  - no leak into the next transaction (a role without a tenant sees nothing,
    the superuser sees everything);
  - rollback on drop;
  - an invalid key refused, with the connection still usable.
- **Real Postgres, sync driver:** isolation, `WITH CHECK`, and immediate
  rollback on drop.
- **Renderer:** the exact SQL and parameters; refusal on MySQL and SQLite;
  invalid keys.
- **Compile error:** `begin_with_security` on the memory backend (sync and
  async).

## Not yet

- **A runtime-checked variant** for executors whose dialect is chosen at run
  time (`DynDialect`). Boxed executors aren't transactional yet; this comes
  with the dyn transaction work in milestone 9.
