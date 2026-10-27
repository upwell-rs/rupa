//! Runs the example Upwell application's command line.
//!
//! ```text
//! cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate apply
//! cargo run -p rupa-example-upwell -- reporting status --url reporting.db --format json
//! ```

use std::process::ExitCode;

use rupa::upwell::migrations::exit_code;
use rupa_example_upwell::Example;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    exit_code(Example::run().await)
}
