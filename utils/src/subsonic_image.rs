pub fn parse_subsonic_path(path_str: &str) -> Option<(&str, Option<&str>)> {
    let parts: Vec<&str> = path_str.split(':').collect();
    if parts.len() >= 2 {
        let id = parts[1];
        let tag = if parts.len() >= 3 {
            Some(parts[2])
        } else {
            None
        };
        Some((id, tag))
    } else {
        None
    }
}

pub fn subsonic_image_url_from_path(
    path_str: &str,
    _server_url: &str,
    _access_token: Option<&str>,
    _max_width: u32,
    _quality: u32,
) -> Option<String> {
    let (_, tag) = parse_subsonic_path(path_str)?;
    if tag == Some("none") {
        return None;
    }

    if let Some(tag) = tag {
        if let Some(url) = decode_embedded_cover_url(tag) {
            return Some(url);
        }
    }

    None
}

fn decode_embedded_cover_url(tag: &str) -> Option<String> {
    let hex = tag.strip_prefix("urlhex_")?;
    if hex.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let chars: Vec<char> = hex.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let hi = chars[i].to_digit(16)?;
        let lo = chars[i + 1].to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
        i += 2;
    }

    String::from_utf8(bytes).ok()
}
