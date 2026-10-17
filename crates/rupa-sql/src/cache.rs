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

use rupa_core::Value;
use rupa_core::dialect::{Capability, Dialect};
use rupa_core::ir::{
    ColumnRef, ExprNode, FromClause, OrderBy, PathSeg, RawPart, Select, Statement, TableRef,
};
use rupa_core::lru::LruCache;

use crate::{RenderError, render, syntax_for, writer::Writer};

/// Shapes the process-wide cache holds by default.
pub const DEFAULT_CAPACITY: usize = 1024;

/// Rendered SQL longer than this (in bytes) is not cached.
pub const MAX_CACHED_SQL_LEN: usize = 16 * 1024;

const SHARDS: usize = 16;

/// Maximum nesting of DSL lowerings; matches the renderer's limit.
const MAX_LOWERING_DEPTH: usize = 32;

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
            // Re-entered (a DSL lowering rendering SQL itself): use a fresh buffer.
            Err(_) => self.render_shaped(statement, dialect, &mut Vec::new()),
        })
    }

    fn render_shaped(
        &self,
        statement: &Statement,
        dialect: &dyn Dialect,
        tokens: &mut Vec<u64>,
    ) -> Result<CachedSql, RenderError> {
        let mut shape = Shape::new(dialect, tokens, false);
        shape.statement(statement);
        if shape.saw_dsl {
            // Shape what is actually rendered: the statement with its DSL
            // calls lowered (see the module docs).
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

        // Miss: render, recording which IR value feeds each placeholder.
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

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

/// Builds the token stream of a statement into a borrowed buffer, cloning
/// its parameter values in token order (a hit binds exactly those).
struct Shape<'a, 'b> {
    tokens: &'b mut Vec<u64>,
    values: Vec<Value>,
    /// The parameters themselves, when recording a binding plan.
    refs: Vec<&'a Value>,
    recording: bool,
    /// Whether the statement still contains a DSL call.
    saw_dsl: bool,
}

/// Token tags. Each tag is followed by its node's fields; lists are prefixed
/// with their length, so the stream is unambiguous.
mod tag {
    pub const SELECT: u64 = 1;
    pub const EXISTS: u64 = 2;
    pub const INSERT: u64 = 3;
    pub const UPDATE: u64 = 4;
    pub const DELETE: u64 = 5;
    pub const RAW: u64 = 6;
    pub const SECURITY: u64 = 7;
    pub const NONE: u64 = 10;
    pub const SOME: u64 = 11;
    pub const RAW_SQL: u64 = 12;
    pub const PARAM: u64 = 20;
    pub const COLUMN: u64 = 21;
    pub const UNARY: u64 = 22;
    pub const BINARY: u64 = 23;
    pub const IN: u64 = 24;
    pub const JSON_PATH: u64 = 25;
    pub const CAST: u64 = 26;
    pub const RAW_OP: u64 = 27;
    pub const CALL: u64 = 28;
    pub const DSL: u64 = 29;
    pub const KEY: u64 = 30;
    pub const INDEX: u64 = 31;
}

const CAPABILITIES: [Capability; 7] = [
    Capability::Ilike,
    Capability::JsonPath,
    Capability::JsonContainment,
    Capability::Returning,
    Capability::Savepoints,
    Capability::NativeRls,
    Capability::ReadOnlyTransactions,
];

impl<'a, 'b> Shape<'a, 'b> {
    /// Starts a shape with the dialect: its id picks the SQL syntax, and its
    /// capabilities decide what renders at all.
    fn new(dialect: &dyn Dialect, tokens: &'b mut Vec<u64>, recording: bool) -> Self {
        let caps = CAPABILITIES
            .iter()
            .enumerate()
            .filter(|(_, c)| dialect.supports(**c))
            .fold(0u64, |mask, (i, _)| mask | (1 << i));
        tokens.clear();
        tokens.push(dialect.id() as u64);
        tokens.push(caps);
        Self {
            tokens,
            values: Vec::new(),
            refs: Vec::new(),
            recording,
            saw_dsl: false,
        }
    }

    fn push(&mut self, token: u64) {
        self.tokens.push(token);
    }

    fn len(&mut self, n: usize) {
        self.push(n as u64);
    }

    /// A static string, by address and length: equal tokens mean the same
    /// string (identical text at two addresses only costs a duplicate entry).
    fn text(&mut self, s: &'static str) {
        self.push(s.as_ptr() as u64);
        self.len(s.len());
    }

    fn param(&mut self, v: &'a Value) {
        self.push(tag::PARAM);
        if self.recording {
            self.refs.push(v);
        } else {
            self.values.push(v.clone());
        }
    }

    fn statement(&mut self, statement: &'a Statement) {
        match statement {
            Statement::Select(s) => {
                self.push(tag::SELECT);
                self.select(s);
            }
            Statement::Exists(s) => {
                self.push(tag::EXISTS);
                self.select(s);
            }
            Statement::Insert(i) => {
                self.push(tag::INSERT);
                self.table(i.table);
                self.len(i.columns.len());
                for c in &i.columns {
                    self.text(c);
                }
                self.len(i.rows.len());
                for row in &i.rows {
                    self.exprs(row);
                }
                match &i.returning {
                    None => self.push(tag::NONE),
                    Some(cols) => {
                        self.push(tag::SOME);
                        self.columns(cols);
                    }
                }
            }
            Statement::Update(u) => {
                self.push(tag::UPDATE);
                self.table(u.table);
                self.len(u.set.len());
                for (column, value) in &u.set {
                    self.text(column);
                    self.expr(value);
                }
                self.opt_expr(u.filter.as_ref());
            }
            Statement::Delete(d) => {
                self.push(tag::DELETE);
                self.table(d.table);
                self.opt_expr(d.filter.as_ref());
            }
            Statement::Raw(raw) => {
                self.push(tag::RAW);
                self.len(raw.parts.len());
                for part in &raw.parts {
                    match part {
                        RawPart::Sql(s) => {
                            self.push(tag::RAW_SQL);
                            self.text(s);
                        }
                        RawPart::Param(v) => self.param(v),
                    }
                }
            }
            // Never cached (see the module docs); a tag keeps this total.
            Statement::ApplySecurity(_) => self.push(tag::SECURITY),
        }
    }

    fn select(&mut self, s: &'a Select) {
        match &s.from {
            FromClause::Table(t) => self.table(*t),
        }
        self.columns(&s.projection);
        self.opt_expr(s.filter.as_ref());
        self.len(s.order_by.len());
        for OrderBy { expr, direction } in &s.order_by {
            self.push(*direction as u64);
            self.expr(expr);
        }
        self.opt_expr(s.limit.as_ref());
        self.opt_expr(s.offset.as_ref());
    }

    fn table(&mut self, t: TableRef) {
        match t.schema {
            None => self.push(tag::NONE),
            Some(schema) => {
                self.push(tag::SOME);
                self.text(schema);
            }
        }
        self.text(t.name);
    }

    fn column(&mut self, c: ColumnRef) {
        self.push(u64::from(c.source.0));
        self.text(c.name);
    }

    fn columns(&mut self, cols: &[ColumnRef]) {
        self.len(cols.len());
        for c in cols {
            self.column(*c);
        }
    }

    fn opt_expr(&mut self, e: Option<&'a ExprNode>) {
        match e {
            None => self.push(tag::NONE),
            Some(e) => {
                self.push(tag::SOME);
                self.expr(e);
            }
        }
    }

    fn exprs(&mut self, list: &'a [ExprNode]) {
        self.len(list.len());
        for e in list {
            self.expr(e);
        }
    }

    fn expr(&mut self, node: &'a ExprNode) {
        match node {
            ExprNode::Column(c) => {
                self.push(tag::COLUMN);
                self.column(*c);
            }
            ExprNode::Param(v) => self.param(v),
            ExprNode::Unary(op, e) => {
                self.push(tag::UNARY);
                self.push(*op as u64);
                self.expr(e);
            }
            ExprNode::Binary(op, l, r) => {
                self.push(tag::BINARY);
                self.push(*op as u64);
                self.expr(l);
                self.expr(r);
            }
            ExprNode::In {
                expr,
                list,
                negated,
            } => {
                self.push(tag::IN);
                self.push(u64::from(*negated));
                self.expr(expr);
                self.exprs(list);
            }
            ExprNode::JsonPath {
                column,
                path,
                as_text,
            } => {
                self.push(tag::JSON_PATH);
                self.column(*column);
                self.push(u64::from(*as_text));
                self.len(path.len());
                for seg in path {
                    match seg {
                        PathSeg::Key(k) => {
                            self.push(tag::KEY);
                            self.text(k);
                        }
                        PathSeg::Index(i) => {
                            self.push(tag::INDEX);
                            self.push(u64::from(*i));
                        }
                    }
                }
            }
            ExprNode::Cast(e, ty) => {
                self.push(tag::CAST);
                self.push(*ty as u64);
                self.expr(e);
            }
            ExprNode::RawOp(op, l, r) => {
                self.push(tag::RAW_OP);
                self.text(op);
                self.expr(l);
                self.expr(r);
            }
            ExprNode::Call(name, args) => {
                self.push(tag::CALL);
                self.text(name);
                self.exprs(args);
            }
            // The caller lowers DSL calls and shapes again; the tokens still
            // keep this shape distinct.
            ExprNode::Dsl(call) => {
                self.saw_dsl = true;
                self.push(tag::DSL);
                self.push(std::ptr::from_ref(call.def) as u64);
                self.exprs(&call.args);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DSL lowering ahead of shaping
// ---------------------------------------------------------------------------

/// `statement` with every DSL call lowered for `dialect`, recursively.
fn lower_statement(statement: &Statement, dialect: &dyn Dialect) -> Result<Statement, RenderError> {
    let mut statement = statement.clone();
    let lower = |e: &mut ExprNode| lower_expr(e, dialect, 0);
    let lower_opt = |e: &mut Option<ExprNode>| e.as_mut().map_or(Ok(()), lower);
    match &mut statement {
        Statement::Select(s) | Statement::Exists(s) => {
            lower_opt(&mut s.filter)?;
            lower_opt(&mut s.limit)?;
            lower_opt(&mut s.offset)?;
            for o in &mut s.order_by {
                lower(&mut o.expr)?;
            }
        }
        Statement::Insert(i) => {
            for e in i.rows.iter_mut().flatten() {
                lower(e)?;
            }
        }
        Statement::Update(u) => {
            for (_, e) in &mut u.set {
                lower(e)?;
            }
            lower_opt(&mut u.filter)?;
        }
        Statement::Delete(d) => lower_opt(&mut d.filter)?,
        Statement::Raw(_) | Statement::ApplySecurity(_) => {}
    }
    Ok(statement)
}

fn lower_expr(e: &mut ExprNode, dialect: &dyn Dialect, depth: usize) -> Result<(), RenderError> {
    match e {
        ExprNode::Dsl(call) => {
            if depth >= MAX_LOWERING_DEPTH {
                return Err(RenderError::TooDeep);
            }
            let mut lowered = call.lower(dialect)?;
            lower_expr(&mut lowered, dialect, depth + 1)?;
            *e = lowered;
        }
        ExprNode::Column(_) | ExprNode::Param(_) | ExprNode::JsonPath { .. } => {}
        ExprNode::Unary(_, inner) | ExprNode::Cast(inner, _) => lower_expr(inner, dialect, depth)?,
        ExprNode::Binary(_, l, r) | ExprNode::RawOp(_, l, r) => {
            lower_expr(l, dialect, depth)?;
            lower_expr(r, dialect, depth)?;
        }
        ExprNode::In { expr, list, .. } => {
            lower_expr(expr, dialect, depth)?;
            for item in list {
                lower_expr(item, dialect, depth)?;
            }
        }
        ExprNode::Call(_, args) => {
            for arg in args {
                lower_expr(arg, dialect, depth)?;
            }
        }
    }
    Ok(())
}
