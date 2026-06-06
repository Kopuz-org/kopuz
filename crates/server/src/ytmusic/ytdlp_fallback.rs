use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use super::player::{AudioFormat, YtStreamInfo};

pub async fn resolve(video_id: &str, cookies: &str) -> Result<YtStreamInfo, String> {
    if which("yt-dlp").is_none() {
        return Err(
            "yt-dlp not installed — needed for Music Premium tracks. \
             Install it (e.g. `pacman -S yt-dlp` / `pipx install yt-dlp`) and retry."
                .to_string(),
        );
    }

    let cookies_path = write_netscape_cookies(cookies)
        .map_err(|e| format!("write cookies.txt for yt-dlp: {e}"))?;
    let result = run_resolve(video_id, &cookies_path).await;
    let _ = tokio::fs::remove_file(&cookies_path).await;
    result
}

async fn run_resolve(video_id: &str, cookies_path: &PathBuf) -> Result<YtStreamInfo, String> {
    let url = format!("https://music.youtube.com/watch?v={video_id}");
    let cookies_str = cookies_path.to_string_lossy().into_owned();

    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "--cookies",
        cookies_str.as_str(),
        "-f",
        "bestaudio[ext=webm]/bestaudio[ext=m4a]/bestaudio",
        "--extractor-args",
        "youtube:player_client=web_safari",
        "--no-playlist",
        "--no-warnings",
        "--quiet",
        "--print",
        "%(url)s|%(http_headers.User-Agent)s|%(filesize,filesize_approx)s|%(ext)s",
        url.as_str(),
    ]);
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| "yt-dlp timed out after 30s".to_string())?
        .map_err(|e| format!("spawn yt-dlp: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .unwrap_or_else(|| stderr.lines().last().unwrap_or(""));
        return Err(format!(
            "yt-dlp exit {}: {}",
            out.status.code().unwrap_or(-1),
            detail.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .next()
        .ok_or_else(|| "yt-dlp returned no output".to_string())?
        .trim();
    let parts: Vec<&str> = line.splitn(4, '|').collect();
    if parts.len() != 4 {
        return Err(format!(
            "yt-dlp --print output malformed (expected 4 fields, got {}): {line}",
            parts.len()
        ));
    }

    let user_agent = match parts[1] {
        "" | "NA" => default_web_safari_ua().to_string(),
        s => s.to_string(),
    };
    let format = match parts[3] {
        "m4a" | "mp4" => AudioFormat::M4a,
        _ => AudioFormat::Webm,
    };

    Ok(YtStreamInfo {
        url: parts[0].to_string(),
        format,
        user_agent,
        content_length: parts[2].parse::<u64>().ok(),
    })
}

fn default_web_safari_ua() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.5 Safari/605.1.15"
}

fn write_netscape_cookies(cookie_header: &str) -> std::io::Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!("kopuz-yt-ytdlp-cookies-{}.txt", std::process::id()));
    let mut f = std::fs::File::create(&path)?;
    writeln!(f, "# Netscape HTTP Cookie File")?;
    const FAR_FUTURE: u64 = 2_147_483_647;
    for pair in cookie_header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        writeln!(
            f,
            ".youtube.com\tTRUE\t/\tTRUE\t{FAR_FUTURE}\t{name}\t{}",
            value.trim()
        )?;
    }
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

fn which(bin: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
