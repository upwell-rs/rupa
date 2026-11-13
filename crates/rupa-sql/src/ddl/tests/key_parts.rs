use rupa_core::DslError;
use rupa_core::ir::{DslCall, DslFnDef, ExprNode};

use super::expressions::{column, on_expression};
use super::*;

fn as_is(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Ok(args.remove(0))
}

static AS_IS: DslFnDef = DslFnDef {
    name: "as_is",
    lower: as_is,
    eval: None,
};

#[test]
fn bare_column_expressions_render_as_plain_key_parts() {
    let direct = on_expression(column("title"));
    let lowered = on_expression(ExprNode::Dsl(DslCall {
        def: &AS_IS,
        args: vec![column("title")],
    }));
    let expected = [
        r#"CREATE INDEX "notes_expr" ON "app"."notes" ("title")"#,
        "CREATE INDEX `notes_expr` ON `app`.`notes` (`title`)",
        r#"CREATE INDEX "app"."notes_expr" ON "notes" ("title")"#,
    ];

    assert_eq!(ok(&SchemaOp::CreateIndex(direct)), expected);
    assert_eq!(ok(&SchemaOp::CreateIndex(lowered)), expected);
}
