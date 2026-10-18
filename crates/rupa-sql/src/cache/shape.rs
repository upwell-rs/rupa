//! The token stream that identifies a statement's shape.

use rupa_core::Value;
use rupa_core::dialect::{Capability, Dialect};
use rupa_core::ir::{
    ColumnRef, ExprNode, FromClause, OrderBy, PathSeg, RawPart, Select, Statement, TableRef,
};

/// Builds the token stream of a statement into a borrowed buffer, cloning
/// its parameter values in token order (a hit binds exactly those).
pub(super) struct Shape<'a, 'b> {
    tokens: &'b mut Vec<u64>,
    pub(super) values: Vec<Value>,
    /// The parameters themselves, when recording a binding plan.
    pub(super) refs: Vec<&'a Value>,
    recording: bool,
    /// Whether the statement still contains a DSL call.
    pub(super) saw_dsl: bool,
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
    /// Starts a join's sources; a single table starts with its schema token
    /// (`NONE`/`SOME`) instead, so the two never share a shape.
    pub const JOIN: u64 = 40;
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
    pub(super) fn new(dialect: &dyn Dialect, tokens: &'b mut Vec<u64>, recording: bool) -> Self {
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

    /// Never cached (see the module docs); a tag keeps this total.
    pub(super) fn statement(&mut self, statement: &'a Statement) {
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
            Statement::ApplySecurity(_) => self.push(tag::SECURITY),
        }
    }

    fn select(&mut self, s: &'a Select) {
        match &s.from {
            FromClause::Table(t) => self.table(*t),
            FromClause::Join { base, joins } => {
                self.push(tag::JOIN);
                self.table(*base);
                self.len(joins.len());
                for join in joins {
                    self.push(join.kind as u64);
                    self.table(join.table);
                    self.expr(&join.on);
                }
            }
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

    /// The caller lowers DSL calls and shapes again; the tokens still
    /// keep this shape distinct.
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
            ExprNode::Dsl(call) => {
                self.saw_dsl = true;
                self.push(tag::DSL);
                self.push(std::ptr::from_ref(call.def) as u64);
                self.exprs(&call.args);
            }
        }
    }
}
