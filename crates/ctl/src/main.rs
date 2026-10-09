use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use ctl::Cli;

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let api = match cli.address {
        Some(address) => {
            let path = cli.token_file.ok_or("--address requires --token-file")?;
            let token = ctl::read_secret(std::fs::File::open(path)?)?;
            client::GrpcApi::connect_tcp(address, &token)?
        }
        None => {
            let socket = cli
                .socket
                .or_else(client::default_socket_path)
                .ok_or("no default socket; supply --socket")?;
            client::GrpcApi::new(socket)?
        }
    };
    let secret = match cli.command.secret_input() {
        None => None,
        Some(true) => Some(ctl::read_secret(io::stdin().lock())?),
        Some(false) if io::stdin().is_terminal() => {
            Some(rpassword::prompt_password("Password or token: ")?)
        }
        Some(false) => {
            return Err("use --password-stdin or --secret-stdin when there is no terminal".into());
        }
    };
    let result = tokio::time::timeout(
        Duration::from_secs(cli.timeout),
        ctl::execute(&api, cli.command, secret),
    )
    .await
    .map_err(|_| "daemon request timed out")??;
    match ctl::write_output(io::stdout().lock(), &result, cli.json) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.map_err(Into::into),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .without_time()
        .with_target(false)
        .with_ansi(io::stderr().is_terminal())
        .init();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
