use std::io::{self, Read, Write};
use std::path::PathBuf;

use api::{ApiError, FieldValue, KopuzApi, PlayerCommand};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "kopuzctl",
    version,
    about = "Control a running Kopuz app or daemon"
)]
pub struct Cli {
    /// Unix socket or Windows pipe to connect to.
    #[arg(long, global = true, env = "KOPUZ_SOCKET", conflicts_with = "address")]
    pub socket: Option<PathBuf>,
    /// TCP host:port served by kopuzd --listen.
    #[arg(long, global = true, requires = "token_file")]
    pub address: Option<String>,
    /// File containing the daemon's TCP bearer token.
    #[arg(long, global = true, requires = "address")]
    pub token_file: Option<PathBuf>,
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Maximum seconds to wait for the daemon, including browser sign-in.
    #[arg(long, global = true, default_value = "300", value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: u64,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show daemon and playback status.
    Status,
    /// Resume playback, or replace the queue with library track keys.
    Play { keys: Vec<String> },
    /// Pause playback.
    Pause,
    /// Toggle playback.
    Toggle,
    /// Stop playback; the daemon keeps running.
    Stop,
    /// Play the next track.
    Next,
    /// Play the previous track.
    Previous,
    /// Seek to an absolute position in seconds.
    Seek { seconds: u64 },
    /// Set volume from 0 to 100.
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// List a window of the playback queue.
    Queue(PageArgs),
    /// Search the active source's indexed library.
    Search {
        query: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// List services or describe a service's setup fields.
    Services { id: Option<String> },
    /// Configure and select music sources.
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    /// List or manage background library jobs.
    Jobs {
        #[command(subcommand)]
        command: Option<JobCommand>,
    },
}

#[derive(Args)]
pub struct PageArgs {
    #[arg(long, default_value = "0")]
    pub offset: u32,
    #[arg(long, default_value = "50", value_parser = clap::value_parser!(u32).range(1..=200))]
    pub limit: u32,
}

impl From<PageArgs> for api::Page {
    fn from(value: PageArgs) -> Self {
        Self {
            offset: value.offset,
            limit: value.limit,
        }
    }
}

#[derive(Subcommand)]
pub enum SourceCommand {
    /// List configured sources and their IDs.
    List,
    /// Create a source using fields published by the daemon.
    Add {
        service: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "field", value_parser = parse_field)]
        fields: Vec<FieldValue>,
        /// Read one write-only setup field from standard input.
        #[arg(long, value_name = "FIELD")]
        secret_stdin: Option<String>,
        #[arg(long)]
        activate: bool,
    },
    /// Sign in with a password, or open browser sign-in when username is omitted.
    Login {
        id: String,
        #[arg(long)]
        username: Option<String>,
        #[arg(long, requires = "username")]
        password_stdin: bool,
    },
    /// Store a token obtained elsewhere, without printing it.
    Token {
        id: String,
        #[arg(long)]
        user_id: Option<String>,
        #[arg(long)]
        secret_stdin: bool,
    },
    /// Make a source active.
    Use { id: String },
    /// Check a source's connection and authentication.
    Check { id: String },
    /// Remove stored credentials from a source.
    Logout { id: String },
    /// Delete a configured source.
    Remove { id: String },
}

#[derive(Subcommand)]
pub enum JobCommand {
    /// Start a library scan or synchronization.
    Start {
        #[arg(value_enum)]
        kind: Job,
    },
    /// Cancel a background job.
    Cancel { id: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Job {
    Scan,
    LibrarySync,
    FavoritesSync,
    PlaylistSync,
}

fn parse_field(value: &str) -> Result<FieldValue, String> {
    let (key, value) = value.split_once('=').ok_or("use KEY=VALUE for --field")?;
    if key.trim().is_empty() {
        return Err("a field name cannot be empty".into());
    }
    Ok(FieldValue::new(key, value))
}

impl Command {
    pub fn secret_input(&self) -> Option<bool> {
        match self {
            Self::Source {
                command:
                    SourceCommand::Add {
                        secret_stdin: Some(_),
                        ..
                    },
            } => Some(true),
            Self::Source {
                command:
                    SourceCommand::Login {
                        username: Some(_),
                        password_stdin,
                        ..
                    },
            } => Some(*password_stdin),
            Self::Source {
                command: SourceCommand::Token { secret_stdin, .. },
            } => Some(*secret_stdin),
            _ => None,
        }
    }
}

pub fn read_secret(input: impl Read) -> io::Result<String> {
    let mut bytes = Vec::new();
    input.take(65_537).read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secret exceeds 64 KiB",
        ));
    }
    let mut secret = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "secret must be UTF-8"))?;
    if secret.ends_with('\n') {
        secret.pop();
        if secret.ends_with('\r') {
            secret.pop();
        }
    }
    if secret.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secret cannot be empty",
        ));
    }
    Ok(secret)
}

fn source_row(source: api::SourceInfo) -> Value {
    json!({
        "id": source.id,
        "name": source.name,
        "service": source.service.id,
        "active": source.active,
        "authenticated": source.authenticated,
        "state": source.state.map(|state| format!("{state:?}").to_lowercase()),
    })
}

fn track_row(track: api::TrackInfo) -> Value {
    json!({
        "key": track.key,
        "title": track.title,
        "artist": track.artist,
        "album": track.album,
        "duration_ms": track.duration_ms,
    })
}

fn secret(value: Option<String>) -> Result<String, ApiError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::invalid_input("a nonempty secret is required"))
}

pub async fn execute(
    api: &dyn KopuzApi,
    command: Command,
    input: Option<String>,
) -> Result<Value, ApiError> {
    let status = match api.handshake().await? {
        api::Handshake::Ready(status) => status,
        api::Handshake::Mismatched { daemon, client } => {
            return Err(ApiError::unsupported(format!(
                "protocol mismatch: daemon {daemon}, client {client}; install matching Kopuz versions"
            )));
        }
    };
    let player_command = match command {
        Command::Pause => Some(PlayerCommand::Pause),
        Command::Toggle => Some(PlayerCommand::Toggle),
        Command::Stop => Some(PlayerCommand::Stop),
        Command::Next => Some(PlayerCommand::Next),
        Command::Previous => Some(PlayerCommand::Previous),
        Command::Seek { seconds } => Some(PlayerCommand::Seek {
            position_ms: seconds
                .checked_mul(1000)
                .ok_or_else(|| ApiError::invalid_input("seek position is too large"))?,
        }),
        Command::Volume { percent } => Some(PlayerCommand::SetVolume {
            volume: f32::from(percent) / 100.0,
        }),
        Command::Play { ref keys } if keys.is_empty() => Some(PlayerCommand::Play),
        _ => None,
    };
    if let Some(command) = player_command {
        return Ok(json!({"revision": api.player_command(command).await?.rev}));
    }
    match command {
        Command::Status => {
            let state = api.player_state().await?;
            Ok(json!({
                "version": status.version,
                "uptime_seconds": status.uptime_secs,
                "protocol_revision": status.proto_revision,
                "phase": format!("{:?}", state.phase).to_lowercase(),
                "volume_percent": (state.volume * 100.0).round(),
                "queue_length": state.queue.length,
                "track": state.track.map(|track| format!("{} — {}", track.artist, track.title)),
            }))
        }
        Command::Play { keys } => {
            let ack = api
                .set_queue(api::SetQueueRequest {
                    mode: api::QueueMode::Replace,
                    context: api::QueueContext::Tracks { keys },
                    start_index: Some(0),
                    shuffle: None,
                })
                .await?;
            Ok(json!({"revision": ack.rev}))
        }
        Command::Queue(page) => {
            let queue = api.queue_window(page.into()).await?;
            Ok(Value::Array(
                queue
                    .items
                    .into_iter()
                    .map(|item| {
                        let mut row = track_row(item.track);
                        row["index"] = json!(item.index);
                        row
                    })
                    .collect(),
            ))
        }
        Command::Search { query, page } => {
            let tracks = api
                .tracks(
                    api::TrackFilter {
                        search: Some(query),
                        ..Default::default()
                    },
                    page.into(),
                )
                .await?;
            Ok(Value::Array(
                tracks.items.into_iter().map(track_row).collect(),
            ))
        }
        Command::Services { id } => {
            let services = api.services().await?;
            if let Some(id) = id {
                let service = services
                    .into_iter()
                    .find(|service| service.id == id)
                    .ok_or_else(|| ApiError::not_found("no such service"))?;
                Ok(Value::Array(
                    service
                        .fields
                        .into_iter()
                        .map(|field| {
                            let choices: Vec<_> = match &field.kind {
                                api::FieldKind::Choice { options, .. }
                                | api::FieldKind::Radio { options } => {
                                    options.iter().map(|option| option.value.clone()).collect()
                                }
                                _ => Vec::new(),
                            };
                            json!({
                                "key": field.key,
                                "required": field.required,
                                "secret": matches!(field.kind, api::FieldKind::Secret),
                                "default": field.value,
                                "choices": choices,
                            })
                        })
                        .collect(),
                ))
            } else {
                Ok(Value::Array(
                    services
                        .into_iter()
                        .map(|service| {
                            json!({
                                "id": service.id,
                                "experimental": service.experimental,
                            })
                        })
                        .collect(),
                ))
            }
        }
        Command::Source { command } => execute_source(api, command, input).await,
        Command::Jobs { command } => match command {
            None => Ok(Value::Array(
                api.jobs()
                    .await?
                    .into_iter()
                    .map(|job| {
                        json!({
                            "id": job.id,
                            "kind": format!("{:?}", job.kind),
                            "state": format!("{:?}", job.state),
                            "phase": job.phase,
                            "current": job.current,
                            "total": job.total,
                            "error": job.error.map(|error| error.message),
                        })
                    })
                    .collect(),
            )),
            Some(JobCommand::Cancel { id }) => {
                api.cancel_job(id).await?;
                Ok(json!({"ok": true}))
            }
            Some(JobCommand::Start { kind }) => {
                let kind = match kind {
                    Job::Scan => api::JobKind::Scan,
                    Job::LibrarySync => api::JobKind::LibrarySync,
                    Job::FavoritesSync => api::JobKind::FavoritesSync,
                    Job::PlaylistSync => api::JobKind::PlaylistSync,
                };
                Ok(json!({"job_id": api.start_job(kind).await?.job_id}))
            }
        },
        _ => Err(ApiError::internal("unhandled playback command")),
    }
}

async fn execute_source(
    api: &dyn KopuzApi,
    command: SourceCommand,
    input: Option<String>,
) -> Result<Value, ApiError> {
    match command {
        SourceCommand::List => Ok(Value::Array(
            api.sources().await?.into_iter().map(source_row).collect(),
        )),
        SourceCommand::Add {
            service,
            name,
            url,
            mut fields,
            secret_stdin,
            activate,
        } => {
            if let Some(url) = url {
                if fields.iter().any(|field| field.key == "url") {
                    return Err(ApiError::invalid_input(
                        "use either --url or --field url=VALUE",
                    ));
                }
                fields.push(FieldValue::new("url", url));
            }
            let offered = api
                .services()
                .await?
                .into_iter()
                .find(|entry| entry.id == service)
                .ok_or_else(|| ApiError::not_found("no such service; use kopuzctl services"))?;
            let mut seen = std::collections::HashSet::new();
            for field in &fields {
                let spec = offered
                    .fields
                    .iter()
                    .find(|spec| spec.key == field.key)
                    .ok_or_else(|| {
                        ApiError::invalid_input(format!("unknown setup field: {}", field.key))
                    })?;
                if matches!(spec.kind, api::FieldKind::Secret) {
                    return Err(ApiError::invalid_input(
                        "use --secret-stdin for secret fields",
                    ));
                }
                if !seen.insert(field.key.clone()) {
                    return Err(ApiError::invalid_input("duplicate setup field"));
                }
            }
            let mut secrets = Vec::new();
            if let Some(key) = secret_stdin {
                if !offered
                    .fields
                    .iter()
                    .any(|field| field.key == key && matches!(field.kind, api::FieldKind::Secret))
                {
                    return Err(ApiError::invalid_input(
                        "this service has no such secret field",
                    ));
                }
                secrets.push(FieldValue::new(key, secret(input)?));
            }
            for field in offered.fields {
                if !seen.contains(&field.key)
                    && !matches!(field.kind, api::FieldKind::Secret | api::FieldKind::Note)
                    && let Some(value) = field.value
                {
                    fields.push(FieldValue::new(field.key, value));
                }
            }
            let source = api
                .upsert_source(api::SourceDraft {
                    id: None,
                    name,
                    service,
                    values: fields,
                    secrets,
                })
                .await?;
            let source = if activate {
                api.switch_source(source.id).await?
            } else {
                source
            };
            Ok(source_row(source))
        }
        SourceCommand::Login { id, username, .. } => {
            let source = if let Some(username) = username {
                api.login_source(api::SourceLoginRequest {
                    server_id: id,
                    username,
                    password: secret(input)?,
                })
                .await?
            } else {
                api.authenticate_source(id).await?
            };
            Ok(source_row(source))
        }
        SourceCommand::Token { id, user_id, .. } => Ok(source_row(
            api.provision_credentials(api::CredentialProvision {
                server_id: id,
                secret: secret(input)?,
                user_id,
                browser: None,
            })
            .await?,
        )),
        SourceCommand::Use { id } => Ok(source_row(api.switch_source(id).await?)),
        SourceCommand::Check { id } => {
            Ok(json!({"state": format!("{:?}", api.validate_source(id).await?).to_lowercase()}))
        }
        SourceCommand::Logout { id } => {
            api.clear_credentials(id).await?;
            Ok(json!({"ok": true}))
        }
        SourceCommand::Remove { id } => {
            api.delete_source(id).await?;
            Ok(json!({"ok": true}))
        }
    }
}

fn cell(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    };
    text.chars()
        .flat_map(|character| {
            if character.is_control() {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

pub fn write_output(mut output: impl Write, value: &Value, json: bool) -> io::Result<()> {
    if json {
        return writeln!(output, "{value}");
    }
    match value {
        Value::Array(rows) if rows.is_empty() => writeln!(output, "No entries."),
        Value::Array(rows) => {
            let Some(first) = rows[0].as_object() else {
                return writeln!(output, "{value}");
            };
            let keys: Vec<_> = first.keys().collect();
            writeln!(
                output,
                "{}",
                keys.iter()
                    .map(|key| key.to_uppercase())
                    .collect::<Vec<_>>()
                    .join("\t")
            )?;
            for row in rows {
                writeln!(
                    output,
                    "{}",
                    keys.iter()
                        .map(|key| cell(&row[key.as_str()]))
                        .collect::<Vec<_>>()
                        .join("\t")
                )?;
            }
            Ok(())
        }
        Value::Object(row) => {
            for (key, value) in row {
                writeln!(output, "{key}: {}", cell(value))?;
            }
            Ok(())
        }
        other => writeln!(output, "{}", cell(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_input_preserves_spaces_but_strips_a_line_ending() {
        assert_eq!(read_secret(&b"  secret  \r\n"[..]).unwrap(), "  secret  ");
        assert!(read_secret(&b"\n"[..]).is_err());
        assert!(read_secret(&vec![b'x'; 65_537][..]).is_err());
    }

    #[test]
    fn invalid_options_fail_before_connecting() {
        for args in [
            vec!["kopuzctl", "volume", "101"],
            vec!["kopuzctl", "--address", "localhost:9000", "status"],
            vec!["kopuzctl", "source", "login", "id", "--password-stdin"],
            vec!["kopuzctl", "queue", "--limit", "0"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        assert_eq!(
            Cli::try_parse_from(["kopuzctl", "--help"])
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::DisplayHelp
        );
    }

    #[test]
    fn output_escapes_terminal_controls_and_keeps_json_lossless() {
        let value = json!({"name": "a\n\u{1b}[31m"});
        let mut human = Vec::new();
        write_output(&mut human, &value, false).unwrap();
        assert!(!human.contains(&27));
        assert_eq!(human.iter().filter(|byte| **byte == b'\n').count(), 1);
        let mut machine = Vec::new();
        write_output(&mut machine, &value, true).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&machine).unwrap(), value);
    }
}
