# rupa-driver-postgres

## Role

Sync Postgres executor and pool on the `postgres` crate and r2d2. Cannot run inside a tokio runtime.

## Usage

`PgPool::connect(&DbConfig::new("postgres://.."))?`. Usually enabled through a feature of the `rupa` facade.

## Internal role

Value conversion, the prepared-statement cache, the error type and the URL/TLS setup are shared with `rupa-driver-tokio-postgres` (`shared`).

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `pool` | `PgPool` on r2d2 |
| `dynamic` | `PgDriver` (`postgres-sync`), the registry entry (implies `pool`) |
| `tls-rustls` | TLS on rustls (ring, webpki roots), as the URL's `sslmode` asks |
