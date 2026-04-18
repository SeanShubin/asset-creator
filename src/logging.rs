//! Application logging. Writes to both stderr and `asset-creator.log`
//! in the working directory. The log file is overwritten on every
//! launch so it always matches the current run.

use std::fs::File;
use std::io::Write;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const LOG_FILE: &str = "asset-creator.log";

/// Set up tracing (stderr + file) and a panic hook.
/// Call before `App::new()`.
pub fn init() {
    let file = File::create(LOG_FILE).expect("failed to create log file");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,asset_creator=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(file).with_ansi(false))
        .init();

    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!("PANIC: {info}\n\n{backtrace}");
        // Append to the log file directly — tracing may not be available
        // during panic (the subscriber could be torn down).
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(LOG_FILE) {
            let _ = writeln!(f, "\n{msg}");
        }
        eprintln!("{msg}");
    }));
}
