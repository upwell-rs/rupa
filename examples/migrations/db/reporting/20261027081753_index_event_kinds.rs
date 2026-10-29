//! Index event kinds.

use rupa::migrate::Error;
use rupa::migrate::sync::{Context, Migration, Session};

/// Index event kinds.
#[derive(Debug)]
pub struct IndexEventKinds;

impl<S: Session> Migration<S> for IndexEventKinds {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("CREATE INDEX events_kind ON events (kind)")
    }

    fn down(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("DROP INDEX events_kind")
    }
}
