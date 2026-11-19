use super::*;
use crate::expr::ExprOps;
use crate::schema::{ColumnDef, IndexColumn};

#[test]
fn a_table_collects_checks_and_uniques() {
    let table = docs()
        .check(Check::on::<Doc>("c", Doc::OWNER.gt(0_i64)))
        .unique(Unique::on("u", Doc::OWNER).and(Doc::SLUG));
    let names: Vec<_> = table.constraints.iter().map(Constraint::name).collect();

    assert_eq!(names, ["c", "u"]);
}

#[test]
fn a_constraint_replaces_one_of_the_same_name() {
    let table = docs()
        .unique(Unique::on("u", Doc::OWNER))
        .unique(Unique::on("u", Doc::SLUG));

    assert_eq!(table.constraints.len(), 1);
    assert_eq!(
        table.constraints[0],
        Constraint::Unique(Unique::on("u", Doc::SLUG).build())
    );
}

#[test]
fn a_new_table_has_no_constraints() {
    assert!(docs().constraints.is_empty());
}

#[test]
fn removing_a_column_removes_the_uniques_over_it_only() {
    let table = docs()
        .column(ColumnDef::new("owner", ColumnType::BigInt))
        .unique(Unique::on("u_owner", Doc::OWNER).and(Doc::SLUG))
        .unique(Unique::new(
            "u_title",
            [IndexColumn::new("title", ColumnType::Text)],
        ))
        .check(Check::on::<Doc>("c", Doc::OWNER.gt(0_i64)))
        .without("owner");
    let names: Vec<_> = table.constraints.iter().map(Constraint::name).collect();

    assert_eq!(names, ["u_title", "c"]);
}
