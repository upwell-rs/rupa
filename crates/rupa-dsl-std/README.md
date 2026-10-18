# rupa-dsl-std

## Role

The built-in DSL functions (`ilike`, JSON containment, ..), each with a
lowering per dialect and an `eval` for the memory backend.

## Usage

`rupa::dsl::ilike(email, $pattern)` in `#[query(filter = ..)]`, or
`rupa::dsl::ilike(col!(User::email), "a%")` with the builder.

## Internal role

Written with `#[rupa::dsl::function]`, like user functions, so it exercises
the same macro.

## Feature flags

None.
