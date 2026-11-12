use rupa::migrate::{Error, Planned, RunError, ValidationError};
use rupa::prelude::*;
use rupa::sql::{RenderError, SchemaError};
use rupa::{DialectId, insert, select};

use super::support::{dialect, planned};
use super::{Harness, Session, catalog};
use crate::migrations::fixture::{
    BigText, KindNote, NewKindNote, Plans, create_big_text_table, create_tsvector_table,
};

/// A kind with a different type name per dialect applies everywhere, and the
/// column takes values.
pub(super) fn case_a_kind_stored_everywhere_applies_on_every_dialect<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(planned::<H>("1"), create_big_text_table))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a kind every dialect stores applies");

    h.execute("INSERT INTO kind_notes (body) VALUES ('long text')");

    assert_eq!(
        h.count("SELECT count(*) FROM kind_notes WHERE body = 'long text'"),
        1
    );
}

/// A kind whose `sql_type` every driver binds straight into the native type
/// round-trips through an entity: insert, filter by equality, select.
pub(super) fn case_a_kind_round_trips_through_an_entity_on_every_dialect<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(planned::<H>("1"), create_big_text_table))];
    let rows = [
        NewKindNote {
            body: BigText("long text".into()),
        },
        NewKindNote {
            body: BigText("other".into()),
        },
    ];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a kind every dialect stores applies");

    let session = h.connect();
    let inserted = wait!(
        h,
        insert::<KindNote>()
            .values(&rows)
            .affected()
            .execute(&session)
    )
    .expect("kind values bind");
    let found = wait!(
        h,
        select::<KindNote>()
            .filter(KindNote::BODY.eq(BigText("long text".into())))
            .all()
            .execute(&session)
    )
    .expect("kind values filter and decode");

    assert_eq!(inserted, 2);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].body, BigText("long text".into()));
}

/// A PostgreSQL-only kind applies there. Other dialects refuse it at
/// preflight, before anything is created.
pub(super) fn case_a_postgres_only_kind_fails_typed_elsewhere_before_acquisition<H: Harness>(
    h: &mut H,
) {
    let entries = [Planned(Plans(planned::<H>("1"), create_tsvector_table))];
    let result = wait!(h, h.connect().apply(&catalog(&entries)));

    if dialect(h) == DialectId::Postgres {
        result.expect("a tsvector column on PostgreSQL");

        assert_eq!(h.count("SELECT count(*) FROM kind_documents"), 0);

        return;
    }

    assert!(
        matches!(
            result,
            Err(RunError::Infrastructure {
                source: Error::Validation(ValidationError::Schema(ref error)),
                ..
            }) if matches!(
                **error,
                RenderError::Schema(ref schema) if matches!(
                    **schema,
                    SchemaError::UnsupportedType {
                        column: "terms",
                        kind: "tsvector",
                        ..
                    }
                )
            )
        ),
        "{result:?}"
    );
    assert!(!h.table_exists("_rupa_migrations"));
    assert!(!h.table_exists("kind_documents"));
}
