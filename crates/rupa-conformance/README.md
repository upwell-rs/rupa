# rupa-conformance

## Role

The behavioural test suite every driver runs, so that a query means the same
on every backend. Not published.

## Usage

A driver's tests implement `Harness` (or `DatabaseHarness`) and call `run`,
`run_transactions` or `run_database`. Drivers with a migration session
implement `migrations::sync::{Harness, Target}` and call `migrations::sync::run`,
then run the same cases on the async contracts with
`migrations::asynchronous::run(&mut Blocked(harness))`. The harness must be
`Sync`: the concurrency case shares it across threads.

## Internal role

Cases are written once against `AsyncExecutor`; sync drivers run them through
`Blocking`. `cases` covers selects, writes and DSL functions, `joins` the join
builder and declared relations, `tx_cases` transactions, `database` pools and
`Database`, `migrations` migration history, exclusion and control guarantees
(with fixtures drivers reuse in their own migration tests).

## Feature flags

| feature | adds |
|---|---|
| `migrations` | the migration cases in both modes and their fixtures |
