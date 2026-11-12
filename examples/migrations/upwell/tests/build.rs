//! The control for the commands tests: building the example application
//! constructs a component like their `Marker`. A separate test binary, so its
//! counter is not shared with theirs.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};

use rupa_example_upwell::Example;
use support::{Scratch, source};
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
    let scratch = Scratch::new("build");

    let _app = Example::builder()
        .expect("the generated builder constructs")
        .config_source(source(&scratch.toml("apply")))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect("the application builds");

    assert_eq!(
        BUILDS.load(Ordering::SeqCst),
        1,
        "the marker proves a build"
    );
}
