//! Canonical records for table constraints.

use rupa_core::schema::{Check, Constraint, ConstraintKind, Unique};

use super::Encoder;

impl Encoder<'_> {
    /// What a table adds to its record when it has constraints: their count
    /// and each constraint. A table without any adds nothing, so it encodes
    /// as it did before constraints existed.
    pub(super) fn table_constraints(&mut self, constraints: &[Constraint]) {
        if constraints.is_empty() {
            return;
        }

        self.str("constraints");
        self.len(constraints.len());

        for constraint in constraints {
            self.constraint(constraint);
        }
    }

    pub(super) fn constraint(&mut self, constraint: &Constraint) {
        match constraint {
            Constraint::Check(check) => self.check(check),
            Constraint::Unique(unique) => self.unique(unique),
        }
    }

    pub(super) fn constraint_kind(&mut self, kind: ConstraintKind) {
        self.str(match kind {
            ConstraintKind::Check => "check",
            ConstraintKind::Unique => "unique",
        });
    }

    fn check(&mut self, check: &Check) {
        self.constraint_kind(ConstraintKind::Check);
        self.str(check.name);
        self.ddl_expr(&check.condition);
    }

    /// Column types are not part of a constraint's identity, as for indexes.
    fn unique(&mut self, unique: &Unique) {
        self.constraint_kind(ConstraintKind::Unique);
        self.str(unique.name);
        self.names(
            &unique
                .columns
                .iter()
                .map(|column| column.name)
                .collect::<Vec<_>>(),
        );
    }
}
