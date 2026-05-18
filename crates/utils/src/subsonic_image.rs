pub fn parse_subsonic_path(path_str: &str) -> Option<(&str, Option<&str>)> {
    let parts: Vec<&str> = path_str.split(':').collect();
    if parts.len() >= 2 {
        let id = parts[1].trim();
        if id.is_empty() {
            return None;
        }
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
    server_url: &str,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> Option<String> {
    let (id, tag) = parse_subsonic_path(path_str)?;
    if tag == Some("none") {
        return None;
    }

    if let Some(tag) = tag {
        if let Some(url) = decode_embedded_cover_url(tag) {
            return Some(url);
        }
    }

    let mut url = reqwest::Url::parse(&format!(
        "{}/rest/getCoverArt.view",
        server_url.trim_end_matches('/')
    ))
    .unwrap_or_else(|_| reqwest::Url::parse("http://127.0.0.1/").unwrap());

    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("id", id);
        pairs.append_pair("size", &max_width.to_string());
        pairs.append_pair("quality", &quality.to_string());
        if let Some(token) = access_token {
            pairs.append_pair("access_token", token);
        }
    }

    Some(url.to_string())
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
