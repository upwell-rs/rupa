# rupa-bench

## Role

Criterion benchmarks for rupa. Not published. See `docs/benchmarks.md`.

## Usage

```sh
cargo bench -p rupa-bench --bench build_render --bench execute -- --quick
```

## Internal role

Shared fixtures (entities, generated rows) live in `src/lib.rs`; each bench
is a file under `benches/`.

## Feature flags

None.
