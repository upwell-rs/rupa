# rupa-driver-sqlx

## Role

Async executors and pools for Postgres, MySQL and SQLite on sqlx and deadpool.

## Usage

`rupa_driver_sqlx::postgres::PgPool::connect(&config).await?` (or `mysql::`, `sqlite::`). Usually enabled through a feature of the `rupa` facade.

## Internal role

One generic `SqlxExecutor<DB>` and `SqlxPool<DB>`; each database implements `Backend` (binding and decoding) and, with `pool`, `PoolBackend` (config to connect options).

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `postgres`, `mysql` | those databases |
| `sqlite` / `sqlite-unbundled` | SQLite compiled in / linked from the system |
| `pool` | `SqlxPool` on deadpool (tokio) |
| `dynamic` | `SqlxDriver`, the registry entries (implies `pool`) |
| `tls-rustls` | TLS on rustls (ring, webpki roots), configured in the URL |
