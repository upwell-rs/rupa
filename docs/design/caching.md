# Caching rendered SQL and prepared statements

Status: **implemented** (2026-10-17). Closes the statement-caching item
agreed in milestone 2 and listed in [`upwell-prep.md`](upwell-prep.md).

A repeated query now skips both of its fixed costs:

| layer | cache | key | scope | bound |
|---|---|---|---|---|
| rendering (`rupa-sql`) | `RenderCache` | statement **shape** + dialect | process-wide | LRU, 1024 shapes, sharded |
| parsing/planning (the database) | native prepared statements | SQL text | per connection | LRU, per driver (below) |

## Rendered SQL, by shape

### The rule: values never enter the cache

The key is the statement's *shape*. One pass over the IR emits a token
stream:

- the dialect id and its capability set;
- every node kind and operator;
- identifiers, by the address and length of their `&'static str`;
- JSON path steps, cast types, and list lengths (an `IN` list of 3 items is a
  different shape from one of 4);
- a bare "parameter here" token per bound value.

The stream is both the key and the equality check. Its hash only picks a
shard and a bucket, and a hit requires the stored stream to be *identical*.
So two statements share SQL only if the renderer would produce the same text
for both.

An entry holds the SQL text (`Arc<str>`, shared with callers) and a
**binding plan**: for each placeholder, which of the shape's parameters fills
it. A hit clones the *caller's* values through the plan. No value, not even a
redacted one, is stored, logged, or compared. This is what "don't cache with
actual params" means here: two calls with different values share SQL text,
and nothing else.

### Why a plan, and how it's recorded

Usually placeholder *n* is the *n*th parameter of the IR, and the plan is
stored as "identity" (`None`). But not always: MySQL emulates `NULLS LAST` by
rendering a sort key twice, so a parameter inside it fills two placeholders.
Rather than mirror the renderer's emission order in the cache, which would be
two pieces of code that must agree forever, the renderer *reports* it. On a
miss, the writer records the address of the IR value behind each placeholder,
and the cache maps those addresses back to the shape's parameter positions.
Any placeholder whose value isn't an IR parameter makes the statement
uncacheable.

### DSL functions

DSL calls are lowered at render time by user code, which can inspect its
arguments. A lowering that picks different SQL depending on a value would
make a shape taken *before* lowering unsafe. So when the shape pass meets a
DSL call, the statement is lowered first and the *lowered* statement is
shaped. The cost is one clone of statements that use DSL functions. A
value-dependent lowering then simply yields two shapes (tested).

### What bypasses the cache

| case | why |
|---|---|
| `ApplySecurity` (RLS context) | its parameters are created during rendering; it runs once per transaction anyway |
| SQL longer than 16 KiB | huge multi-row inserts and long `IN` lists would crowd out the common shapes |
| render errors | returned, never cached |
| capacity `0` | `rupa_sql::render_cache().set_capacity(0)` turns caching off |

### Bounds and concurrency

`RenderCache` is 16 shards, each a `Mutex` around an O(1) LRU
(`rupa_core::lru::LruCache`, a slab with recency links and a pass-through
hasher, since the keys are already hashes). The lock is held for the lookup
and an `Arc` clone only. Rendering and binding happen outside it, so
connections on different threads rarely contend. The capacity is split evenly
across shards, so one shard can evict while others have room. That is the
usual trade-off for a sharded cache.

`rupa_sql::render_cached` uses the process-wide instance, and every driver
calls it. `RenderCache::stats()` reports hits, misses, bypasses and entries.

## Prepared statements, per connection

Each driver keeps the database's own prepared statements, keyed by the SQL
text above, so the database parses and plans a query once per connection.
Postgres may then switch to a generic plan after a few executions; see
`plan_cache_mode`.

| driver | cache | default | option |
|---|---|---|---|
| tokio-postgres, postgres | `StatementCache`: core LRU of `Statement`s; evicting one closes it on the server; a statement whose execution failed is evicted (it may be stale) | 256 | `PostgresOptions::statement_cache_size` |
| rusqlite | rusqlite's `prepare_cached` LRU | 128 | `SqliteOptions::statement_cache_size` |
| mysql | the `mysql` crate's LRU of server-side statements | 32 | `MySqlOptions::statement_cache_size` |

`0` turns the cache off for that driver. The key is the render cache's
`Arc<str>`, so the Postgres caches store it without copying the text.

## Numbers

`--quick`, same machine and run (Windows 11, Rust 1.99):

| benchmark | uncached | cached |
|---|---|---|
| render `get_by_id`, Postgres / MySQL / SQLite | 387 / 452 / 417 ns | 190 / 164 / 163 ns |
| render `select_filtered_page`, Postgres / MySQL / SQLite | 1.09 / 1.80 / 1.49 µs | 342 / 340 / 348 ns |
| render `insert_100` (700 values) | 36–50 µs | 33–35 µs |
| SQLite `get_by_id` end to end (in memory) | 2.33 µs | 2.18 µs |

- **Rendering:** 2.4–5× faster for typical queries. Large multi-row inserts
  gain little, because cloning their bound values dominates either way.
- **End to end:** an in-memory SQLite lookup saves ~150 ns. The rest is
  SQLite and row decoding, and rusqlite already cached the prepared
  statement before this change.
- **Networked databases:** the prepared-statement cache is the bigger win
  there. It removes a round trip per query, which outweighs everything above.

## Not done

- **Render plans for the memory backend.** It evaluates the IR directly, and
  nothing is rendered.
- **Pipelining or batching** of prepared statements. That is a separate
  feature.
