use std::future::Future;
use std::time::Duration;

use smb2::{ClientConfig, SmbClient, Tree};

use crate::source::SourceError;

mod stream;
pub(crate) use stream::SmbStream;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Location {
    address: String,
    share: String,
    root: String,
}

impl Location {
    pub fn parse(value: &str) -> Result<Self, SourceError> {
        let invalid = || {
            SourceError::InvalidInput(
                "expected smb://host/share[/folder] without credentials".into(),
            )
        };
        let raw_path = value
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('/'))
            .ok_or_else(invalid)?
            .1;
        let parts: Vec<String> = raw_path
            .trim_end_matches('/')
            .split('/')
            .map(|part| {
                let decoded = percent_encoding::percent_decode_str(part)
                    .decode_utf8()
                    .map_err(|_| invalid())?;
                if !valid_name(&decoded) {
                    return Err(invalid());
                }
                Ok(decoded.into_owned())
            })
            .collect::<Result<_, _>>()?;
        let url = reqwest::Url::parse(value).map_err(|_| invalid())?;
        if url.scheme() != "smb"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err(invalid());
        }
        let host = url.host_str().ok_or_else(invalid)?.trim_matches(['[', ']']);
        let port = url.port().unwrap_or(445);
        let address = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        Ok(Self {
            address,
            share: parts[0].clone(),
            root: parts[1..].join("/"),
        })
    }

    fn path(&self, relative: &str) -> Result<String, SourceError> {
        if !relative.is_empty() && !relative.split('/').all(valid_name) {
            return Err(SourceError::InvalidInput("invalid SMB file path".into()));
        }
        Ok([self.root.as_str(), relative]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/"))
    }

    pub(crate) async fn connect(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Session, SourceError> {
        let (domain, username) = username.split_once('\\').unwrap_or(("", username));
        let mut client = call(SmbClient::connect(ClientConfig {
            addr: self.address.clone(),
            username: username.to_string(),
            password: password.to_string(),
            domain: domain.to_string(),
            timeout: Duration::from_secs(10),
            dfs_enabled: false,
            ..Default::default()
        }))
        .await?;
        client.connection().set_response_timeout(Some(TIMEOUT));
        client.connection().set_send_timeout(Some(TIMEOUT));
        let tree = call(client.connect_share(&self.share)).await?;
        Ok(Session {
            client,
            tree,
            location: self.clone(),
        })
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

pub(crate) struct Session {
    client: SmbClient,
    tree: Tree,
    location: Location,
}

impl Session {
    pub(crate) async fn list(
        &mut self,
        path: &str,
    ) -> Result<Vec<smb2::DirectoryEntry>, SourceError> {
        let path = self.location.path(path)?;
        let entries = call(self.client.list_directory(&mut self.tree, &path)).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| valid_name(&entry.name))
            .collect())
    }

    pub(crate) async fn open(&mut self, path: &str) -> Result<SmbStream, SourceError> {
        let path = self.location.path(path)?;
        let reader = call(self.client.open_file_reader(&self.tree, &path)).await?;
        Ok(SmbStream::new(reader))
    }
}

pub async fn login(url: &str, username: &str, password: &str) -> Result<(), SourceError> {
    Location::parse(url)?
        .connect(username, password)
        .await?
        .list("")
        .await?;
    Ok(())
}

async fn call<T>(future: impl Future<Output = Result<T, smb2::Error>>) -> Result<T, SourceError> {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .map_err(|_| SourceError::Connectivity)?
        .map_err(|error| {
            use smb2::ErrorKind;
            match error.kind() {
                ErrorKind::AuthRequired | ErrorKind::SessionExpired => SourceError::Auth,
                ErrorKind::ConnectionLost | ErrorKind::TimedOut | ErrorKind::Io => {
                    SourceError::Connectivity
                }
                ErrorKind::AccessDenied => {
                    SourceError::Backend("SMB access denied for this share or file".into())
                }
                ErrorKind::NotFound => SourceError::Backend("SMB share or file not found".into()),
                _ => SourceError::Backend("SMB request failed".into()),
            }
        })
}

#[cfg(test)]
mod tests;
