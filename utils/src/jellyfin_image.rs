pub fn jellyfin_image_url(
    server_url: &str,
    item_id: &str,
    image_tag: Option<&str>,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> String {
    if let Some(tag) = image_tag {
        if let Some(url) = decode_embedded_cover_url(tag) {
            return url;
        }
    }

    let mut params = Vec::new();
    params.push(format!("maxWidth={}", max_width));
    params.push(format!("quality={}", quality));

    if let Some(tag) = image_tag {
        params.push(format!("tag={}", tag));
    }
    if let Some(token) = access_token {
        params.push(format!("api_key={}", token));
    }

    let mut url = format!("{}/Items/{}/Images/Primary", server_url, item_id);
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    url
}

pub fn parse_jellyfin_path(path_str: &str) -> Option<(&str, Option<&str>)> {
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

pub fn jellyfin_image_url_from_path(
    path_str: &str,
    server_url: &str,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> Option<String> {
    let (id, tag) = parse_jellyfin_path(path_str)?;
    if tag == Some("none") {
        return None;
    }

    if let Some(tag) = tag {
        if let Some(url) = decode_embedded_cover_url(tag) {
            return Some(url);
        }
    }

    Some(jellyfin_image_url(
        server_url,
        id,
        tag,
        access_token,
        max_width,
        quality,
    ))
}

pub fn track_cover_url_with_album_fallback(
    track_path_str: &str,
    album_id_str: &str,
    server_url: &str,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> Option<String> {
    if let Some((id, Some(tag))) = parse_jellyfin_path(track_path_str) {
        if tag == "none" {
            return None;
        }
        if let Some(url) = decode_embedded_cover_url(tag) {
            return Some(url);
        }

        return Some(jellyfin_image_url(
            server_url,
            id,
            Some(tag),
            access_token,
            max_width,
            quality,
        ));
    }

    if !album_id_str.is_empty() {
        if let Some((album_item_id, album_tag)) = parse_jellyfin_path(album_id_str) {
            if album_tag == Some("none") {
                return None;
            }

            if let Some(tag) = album_tag {
                if let Some(url) = decode_embedded_cover_url(tag) {
                    return Some(url);
                }
            }

            return Some(jellyfin_image_url(
                server_url,
                album_item_id,
                album_tag,
                access_token,
                max_width,
                quality,
            ));
        }
    }

    if let Some((id, _)) = parse_jellyfin_path(track_path_str) {
        return Some(jellyfin_image_url(
            server_url,
            id,
            None,
            access_token,
            max_width,
            quality,
        ));
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
