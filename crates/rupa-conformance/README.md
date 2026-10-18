# rupa-conformance

## Role

The behavioural test suite every driver runs, so that a query means the same
on every backend. Not published.

## Usage

A driver's tests implement `Harness` (or `DatabaseHarness`) and call `run`,
`run_transactions` or `run_database`.

## Internal role

Cases are written once against `AsyncExecutor`; sync drivers run them through
`Blocking`. `cases` covers selects, writes and DSL functions, `joins` the join
builder and declared relations, `tx_cases` transactions, `database` pools and
`Database`.

## Feature flags

None.
