//! Headless Kopuz daemon: a core from `daemon::boot`, served on a socket
//! and, with `--listen`, on a token-gated TCP port.

use std::path::PathBuf;
use std::process::ExitCode;

use kopuzd::ServeArgs;
use kopuzd::identity::{AppIdentity, validate_app_id};

const USAGE: &str = "\
usage: kopuzd [options]

  --socket <path>        socket (named pipe on Windows) to serve on
  --listen <ip>:<port>   also serve on a TCP port, gated on a token
  --token-file <path>    where the TCP token lives
  --db-path <file>       library database to open
  --app-id <id>          AppUserModelID the Windows media flyout and
                         notifications show this process as; defaults to
                         $KOPUZ_APP_ID, else none is set
  --app-name <name>      display name registered for the app id (Windows)
  --app-icon <path>      .ico or .png registered for the app id (Windows)
  -h, --help             show this help";

fn parse_args(
    argv: impl IntoIterator<Item = String>,
    env_app_id: Option<String>,
) -> Result<ServeArgs, String> {
    let mut args = ServeArgs::default();
    let mut app_id = None;
    let mut app_name = None;
    let mut app_icon = None;
    if let Some(id) = env_app_id.filter(|id| !id.is_empty()) {
        validate_app_id(&id).map_err(|error| format!("KOPUZ_APP_ID: {error}"))?;
        app_id = Some(id);
    }
    let mut iter = argv.into_iter();
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
            "--app-id" => {
                let id = iter.next().ok_or("--app-id requires an id")?;
                validate_app_id(&id)?;
                app_id = Some(id);
            }
            "--app-name" => {
                app_name = Some(iter.next().ok_or("--app-name requires a name")?);
            }
            "--app-icon" => {
                let icon = iter.next().ok_or("--app-icon requires a path")?;
                // The shell resolves IconUri on its own, not from our cwd.
                app_icon = Some(
                    std::path::absolute(&icon)
                        .map_err(|error| format!("--app-icon {icon}: {error}"))?,
                );
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    args.identity = match app_id {
        Some(id) => Some(AppIdentity {
            id,
            name: app_name,
            icon: app_icon,
        }),
        None if app_name.is_some() || app_icon.is_some() => {
            return Err("--app-name and --app-icon need an app id".to_string());
        }
        None => None,
    };
    Ok(args)
}

fn main() -> ExitCode {
    let _log_guard = kopuzd::init_logging();

    let args = match parse_args(std::env::args().skip(1), std::env::var("KOPUZ_APP_ID").ok()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str], env: Option<&str>) -> Result<ServeArgs, String> {
        parse_args(
            argv.iter().map(|arg| arg.to_string()),
            env.map(str::to_string),
        )
    }

    #[test]
    fn no_app_id_sets_none() {
        assert_eq!(parse(&[], None).unwrap().identity, None);
        assert_eq!(parse(&[], Some("")).unwrap().identity, None);
    }

    #[test]
    fn app_id_flag_beats_env() {
        let id = |args: ServeArgs| args.identity.unwrap().id;
        assert_eq!(id(parse(&[], Some("env.id")).unwrap()), "env.id");
        assert_eq!(
            id(parse(&["--app-id", "flag.id"], Some("env.id")).unwrap()),
            "flag.id"
        );
    }

    #[test]
    fn app_name_and_icon_are_taken() {
        let args = parse(
            &[
                "--app-id",
                "a.b",
                "--app-name",
                "Formal Music",
                "--app-icon",
                "icon.ico",
            ],
            None,
        )
        .unwrap();
        let identity = args.identity.unwrap();
        assert_eq!(identity.name.as_deref(), Some("Formal Music"));
        let icon = identity.icon.unwrap();
        assert!(icon.is_absolute());
        assert!(icon.ends_with("icon.ico"));
    }

    #[test]
    fn bad_app_ids_are_refused() {
        assert!(parse(&["--app-id"], None).is_err());
        assert!(parse(&["--app-id", "has space"], None).is_err());
        assert!(parse(&[], Some(r"a\b")).is_err());
        assert!(parse(&["--app-name"], None).is_err());
        assert!(parse(&["--app-name", "Kopuz"], None).is_err());
        assert!(parse(&["--app-icon", "icon.ico"], None).is_err());
    }

    #[test]
    fn help_lists_the_identity_flags() {
        let usage = parse(&["--help"], None).unwrap_err();
        for flag in ["--app-id", "--app-name", "--app-icon", "KOPUZ_APP_ID"] {
            assert!(usage.contains(flag), "{flag} missing from --help");
        }
    }
}
