# rupa-driver-sqlx

## Role

Async executors and pools for Postgres, MySQL and SQLite on sqlx and deadpool.

## Usage

`rupa_driver_sqlx::postgres::PgPool::connect(&config).await?` (or `mysql::`, `sqlite::`). Usually enabled through a feature of the `rupa` facade.

## Internal role

One generic `SqlxExecutor<DB>` and `SqlxPool<DB>`; each database implements `Backend` (binding and decoding) and, with `pool`, `PoolBackend` (config to connect options).

With `migrations`, every enabled database's executor is a `rupa_migrate::asynchronous::Session` over the engine protocol it shares with the native driver (`rupa_migrate::protocol`), through one generic `Io` implementation: PostgreSQL runs `TransactionalRequired` and `NonTransactional` bodies, MySQL `NonTransactional`, SQLite `TransactionalRequired`. sqlx has no SQLite authorizer API, so the SQLite session installs the shared policy through the locked raw handle (`libsqlite3-sys`, the build sqlx links). `SqlxPool` discards connections a run left owned, mid-attempt or poisoned.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `postgres`, `mysql` | those databases |
| `sqlite` / `sqlite-unbundled` | SQLite compiled in / linked from the system |
| `pool` | `SqlxPool` on deadpool (tokio) |
| `dynamic` | `SqlxDriver`, the registry entries (implies `pool`) |
| `tls-rustls` | TLS on rustls (ring, webpki roots), configured in the URL |
| `migrations` | `rupa_migrate::asynchronous::Session` on each enabled database's `SqlxExecutor` |
