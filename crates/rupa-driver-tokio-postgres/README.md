# rupa-driver-tokio-postgres

## Role

Async Postgres executor and pool on tokio-postgres and deadpool.

## Usage

`PgPool::connect(&DbConfig::new("postgres://..")).await?`, inside a tokio runtime. Usually enabled through a feature of the `rupa` facade.

## Internal role

Keeps server-side prepared statements per connection in an LRU (`StatementCache`, on `hashlink`). `shared` is reused by the sync driver, as is `protocol`: what both drivers need to implement the shared PostgreSQL migration protocol (`rupa_migrate::protocol::postgres`).

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `pool` | `PgPool` on deadpool (tokio) |
| `dynamic` | `PgDriver` (`postgres`), the registry entry (implies `pool`) |
| `migration-protocol` | `PgError::Migration` and the row decoding both PostgreSQL drivers use for the shared migration protocol (no async migration code) |
| `migrations` | `rupa_migrate::asynchronous::Session` on `PgExecutor`, with `TransactionalRequired` and `NonTransactional` bodies (implies `migration-protocol`) |
| `tls-rustls` | TLS on rustls (ring, webpki roots), as the URL's `sslmode` asks |
