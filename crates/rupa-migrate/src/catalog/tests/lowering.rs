use rupa_core::DslError;
use rupa_core::ir::{DslCall, DslFnDef, ExprNode};
use rupa_core::{Dialect, DialectId};

use super::expressions::{call, column, filtered, on};
use super::plans::{assert_distinct_digests, plan_digest};
use super::*;

fn plain(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Ok(ExprNode::Call("lower", vec![args.remove(0)]))
}

fn trimmed(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Ok(ExprNode::Call(
        "lower",
        vec![ExprNode::Call("trim", vec![args.remove(0)])],
    ))
}

fn without_sqlite(dialect: &dyn Dialect, args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    match dialect.id() {
        DialectId::Sqlite => Err(DslError::unsupported("fold", dialect.id())),
        _ => plain(dialect, args),
    }
}

fn without_sqlite_reworded(
    dialect: &dyn Dialect,
    args: Vec<ExprNode>,
) -> Result<ExprNode, DslError> {
    match dialect.id() {
        DialectId::Sqlite => Err(DslError::Other("a different explanation".into())),
        _ => plain(dialect, args),
    }
}

fn nested(dialect: &dyn Dialect, args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Ok(ExprNode::Dsl(DslCall {
        def: &TRIMMING,
        args: vec![plain(dialect, args)?],
    }))
}

static FOLD: DslFnDef = DslFnDef {
    name: "fold",
    lower: plain,
    eval: None,
};
static FOLD_TRIMMED: DslFnDef = DslFnDef {
    name: "fold",
    lower: trimmed,
    eval: None,
};
static RENAMED: DslFnDef = DslFnDef {
    name: "renamed",
    lower: plain,
    eval: None,
};
static NO_SQLITE: DslFnDef = DslFnDef {
    name: "fold",
    lower: without_sqlite,
    eval: None,
};
static NO_SQLITE_REWORDED: DslFnDef = DslFnDef {
    name: "fold",
    lower: without_sqlite_reworded,
    eval: None,
};
static TRIMMING: DslFnDef = DslFnDef {
    name: "trimming",
    lower: trimmed,
    eval: None,
};
static NESTED: DslFnDef = DslFnDef {
    name: "nested",
    lower: nested,
    eval: None,
};

fn dsl(def: &'static DslFnDef) -> ExprNode {
    ExprNode::Dsl(DslCall {
        def,
        args: vec![column("title")],
    })
}

fn indexed(def: &'static DslFnDef) -> Plan {
    Plan::new().create_index(on(dsl(def)))
}

#[test]
fn a_changed_lowering_changes_the_checksum() {
    let variants: [fn() -> Plan; 3] = [
        || indexed(&FOLD),
        || indexed(&FOLD_TRIMMED),
        || Plan::new().create_index(on(call("lower", column("title")))),
    ];

    assert_distinct_digests(&variants, "the lowering is what the index renders as");
}

#[test]
fn same_named_functions_with_different_lowerings_differ() {
    assert_ne!(
        plan_digest(|| indexed(&FOLD)),
        plan_digest(|| indexed(&FOLD_TRIMMED))
    );
}

#[test]
fn a_functions_name_is_not_its_identity() {
    assert_eq!(
        plan_digest(|| indexed(&FOLD)),
        plan_digest(|| indexed(&RENAMED))
    );
}

#[test]
fn a_dialect_without_a_lowering_is_encoded_distinctly() {
    let variants: [fn() -> Plan; 2] = [|| indexed(&FOLD), || indexed(&NO_SQLITE)];

    assert_distinct_digests(&variants, "a missing lowering is not a lowering");
}

#[test]
fn the_reason_a_lowering_is_missing_is_not_hashed() {
    assert_eq!(
        plan_digest(|| indexed(&NO_SQLITE)),
        plan_digest(|| indexed(&NO_SQLITE_REWORDED))
    );
}

#[test]
fn calls_in_predicates_and_nested_lowerings_are_encoded_too() {
    let variants: [fn() -> Plan; 3] = [
        || Plan::new().create_index(filtered(dsl(&FOLD))),
        || Plan::new().create_index(filtered(dsl(&FOLD_TRIMMED))),
        || Plan::new().create_index(on(dsl(&NESTED))),
    ];

    assert_distinct_digests(&variants, "every position encodes its lowering");
}
