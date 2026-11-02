//! The Chrome DevTools Protocol over `--remote-debugging-pipe`: commands on fd 3,
//! replies on fd 4 (`--remote-debugging-io-pipes` handles on Windows), each JSON
//! ended by a NUL. Cookies arrive decrypted, so no keyring prompt, and unlike a
//! debugging port the pipe is reachable only by the process that started the browser.

use std::io;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::browser::{BrowserBin, browser_command, in_flatpak, spawn_browser};
use super::store::{Cookie, host_matches_domain};

#[cfg(all(test, unix))]
use imp::BrowserEnds;
pub(crate) use imp::{Cdp, pipes};

/// A starting browser answers its first command once it is up, well inside this.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// `flatpak-spawn --host` and `flatpak run` start the browser in a process tree
/// that does not inherit the pipe ends, as may a custom command line.
pub(crate) fn pipe_reaches(bin: &BrowserBin) -> bool {
    matches!(bin, BrowserBin::Path(_)) && !in_flatpak()
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::unix::pipe;
    use tokio::process::Command;

    pub(crate) struct Cdp {
        commands: pipe::Sender,
        replies: BufReader<pipe::Receiver>,
        pub(super) next_id: u64,
    }

    pub(crate) struct BrowserEnds {
        pub(super) read: OwnedFd,
        pub(super) write: OwnedFd,
    }

    pub(crate) fn pipes() -> io::Result<(Cdp, BrowserEnds)> {
        let (browser_read, commands) = std::io::pipe()?;
        let (replies, browser_write) = std::io::pipe()?;
        let cdp = Cdp {
            commands: pipe::Sender::from_owned_fd(OwnedFd::from(commands))?,
            replies: BufReader::new(pipe::Receiver::from_owned_fd(OwnedFd::from(replies))?),
            next_id: 0,
        };
        let ends = BrowserEnds {
            read: browser_read.into(),
            write: browser_write.into(),
        };
        Ok((cdp, ends))
    }

    impl BrowserEnds {
        /// Drop `self` once the child has spawned, or the browser never sees EOF.
        pub(crate) fn attach(&self, command: &mut Command) {
            let (read, write) = (self.read.as_raw_fd(), self.write.as_raw_fd());
            command.arg("--remote-debugging-pipe");
            // SAFETY: only async-signal-safe calls between fork and exec.
            unsafe {
                command.pre_exec(move || {
                    // Either end may already sit on 3 or 4, so both move above
                    // them before the dup2s. dup2 clears close-on-exec.
                    let read = libc::fcntl(read, libc::F_DUPFD_CLOEXEC, 10);
                    let write = libc::fcntl(write, libc::F_DUPFD_CLOEXEC, 10);
                    if read < 0 || write < 0 || libc::dup2(read, 3) < 0 || libc::dup2(write, 4) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
    }

    impl Cdp {
        pub(super) async fn send(&mut self, message: &[u8]) -> io::Result<()> {
            self.commands.write_all(message).await
        }

        pub(super) async fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
            let mut buf = Vec::new();
            if self.replies.read_until(0, &mut buf).await? == 0 {
                return Ok(None);
            }
            if buf.last() == Some(&0) {
                buf.pop();
            }
            Ok(Some(buf))
        }
    }
}

/// Anonymous pipes have no async reads on Windows, so a thread reads the
/// replies and hands them over whole.
#[cfg(windows)]
mod imp {
    use std::io::{self, BufRead, Write};
    use std::os::windows::io::{AsRawHandle, OwnedHandle};

    use tokio::process::Command;
    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};

    pub(crate) struct Cdp {
        commands: std::io::PipeWriter,
        replies: mpsc::UnboundedReceiver<Vec<u8>>,
        pub(super) next_id: u64,
    }

    pub(crate) struct BrowserEnds {
        read: OwnedHandle,
        write: OwnedHandle,
    }

    pub(crate) fn pipes() -> io::Result<(Cdp, BrowserEnds)> {
        let (browser_read, commands) = std::io::pipe()?;
        let (replies, browser_write) = std::io::pipe()?;
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("kopuz-cdp".into())
            .spawn(move || {
                let mut replies = io::BufReader::new(replies);
                loop {
                    let mut buf = Vec::new();
                    match replies.read_until(0, &mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {
                            if buf.last() == Some(&0) {
                                buf.pop();
                            }
                            if sender.send(buf).is_err() {
                                return;
                            }
                        }
                    }
                }
            })?;
        let cdp = Cdp {
            commands,
            replies: receiver,
            next_id: 0,
        };
        let ends = BrowserEnds {
            read: browser_read.into(),
            write: browser_write.into(),
        };
        Ok((cdp, ends))
    }

    impl BrowserEnds {
        /// Drop `self` once the child has spawned, or the browser never sees EOF.
        pub(crate) fn attach(&self, command: &mut Command) {
            for end in [&self.read, &self.write] {
                // SAFETY: a valid handle owned by `self`.
                let made = unsafe {
                    SetHandleInformation(
                        HANDLE(end.as_raw_handle()),
                        HANDLE_FLAG_INHERIT.0,
                        HANDLE_FLAG_INHERIT,
                    )
                };
                if let Err(e) = made {
                    tracing::warn!(error = %e, "could not hand the DevTools pipe to the browser");
                }
            }
            command.arg("--remote-debugging-pipe").arg(format!(
                "--remote-debugging-io-pipes={},{}",
                self.read.as_raw_handle() as usize,
                self.write.as_raw_handle() as usize
            ));
        }
    }

    impl Cdp {
        /// Commands are a few hundred bytes, far below the pipe's buffer.
        pub(super) async fn send(&mut self, message: &[u8]) -> io::Result<()> {
            self.commands.write_all(message)
        }

        pub(super) async fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
            Ok(self.replies.recv().await)
        }
    }
}

fn encode(id: u64, session: Option<&str>, method: &str, params: Value) -> io::Result<Vec<u8>> {
    let mut message = json!({ "id": id, "method": method, "params": params });
    if let Some(session) = session {
        message["sessionId"] = session.into();
    }
    let mut message = serde_json::to_vec(&message)?;
    message.push(0);
    Ok(message)
}

fn reply_to(message: &[u8], id: u64, method: &str) -> Option<io::Result<Value>> {
    let mut reply = serde_json::from_slice::<Value>(message).ok()?;
    if reply.get("id").and_then(Value::as_u64) != Some(id) {
        return None;
    }
    if let Some(error) = reply.get("error") {
        return Some(Err(io::Error::other(format!("{method}: {error}"))));
    }
    Some(Ok(reply
        .get_mut("result")
        .map(Value::take)
        .unwrap_or_default()))
}

pub(crate) fn cookies_in(reply: &Value, domain: &str) -> Vec<Cookie> {
    reply["cookies"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|c| {
                    c["domain"]
                        .as_str()
                        .is_some_and(|host| host_matches_domain(host, domain))
                })
                .filter_map(|c| {
                    Some(Cookie {
                        domain: c["domain"].as_str()?.to_owned(),
                        name: c["name"].as_str()?.to_owned(),
                        value: c["value"].as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Cdp {
    pub(crate) async fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.call_in(None, method, params).await
    }

    async fn call_in(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> io::Result<Value> {
        let exchange = async {
            self.next_id += 1;
            let id = self.next_id;
            self.send(&encode(id, session, method, params)?).await?;
            loop {
                let Some(message) = self.next().await? else {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                };
                if let Some(reply) = reply_to(&message, id, method) {
                    return reply;
                }
            }
        };
        tokio::time::timeout(CALL_TIMEOUT, exchange)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("{method}: no reply")))?
    }

    /// Read from the cookie manager, so a fresh sign-in shows before it reaches disk.
    pub(crate) async fn cookies(&mut self, domain: &str) -> io::Result<Vec<Cookie>> {
        let reply = self.call("Storage.getCookies", json!({})).await?;
        Ok(cookies_in(&reply, domain))
    }

    /// Quitting also writes the cookies to disk.
    pub(crate) async fn close(&mut self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.call("Browser.close", json!({})),
        )
        .await;
    }
}

pub(crate) async fn read_headless(
    bin: &BrowserBin,
    user_data_dir: &Path,
    args: &[String],
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    let (mut cdp, ends) = pipes().map_err(|e| format!("DevTools pipe: {e}"))?;
    let mut command = browser_command(bin);
    command
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .args(args);
    ends.attach(&mut command);
    command.arg("about:blank");
    // Same reason as the sign-in window: leave kopuz's WebView2 job object.
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0100_0000);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = spawn_browser(&mut command).map_err(|e| format!("spawn {bin}: {e}"))?;
    drop(ends);
    let cookies = cdp.cookies(domain).await;
    cdp.close().await;
    if tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    cookies.map_err(|e| format!("read cookies over DevTools: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_one_json_message_ended_by_nul() {
        let message = encode(7, None, "Storage.getCookies", json!({})).expect("encodes");
        assert_eq!(message.last(), Some(&0));
        let body: Value = serde_json::from_slice(&message[..message.len() - 1]).expect("json");
        assert_eq!(
            body,
            json!({ "id": 7, "method": "Storage.getCookies", "params": {} })
        );
        assert!(!message[..message.len() - 1].contains(&0));
    }

    #[test]
    fn a_page_command_names_its_session() {
        let message =
            encode(2, Some("S1"), "Page.navigate", json!({ "url": "u" })).expect("encodes");
        let body: Value = serde_json::from_slice(&message[..message.len() - 1]).expect("json");
        assert_eq!(body["sessionId"], "S1");
    }

    #[test]
    fn only_the_reply_to_this_command_is_taken() {
        let event = br#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"A","type":"page"}}}"#;
        let other = br#"{"id":3,"result":{}}"#;
        let reply = br#"{"id":4,"result":{"cookies":[{"name":"YSC","value":"abc","domain":".youtube.com","path":"/","expires":-1,"size":14,"httpOnly":true,"secure":true,"session":true,"sameSite":"None","priority":"Medium","sameParty":false,"sourceScheme":"Secure","sourcePort":443}]}}"#;
        assert!(reply_to(event, 4, "Storage.getCookies").is_none());
        assert!(reply_to(other, 4, "Storage.getCookies").is_none());
        assert!(reply_to(b"not json", 4, "Storage.getCookies").is_none());
        let result = reply_to(reply, 4, "Storage.getCookies")
            .expect("matched")
            .expect("ok");
        assert_eq!(result["cookies"][0]["name"], "YSC");
    }

    #[test]
    fn an_error_reply_is_an_error() {
        let reply = br#"{"id":1,"error":{"code":-32601,"message":"'Storage.nope' wasn't found"}}"#;
        let error = reply_to(reply, 1, "Storage.nope")
            .expect("matched")
            .expect_err("error");
        assert!(error.to_string().contains("wasn't found"));
    }

    #[test]
    fn cookies_are_kept_only_for_the_domain() {
        let reply = json!({ "cookies": [
            { "name": "SAPISID", "value": "a/b", "domain": ".youtube.com" },
            { "name": "PREF", "value": "f6=40000000", "domain": "music.youtube.com" },
            { "name": "SID", "value": "g", "domain": ".google.com" },
            { "name": "X", "value": "x", "domain": "notyoutube.com" },
            { "name": "broken" },
        ]});
        let cookies = cookies_in(&reply, "youtube.com");
        let names: Vec<_> = cookies.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["SAPISID", "PREF"]);
        assert_eq!(cookies[0].value, "a/b");
        assert!(cookies_in(&json!({}), "youtube.com").is_empty());
    }

    /// A thread stands in for the browser on the other ends of real pipes.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_call_round_trips_over_the_pipes() {
        use std::io::{BufRead, Write};

        let (mut cdp, ends) = pipes().expect("pipes");
        let BrowserEnds { read, write } = ends;
        let browser = std::thread::spawn(move || {
            let mut commands = std::io::BufReader::new(std::fs::File::from(read));
            let mut replies = std::fs::File::from(write);
            let mut first = Vec::new();
            commands.read_until(0, &mut first).expect("command");
            let command: Value = serde_json::from_slice(&first[..first.len() - 1]).expect("json");
            assert_eq!(command["method"], "Storage.getCookies");
            let id = command["id"].as_u64().expect("id");
            let mut out = Vec::new();
            out.extend_from_slice(br#"{"method":"Network.dataReceived","params":{}}"#);
            out.push(0);
            out.extend_from_slice(
                format!(r#"{{"id":{id},"result":{{"cookies":[{{"name":"VISITOR_INFO1_LIVE","value":"v","domain":".youtube.com"}}]}}}}"#)
                    .as_bytes(),
            );
            out.push(0);
            replies.write_all(&out).expect("reply");
        });
        let cookies = cdp.cookies("youtube.com").await.expect("cookies");
        browser.join().expect("browser thread");
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "VISITOR_INFO1_LIVE");
        let gone = cdp.call("Browser.close", json!({})).await;
        assert!(gone.is_err());
    }
}
