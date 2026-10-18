# rupa-driver-mysql

## Role

Sync MySQL (8.0+) executor and pool on the `mysql` crate and r2d2.

## Usage

`MySqlPool::connect(&DbConfig::new("mysql://user:pw@host/db"))?`. In async code prefer the sqlx driver. Usually enabled through a feature of the `rupa` facade.

## Internal role

Connections set `CLIENT_FOUND_ROWS` so updates report matched rows as Postgres does. The pool is the core's `r2d2_pool!`.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `pool` | `MySqlPool` on r2d2 |
| `dynamic` | `MySqlDriver`, the registry entry (implies `pool`) |
| `tls-rustls` | TLS on rustls (ring) |
