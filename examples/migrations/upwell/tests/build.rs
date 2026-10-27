//! The control for the commands tests: building the example application
//! constructs a component like their `Marker`. A separate test binary, so its
//! counter is not shared with theirs.

use std::sync::atomic::{AtomicUsize, Ordering};

use rupa_example_upwell::Example;
use upwell::component;

/// How often the application built [`Marker`].
static BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A component every build of the application constructs.
#[component(factory = build_marker)]
pub struct Marker;

async fn build_marker() -> Marker {
    BUILDS.fetch_add(1, Ordering::SeqCst);

    Marker
}

#[tokio::test]
async fn building_the_application_builds_the_marker() {
    let prepared = Example::builder()
        .expect("the generated builder constructs")
        .prepare()
        .expect("the application prepares");
    let _app = prepared.build().await.expect("the application builds");

    assert_eq!(
        BUILDS.load(Ordering::SeqCst),
        1,
        "the marker proves a build"
    );
}
