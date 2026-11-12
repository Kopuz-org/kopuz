//! One app per daemon socket takes `kopuz://` links: a later launch hands its links to it and exits.

use std::path::{Path, PathBuf};

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::App;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

use crate::{Address, Link};

/// Far more than a launch's links need, so a stray writer cannot fill memory.
const MAX_BYTES: u64 = 64 * 1024;

/// Where the app listens: beside the daemon socket it uses, so flavours and dev runs never share one.
pub fn path(address: &Address) -> Option<PathBuf> {
    let socket = match address {
        Address::Socket(path) => path.clone(),
        Address::Tcp(_) => crate::default_socket_path()?,
    };
    let mut name = socket.into_os_string();
    name.push(".app");
    Some(name.into())
}

/// Hand `links` to the app listening at `path`; false when none is, and this launch opens them itself.
pub fn send(path: &Path, links: &[String]) -> bool {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    runtime.block_on(async {
        let Ok(mut stream) = connect(path).await else {
            return false;
        };
        stream.write_all(links.join("\n").as_bytes()).await.is_ok()
            && stream.shutdown().await.is_ok()
    })
}

/// Take the links later launches send, unless another app at `path` already does.
pub fn listen(path: PathBuf, links: UnboundedSender<String>, cx: &App) {
    Link::global(cx).runtime.spawn(async move {
        if let Err(error) = serve(&path, links).await {
            tracing::warn!(%error, path = %path.display(), "not taking links from later launches");
        }
    });
}

#[cfg(unix)]
async fn connect(path: &Path) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(path).await
}

#[cfg(windows)]
async fn connect(path: &Path) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    proto::pipe::connect(path).await
}

#[cfg(unix)]
async fn serve(path: &Path, links: UnboundedSender<String>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    if connect(path).await.is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "another app already takes them",
        ));
    }
    // A socket file nobody answers on was left by an app that did not exit cleanly.
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(read(stream, links.clone()));
    }
}

#[cfg(windows)]
async fn serve(path: &Path, links: UnboundedSender<String>) -> std::io::Result<()> {
    use futures::StreamExt as _;

    let incoming = proto::pipe::Listener::bind(path)?.incoming();
    futures::pin_mut!(incoming);
    while let Some(stream) = incoming.next().await {
        tokio::spawn(read(stream?, links.clone()));
    }
    Ok(())
}

async fn read(stream: impl AsyncRead + Unpin, links: UnboundedSender<String>) {
    let mut text = Vec::new();
    // A pipe reports its writer closing as an error, after the bytes it wrote.
    let _ = stream.take(MAX_BYTES).read_to_end(&mut text).await;
    for link in String::from_utf8_lossy(&text).lines() {
        if !link.is_empty() {
            let _ = links.unbounded_send(link.to_string());
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use futures::StreamExt as _;

    use super::*;

    #[test]
    fn a_later_launch_hands_its_links_to_the_one_listening() {
        let path = std::env::temp_dir().join(format!("kopuz-relay-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(!send(&path, &["kopuz://open/home".to_string()]));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (links, mut taken) = futures::channel::mpsc::unbounded();
        let served = path.clone();
        runtime.spawn(async move { serve(&served, links).await });
        while !path.exists() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let sent = [
            "kopuz://open/home".to_string(),
            "kopuz://search?q=one".to_string(),
        ];
        assert!(send(&path, &sent));
        let got = runtime.block_on(async { (taken.next().await, taken.next().await) });
        assert_eq!(got, (Some(sent[0].clone()), Some(sent[1].clone())));

        // A second listener leaves the first in place, links and all.
        let (other, _kept) = futures::channel::mpsc::unbounded();
        let refused = runtime.block_on(serve(&path, other)).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::AddrInUse);
        std::fs::remove_file(&path).unwrap();
    }
}
