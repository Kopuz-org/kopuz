//! Drive kopuz's actual YouTubeMusicClient::get_stream() against a real
//! video_id and print exactly what duration_secs comes back. If this
//! prints Some(N), the wire-extraction half of the duration fix is
//! working and any remaining 0:00 in the UI is a signal-plumbing bug
//! in the player controller, not a network/parse bug.
#![expect(clippy::print_stdout, reason = "CLI probe example reports its findings on stdout")]

use serde_json::Value;
use server::ytmusic::YouTubeMusicClient;

fn read_kopuz_cookies() -> Result<String, Box<dyn std::error::Error>> {
    let conf: Value = serde_json::from_str(&std::fs::read_to_string(
        std::env::var("HOME").unwrap_or_default() + "/.config/kopuz/config.json",
    )?)?;
    Ok(conf
        .pointer("/server/access_token")
        .and_then(|v| v.as_str())
        .ok_or("no cookies in kopuz config")?
        .to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let video_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "VwliGCRwAgc".to_string());
    let cookies = read_kopuz_cookies()?;
    let yt = YouTubeMusicClient::with_cookies(cookies);

    println!("Calling get_stream({video_id})…");
    let info = yt.get_stream(&video_id).await?;
    println!(
        "✓ url             = {}…",
        info.url.chars().take(80).collect::<String>()
    );
    println!("  format          = {:?}", info.format);
    println!("  user_agent      = {}", info.user_agent);
    println!("  content_length  = {:?}", info.content_length);
    println!("  duration_secs   = {:?}  ← THE FIELD UNDER TEST", info.duration_secs);
    if info.duration_secs.is_none() {
        println!("\n  ⚠ duration_secs is None — pick_plain_format didn't populate it.");
    } else if info.duration_secs == Some(0) {
        println!("\n  ⚠ duration_secs is Some(0) — parsed but zero.");
    } else {
        println!("\n  ✓ Wire-level extraction works. Issue lives elsewhere.");
    }
    Ok(())
}
