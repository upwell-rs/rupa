# rupa-example-app

## Role

Example application crate that applies the `db` crate's migrations on startup.

## Usage

`cargo run -p rupa-example-app` migrates an in-memory database;
`cargo run -p rupa-example-app -- app.db` persists history, so a second run
applies nothing.

## Internal role

Owns the dedicated migration connection and calls `catalog().apply(connection)`.
It depends on the catalog only, never on how `db` discovered it.

## Feature flags

None.
