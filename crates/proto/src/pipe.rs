//! The daemon's address on Windows: a named pipe standing in for the Unix
//! socket the other platforms use.
//!
//! The socket path a caller hands over becomes a name under `\\.\pipe\`.
//! The pipe is created with a DACL that admits only the user who created
//! it, which is the boundary the `0600` socket mode draws elsewhere. Pipe
//! names are one global namespace, so before a client speaks it checks
//! that the pipe it opened is owned by the same user: anyone can create a
//! pipe by that name first, and a squatter must not get to answer as the
//! daemon.

use std::ffi::{OsStr, OsString, c_void};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Every instance is mid-handshake only while the daemon posts the next
/// one, so a busy pipe clears within a scheduler tick or two.
const BUSY_RETRIES: u32 = 40;
const BUSY_RETRY: Duration = Duration::from_millis(50);

/// The pipe a socket path stands for. A name that already lives under
/// `\\.\pipe\` is used as given; any other path is flattened into one.
pub fn name(path: &Path) -> OsString {
    let path = path.to_string_lossy();
    if path.starts_with(PIPE_PREFIX) {
        return OsString::from(path.as_ref());
    }
    let flat: String = path
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' => '-',
            c => c,
        })
        .collect();
    OsString::from(format!("{PIPE_PREFIX}{}", flat.trim_start_matches('-')))
}

/// Where this user's daemon listens unless told otherwise. The SID keeps
/// two users on one machine from contending for a single name.
pub fn default_name() -> io::Result<OsString> {
    let sid = UserSid::current()?.to_string_sid()?;
    Ok(OsString::from(format!("{PIPE_PREFIX}kopuz-{sid}")))
}

/// A pipe the daemon serves on, yielding one connection per client.
#[derive(Debug)]
pub struct Listener {
    name: OsString,
    sddl: Vec<u16>,
    pending: NamedPipeServer,
}

impl Listener {
    /// Take the name. A pipe that already exists makes this fail with
    /// `AddrInUse`, the same signal a served socket gives elsewhere.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let name = name(path);
        let sid = UserSid::current()?.to_string_sid()?;
        let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
        let pending = match instance(&name, &sddl, true) {
            Ok(server) => server,
            Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("a kopuzd is already serving {}", name.to_string_lossy()),
                ));
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            name,
            sddl,
            pending,
        })
    }

    async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.pending.connect().await?;
        let next = instance(&self.name, &self.sddl, false)?;
        Ok(std::mem::replace(&mut self.pending, next))
    }

    /// The connection stream tonic serves.
    pub fn incoming(self) -> impl futures_util::Stream<Item = io::Result<Connection>> + Send {
        futures_util::stream::try_unfold(self, |mut listener| async move {
            let server = listener.accept().await?;
            Ok(Some((Connection(server), listener)))
        })
    }
}

/// One pipe instance under the owner-only descriptor. Every instance
/// carries the same DACL; only the first one claims the name.
fn instance(name: &OsStr, sddl: &[u16], first: bool) -> io::Result<NamedPipeServer> {
    let descriptor = OwnerOnly::from_sddl(sddl)?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: false.into(),
    };
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(
                name,
                std::ptr::from_mut(&mut attributes).cast::<c_void>(),
            )
    }
}

/// Open the daemon's pipe and confirm the same user is behind it.
pub async fn connect(path: &Path) -> io::Result<NamedPipeClient> {
    let name = name(path);
    let mut attempts = 0;
    let client = loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => break client,
            Err(error)
                if error.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32)
                    && attempts < BUSY_RETRIES =>
            {
                attempts += 1;
                tokio::time::sleep(BUSY_RETRY).await;
            }
            Err(error) => return Err(error),
        }
    };
    if owned_by_current_user(&client)? {
        Ok(client)
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is served by another user", name.to_string_lossy()),
        ))
    }
}

fn owned_by_current_user(client: &NamedPipeClient) -> io::Result<bool> {
    let mine = UserSid::current()?;
    let mut owner = PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            HANDLE(client.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let same = unsafe { EqualSid(owner, mine.as_psid()) }.is_ok();
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    Ok(same)
}

/// One accepted client, as tonic wants it.
pub struct Connection(NamedPipeServer);

impl tonic::transport::server::Connected for Connection {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// The SID of the user this process runs as. The buffer holds the
/// `TOKEN_USER` header and the SID bytes it points into, 8-aligned.
struct UserSid(Vec<u64>);

impl UserSid {
    fn current() -> io::Result<Self> {
        let mut raw = HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }
            .map_err(io::Error::other)?;
        let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let handle = HANDLE(token.as_raw_handle());
        let mut needed = 0u32;
        let _ = unsafe { GetTokenInformation(handle, TokenUser, None, 0, &mut needed) };
        let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
        unsafe {
            GetTokenInformation(
                handle,
                TokenUser,
                Some(buffer.as_mut_ptr().cast::<c_void>()),
                needed,
                &mut needed,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Self(buffer))
    }

    fn as_psid(&self) -> PSID {
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    fn to_string_sid(&self) -> io::Result<String> {
        let mut text = PWSTR::null();
        unsafe { ConvertSidToStringSidW(self.as_psid(), &mut text) }.map_err(io::Error::other)?;
        let sid = unsafe { text.to_string() }.map_err(io::Error::other);
        unsafe { LocalFree(Some(HLOCAL(text.0.cast::<c_void>()))) };
        sid
    }
}

/// A security descriptor parsed from SDDL, freed on drop.
struct OwnerOnly(PSECURITY_DESCRIPTOR);

impl OwnerOnly {
    fn from_sddl(sddl: &[u16]) -> io::Result<Self> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR::from_raw(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Self(descriptor))
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::name;
    use std::path::Path;

    #[test]
    fn a_pipe_name_is_kept_and_a_path_is_flattened_into_one() {
        assert_eq!(
            name(Path::new(r"\\.\pipe\kopuz-test")).to_string_lossy(),
            r"\\.\pipe\kopuz-test"
        );
        assert_eq!(
            name(Path::new(r"C:\Users\me\AppData\Local\kopuz\kopuzd.sock")).to_string_lossy(),
            r"\\.\pipe\C--Users-me-AppData-Local-kopuz-kopuzd.sock"
        );
        assert_eq!(
            name(Path::new("/tmp/kopuzd.sock")).to_string_lossy(),
            r"\\.\pipe\tmp-kopuzd.sock"
        );
    }
}
