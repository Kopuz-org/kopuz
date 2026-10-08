//! Headless Kopuz daemon: a core from `daemon::boot`, served on a socket
//! and, with `--listen`, on a token-gated TCP port.

use std::path::PathBuf;
use std::process::ExitCode;

use kopuzd::ServeArgs;

fn parse_args() -> Result<ServeArgs, String> {
    let mut args = ServeArgs::default();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--socket" => {
                args.socket = Some(PathBuf::from(
                    iter.next().ok_or("--socket requires a path")?,
                ));
            }
            "--listen" => {
                let address = iter.next().ok_or("--listen requires an <ip>:<port>")?;
                args.listen = Some(
                    address
                        .parse()
                        .map_err(|error| format!("--listen {address}: {error}"))?,
                );
            }
            "--token-file" => {
                args.token_path = Some(PathBuf::from(
                    iter.next().ok_or("--token-file requires a path")?,
                ));
            }
            "--db-path" => {
                args.db_path = Some(iter.next().ok_or("--db-path requires a path")?);
            }
            "--help" | "-h" => {
                return Err("usage: kopuzd [--socket <path>] [--listen <ip>:<port>] \
                            [--token-file <path>] [--db-path <file>]"
                    .to_string());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let _log_guard = kopuzd::init_logging();

    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };
    match kopuzd::block_on_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "kopuzd exited with an error");
            ExitCode::FAILURE
        }
    }
}
