//! Caching rendered SQL by statement *shape*.
//!
//! Rendering a statement (identifier quoting, operator precedence, dialect
//! emulations) costs more than everything else rupa does per query. But most
//! queries an application runs come from a few hundred call sites, and a call
//! site renders the same SQL every time: only the bound values change. This
//! cache renders each shape once per dialect and reuses the text.
//!
//! # What is a shape
//!
//! One pass over the statement emits a token stream: the dialect and its
//! capabilities, every node kind, operator, identifier, JSON path step, cast
//! type and list length, and a bare "parameter here" token for every bound
//! value. That stream is the key, compared exactly on every hit (the hash
//! only picks the bucket), so two statements share an entry only if they
//! render identically.
//!
//! **Parameter values are never part of the key and never stored.** An entry
//! holds the SQL text and a *binding plan*: for each placeholder, the
//! position of the parameter (in the shape's order) that fills it. A hit binds
//! the caller's own values through the plan. Recording the plan on a miss
//! needs no assumption about the order the renderer emits parameters in (some
//! dialects emit one twice): the renderer reports which IR value fed each
//! placeholder.
//!
//! # What is not cached
//!
//! - `ApplySecurity` statements: their parameters are created while rendering.
//!   They run once per transaction anyway.
//! - SQL longer than [`MAX_CACHED_SQL_LEN`] (huge multi-row inserts, long `IN`
//!   lists), so a few large shapes can't crowd out the common ones.
//! - Statements that fail to render (the error is returned, not cached).
//!
//! DSL calls are lowered *before* the shape is taken. So the shape is that of
//! the SQL actually rendered, even for a function whose lowering depends on
//! its arguments.
//!
//! # Bounds
//!
//! [`RenderCache`] is an LRU of at most `capacity` shapes, split into shards
//! with a lock each, so concurrent connections rarely contend. The process-wide
//! instance ([`render_cache`]) holds [`DEFAULT_CAPACITY`] shapes. Drivers use
//! it through [`render_cached`]; set its capacity to `0` to turn it off.
//!
//! The cached SQL is shared (`Arc<str>`), which also suits drivers' own
//! prepared-statement caches: they are keyed by this text.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use hashlink::LruCache;
use rupa_core::Value;
use rupa_core::dialect::Dialect;
use rupa_core::ir::Statement;

use crate::writer::{MAX_LOWERING_DEPTH, Writer};
use crate::{RenderError, render, syntax_for};

mod lower;
mod shape;

use lower::lower_statement;
use shape::Shape;

/// Shapes the process-wide cache holds by default.
pub const DEFAULT_CAPACITY: usize = 1024;

/// Rendered SQL longer than this (in bytes) is not cached.
pub const MAX_CACHED_SQL_LEN: usize = 16 * 1024;

const SHARDS: usize = 16;

/// SQL text, shared with the cache, and the values to bind, in placeholder
/// order.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedSql {
    /// The SQL text, with the dialect's placeholders.
    pub sql: Arc<str>,
    /// The values to bind, in placeholder order.
    pub params: Vec<Value>,
}

/// Counters of a [`RenderCache`], since it was created or [`cleared`](RenderCache::clear).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Renders served from the cache.
    pub hits: u64,
    /// Cacheable renders that were not cached yet.
    pub misses: u64,
    /// Renders that bypassed the cache (see the [module docs](self)).
    pub bypassed: u64,
    /// Shapes currently cached.
    pub entries: usize,
    /// The most shapes the cache holds.
    pub capacity: usize,
}

struct Entry {
    /// The full shape, compared on every hit.
    shape: Box<[u64]>,
    sql: Arc<str>,
    /// For each placeholder, the index of the shape parameter that fills it;
    /// `None` when that is simply the shape's order (the common case).
    plan: Option<Box<[u32]>>,
}

type Shard = Mutex<LruCache<u64, Arc<Entry>, PassThrough>>;

/// A bounded cache of rendered SQL; see the [module docs](self).
pub struct RenderCache {
    shards: Box<[Shard]>,
    capacity: AtomicUsize,
    hits: AtomicU64,
    misses: AtomicU64,
    bypassed: AtomicU64,
}

impl RenderCache {
    /// A cache for at most `capacity` shapes (`0`: render every time).
    pub fn new(capacity: usize) -> Self {
        let shards = (0..SHARDS)
            .map(|_| Mutex::new(LruCache::with_hasher(shard_capacity(capacity), PassThrough)))
            .collect();
        Self {
            shards,
            capacity: AtomicUsize::new(capacity),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            bypassed: AtomicU64::new(0),
        }
    }

    /// The most shapes the cache holds.
    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::Relaxed)
    }

    /// Changes the capacity (`0` turns caching off), evicting what no longer
    /// fits.
    pub fn set_capacity(&self, capacity: usize) {
        self.capacity.store(capacity, Ordering::Relaxed);
        for shard in self.shards.iter() {
            lock(shard).set_capacity(shard_capacity(capacity));
        }
    }

    /// Forgets every shape and resets the counters.
    pub fn clear(&self) {
        for shard in self.shards.iter() {
            lock(shard).clear();
        }

        for counter in [&self.hits, &self.misses, &self.bypassed] {
            counter.store(0, Ordering::Relaxed);
        }
    }

    /// The cache's counters.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            bypassed: self.bypassed.load(Ordering::Relaxed),
            entries: self.shards.iter().map(|s| lock(s).len()).sum(),
            capacity: self.capacity(),
        }
    }

    /// Renders `statement` for `dialect`, reusing the SQL of an earlier
    /// statement of the same shape.
    ///
    /// Re-entered (a DSL lowering rendering SQL itself): use a fresh buffer.
    pub fn render(
        &self,
        statement: &Statement,
        dialect: &dyn Dialect,
    ) -> Result<CachedSql, RenderError> {
        if self.capacity() == 0 || matches!(statement, Statement::ApplySecurity(_)) {
            return Ok(self.bypass(render(statement, dialect)?));
        }

        TOKENS.with(|buffer| match buffer.try_borrow_mut() {
            Ok(mut tokens) => self.render_shaped(statement, dialect, &mut tokens),
            Err(_) => self.render_shaped(statement, dialect, &mut Vec::new()),
        })
    }

    /// Shape what is actually rendered: the statement with its DSL
    /// calls lowered (see the module docs).
    ///
    /// Miss: render, recording which IR value feeds each placeholder.
    fn render_shaped(
        &self,
        statement: &Statement,
        dialect: &dyn Dialect,
        tokens: &mut Vec<u64>,
    ) -> Result<CachedSql, RenderError> {
        let mut shape = Shape::new(dialect, tokens, false);
        shape.statement(statement);
        if shape.saw_dsl {
            let lowered = lower_statement(statement, dialect)?;
            return self.render_shaped(&lowered, dialect, tokens);
        }

        let values = std::mem::take(&mut shape.values);
        let hash = hash(tokens);
        let shard = &self.shards[hash as usize % SHARDS];

        let cached = lock(shard)
            .get(&hash)
            .filter(|e| *e.shape == **tokens)
            .cloned();
        if let Some(entry) = cached {
            self.hits.fetch_add(1, Ordering::Relaxed);
            let params = match &entry.plan {
                None => values,
                Some(plan) => plan.iter().map(|&i| values[i as usize].clone()).collect(),
            };
            return Ok(CachedSql {
                sql: entry.sql.clone(),
                params,
            });
        }

        let (rendered, sources) =
            Writer::new(dialect, syntax_for(dialect)?).statement_with_sources(statement)?;
        let plan = (rendered.sql.len() <= MAX_CACHED_SQL_LEN)
            .then(|| {
                let mut scratch = Vec::new();
                let mut recording = Shape::new(dialect, &mut scratch, true);
                recording.statement(statement);
                binding_plan(&recording.refs, &sources)
            })
            .flatten();
        let Some(plan) = plan else {
            return Ok(self.bypass(rendered));
        };
        self.misses.fetch_add(1, Ordering::Relaxed);
        let sql: Arc<str> = rendered.sql.into();
        let entry = Arc::new(Entry {
            shape: tokens.as_slice().into(),
            sql: sql.clone(),
            plan,
        });
        lock(shard).insert(hash, entry);
        Ok(CachedSql {
            sql,
            params: rendered.params,
        })
    }

    fn bypass(&self, rendered: crate::Rendered) -> CachedSql {
        self.bypassed.fetch_add(1, Ordering::Relaxed);
        CachedSql {
            sql: rendered.sql.into(),
            params: rendered.params,
        }
    }
}

impl Default for RenderCache {
    /// A cache of [`DEFAULT_CAPACITY`] shapes.
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl std::fmt::Debug for RenderCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderCache")
            .field("stats", &self.stats())
            .finish()
    }
}

/// The process-wide render cache that drivers use.
pub fn render_cache() -> &'static RenderCache {
    static CACHE: OnceLock<RenderCache> = OnceLock::new();
    CACHE.get_or_init(RenderCache::default)
}

/// Renders `statement` through the process-wide [`render_cache`].
pub fn render_cached(
    statement: &Statement,
    dialect: &dyn Dialect,
) -> Result<CachedSql, RenderError> {
    render_cache().render(statement, dialect)
}

fn shard_capacity(capacity: usize) -> usize {
    capacity.div_ceil(SHARDS)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The cache holds no invariants a panic could break mid-update.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// For each placeholder, the index of the shape parameter whose IR value fed
/// it: `Some(None)` when that is the shape's own order, `None` if some
/// placeholder's value isn't a shape parameter at all (uncacheable).
#[allow(clippy::option_option, reason = "uncacheable vs. identity plan")]
fn binding_plan(params: &[&Value], sources: &[*const Value]) -> Option<Option<Box<[u32]>>> {
    let identity = params.len() == sources.len()
        && params
            .iter()
            .zip(sources)
            .all(|(p, s)| std::ptr::eq(*p, *s));
    if identity {
        return Some(None);
    }

    let index: HashMap<*const Value, u32> = params
        .iter()
        .enumerate()
        .map(|(i, v)| (std::ptr::from_ref(*v), u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    sources
        .iter()
        .map(|s| index.get(s).copied())
        .collect::<Option<Box<[u32]>>>()
        .map(Some)
}

thread_local! {
    /// Token buffer reused across renders on this thread.
    static TOKENS: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// FxHash-style mixing; tokens are compared exactly anyway.
fn hash(tokens: &[u64]) -> u64 {
    tokens.iter().fold(0u64, |h, &t| {
        (h.rotate_left(5) ^ t).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95)
    })
}

/// Keys of the LRU are already hashes: pass them through.
#[derive(Debug, Clone, Copy, Default)]
struct PassThrough;

impl std::hash::BuildHasher for PassThrough {
    type Hasher = PassThroughHasher;
    fn build_hasher(&self) -> PassThroughHasher {
        PassThroughHasher(0)
    }
}

struct PassThroughHasher(u64);

impl std::hash::Hasher for PassThroughHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}
