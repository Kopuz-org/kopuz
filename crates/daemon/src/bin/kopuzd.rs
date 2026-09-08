//! Headless Kopuz daemon. The startup path lives in `daemon::boot` so the
//! desktop app can host the same daemon in a child process of its own
//! executable; this binary is the standalone entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use daemon::boot::BootArgs;

fn parse_args() -> Result<BootArgs, String> {
    let mut args = BootArgs::default();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--socket" => {
                args.socket = Some(PathBuf::from(
                    iter.next().ok_or("--socket requires a path")?,
                ));
            }
            "--supervised" => args.supervised = true,
            "--db-path" => {
                args.db_path = Some(iter.next().ok_or("--db-path requires a path")?);
            }
            "--help" | "-h" => {
                return Err(
                    "usage: kopuzd [--socket <path>] [--db-path <file>] [--supervised]".to_string(),
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let _log_guard = daemon::boot::init_logging();

    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };
    match daemon::boot::block_on_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "kopuzd exited with an error");
            ExitCode::FAILURE
        }
    }
}
