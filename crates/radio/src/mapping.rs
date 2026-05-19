use crate::manifest::{EntrySelector, FieldMapping, MatchValueFrom};
use std::collections::HashMap;
use serde_json::Value;

/// Traverse a JSON value using a dot-notation path.
/// Handles both object keys ("foo.bar") and array indices ("foo.0.bar").
pub fn extract_value<'a>(json: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(json);
    }

    let mut current = json;
    for segment in path.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(segment)?;
            }
            Value::Array(arr) => {
                if let Ok(idx) = segment.parse::<usize>() {
                    current = arr.get(idx)?;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(current)
}

pub fn extract_str(json: &Value, path: &str) -> Option<String> {
    let val = extract_value(json, path)?;
    match val {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract artist information, handling potential arrays.
pub fn extract_artist(json: &Value, mapping: &FieldMapping) -> String {
    let val = extract_value(json, &mapping.artist);

    let separator = mapping.artist_separator.as_deref().unwrap_or(", ");

    match val {
        Some(Value::Array(arr)) => {
            let mut artists = Vec::new();
            for item in arr {
                match item {
                    Value::String(s) => artists.push(s.clone()),
                    Value::Object(_) => {
                        if let Some(field) = &mapping.artist_array_field {
                            if let Some(s) = extract_str(item, field) {
                                artists.push(s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if artists.is_empty() {
                "Unknown Artist".to_string()
            } else {
                artists.join(separator)
            }
        }
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                "Unknown Artist".to_string()
            } else {
                s.clone()
            }
        }
        _ => "Unknown Artist".to_string(),
    }
}

pub fn extract_title(json: &Value, mapping: &FieldMapping) -> String {
    extract_str(json, &mapping.title).unwrap_or_else(|| "Unknown".to_string())
}

/// Extract artwork URL, applying template if specified.
pub fn extract_artwork(json: &Value, mapping: &FieldMapping) -> Option<String> {
    let path = mapping.artwork_url.as_ref()?;
    let extracted = extract_str(json, path)?;

    if extracted.is_empty() {
        return None;
    }

    if let Some(template) = &mapping.artwork_url_template {
        Some(template.replace("{value}", &extracted))
    } else {
        Some(extracted)
    }
}

/// Select the correct entry from a JSON array for a specific stream.
/// Used when a single API response contains data for multiple channels.
/// e.g: J1 Station
pub fn select_entry<'a>(
    json: &'a Value,
    selector: &EntrySelector,
    stream_id: &str,
    stream_name_map: &HashMap<String, String>,
) -> Option<&'a Value> {
    let array = extract_value(json, &selector.array_path)?;
    let arr = array.as_array()?;

    let match_value = match selector.match_value_from {
        MatchValueFrom::StreamName => stream_name_map
            .get(stream_id)
            .map(|s| s.as_str())
            .unwrap_or(stream_id),
        MatchValueFrom::StreamId => stream_id,
    };

    arr.iter().find(|entry| {
        extract_value(entry, &selector.match_field)
            .and_then(|v| v.as_str())
            .map(|s| s == match_value)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_value_basic() {
        let json = serde_json::json!({
            "song": {
                "title": "Hello",
                "track": 5
            }
        });

        assert_eq!(extract_str(&json, "song.title"), Some("Hello".to_string()));
        assert_eq!(extract_str(&json, "song.track"), Some("5".to_string()));
        assert_eq!(extract_str(&json, "song.missing"), None);
    }

    #[test]
    fn test_extract_value_array() {
        let json = serde_json::json!({
            "albums": [
                { "image": "cover.jpg" },
                { "image": "back.jpg" }
            ],
            "tags": ["pop", "rock"]
        });

        assert_eq!(extract_str(&json, "albums.0.image"), Some("cover.jpg".to_string()));
        assert_eq!(extract_str(&json, "albums.1.image"), Some("back.jpg".to_string()));
        assert_eq!(extract_str(&json, "tags.0"), Some("pop".to_string()));
        assert_eq!(extract_str(&json, "tags.2"), None);
    }

    #[test]
    fn test_extract_artist_string() {
        let json = serde_json::json!({
            "artist": "John Doe"
        });
        let mapping = FieldMapping {
            title: "title".into(),
            artist: "artist".into(),
            artwork_url: None,
            artwork_url_template: None,
            artist_separator: None,
            artist_array_field: None,
        };
        assert_eq!(extract_artist(&json, &mapping), "John Doe");
    }

    #[test]
    fn test_extract_artist_array_of_strings() {
        let json = serde_json::json!({
            "artists": ["John", "Jane"]
        });
        let mapping = FieldMapping {
            title: "title".into(),
            artist: "artists".into(),
            artwork_url: None,
            artwork_url_template: None,
            artist_separator: Some(" & ".into()),
            artist_array_field: None,
        };
        assert_eq!(extract_artist(&json, &mapping), "John & Jane");
    }

    #[test]
    fn test_extract_artist_array_of_objects() {
        // LISTEN.moe style
        let json = serde_json::json!({
            "song": {
                "artists": [
                    { "name": "Artist1" },
                    { "name": "Artist2" }
                ]
            }
        });
        let mapping = FieldMapping {
            title: "title".into(),
            artist: "song.artists".into(),
            artwork_url: None,
            artwork_url_template: None,
            artist_separator: None, // defaults to ", "
            artist_array_field: Some("name".into()),
        };
        assert_eq!(extract_artist(&json, &mapping), "Artist1, Artist2");
    }

    #[test]
    fn test_extract_artwork_template() {
        let json = serde_json::json!({
            "cover": "image123.jpg"
        });
        let mapping = FieldMapping {
            title: "title".into(),
            artist: "artist".into(),
            artwork_url: Some("cover".into()),
            artwork_url_template: Some("https://cdn.example.com/{value}".into()),
            artist_separator: None,
            artist_array_field: None,
        };
        assert_eq!(
            extract_artwork(&json, &mapping),
            Some("https://cdn.example.com/image123.jpg".to_string())
        );
    }

    // select_entry tests

    /// J1-style: API returns array for multiple channels, select by display name
    #[test]
    fn test_select_entry_stream_name() {
        let json = serde_json::json!({
            "station": [
                { "name": "J1 HITS", "title": "Song A", "artist": "Artist A", "image_url": "a.png" },
                { "name": "J1 GOLD", "title": "Song B", "artist": "Artist B", "image_url": "b.png" }
            ]
        });
        let selector = EntrySelector {
            array_path: "station".into(),
            match_field: "name".into(),
            match_value_from: MatchValueFrom::StreamName,
        };
        let mut name_map = HashMap::new();
        name_map.insert("J1HITS".to_string(), "J1 HITS".to_string());
        name_map.insert("J1GOLD".to_string(), "J1 GOLD".to_string());

        // Selecting HITS
        let entry = select_entry(&json, &selector, "J1HITS", &name_map).unwrap();
        assert_eq!(entry["title"].as_str().unwrap(), "Song A");
        assert_eq!(entry["artist"].as_str().unwrap(), "Artist A");

        // Selecting GOLD
        let entry = select_entry(&json, &selector, "J1GOLD", &name_map).unwrap();
        assert_eq!(entry["title"].as_str().unwrap(), "Song B");
        assert_eq!(entry["artist"].as_str().unwrap(), "Artist B");
    }

    /// Match directly against raw stream_id
    #[test]
    fn test_select_entry_stream_id() {
        let json = serde_json::json!({
            "channels": [
                { "id": "ch_alpha", "now_playing": "Track X" },
                { "id": "ch_beta",  "now_playing": "Track Y" }
            ]
        });
        let selector = EntrySelector {
            array_path: "channels".into(),
            match_field: "id".into(),
            match_value_from: MatchValueFrom::StreamId,
        };
        let empty_map = HashMap::new();

        let entry = select_entry(&json, &selector, "ch_beta", &empty_map).unwrap();
        assert_eq!(entry["now_playing"].as_str().unwrap(), "Track Y");
    }

    /// StreamName falls back to stream_id when stream_name_map has no entry
    #[test]
    fn test_select_entry_stream_name_fallback() {
        let json = serde_json::json!({
            "station": [
                { "name": "main", "title": "Fallback" }
            ]
        });
        let selector = EntrySelector {
            array_path: "station".into(),
            match_field: "name".into(),
            match_value_from: MatchValueFrom::StreamName,
        };
        let empty_map = HashMap::new();

        // No entry in stream_name_map, so stream_id "main" is used directly
        let entry = select_entry(&json, &selector, "main", &empty_map).unwrap();
        assert_eq!(entry["title"].as_str().unwrap(), "Fallback");
    }

    /// Returns None when no array entry matches
    #[test]
    fn test_select_entry_no_match() {
        let json = serde_json::json!({
            "station": [
                { "name": "J1 HITS", "title": "Song A" }
            ]
        });
        let selector = EntrySelector {
            array_path: "station".into(),
            match_field: "name".into(),
            match_value_from: MatchValueFrom::StreamId,
        };
        let empty_map = HashMap::new();

        assert!(select_entry(&json, &selector, "nonexistent", &empty_map).is_none());
    }

    /// Returns None when array_path points to a non-array value
    #[test]
    fn test_select_entry_not_an_array() {
        let json = serde_json::json!({
            "station": { "name": "single" }
        });
        let selector = EntrySelector {
            array_path: "station".into(),
            match_field: "name".into(),
            match_value_from: MatchValueFrom::StreamId,
        };
        let empty_map = HashMap::new();

        assert!(select_entry(&json, &selector, "single", &empty_map).is_none());
    }

    /// Returns None when array_path doesn't exist
    #[test]
    fn test_select_entry_missing_path() {
        let json = serde_json::json!({ "other": 42 });
        let selector = EntrySelector {
            array_path: "station".into(),
            match_field: "name".into(),
            match_value_from: MatchValueFrom::StreamId,
        };
        let empty_map = HashMap::new();

        assert!(select_entry(&json, &selector, "x", &empty_map).is_none());
    }

    /// Works with a nested array path
    #[test]
    fn test_select_entry_nested_array_path() {
        let json = serde_json::json!({
            "data": {
                "streams": [
                    { "key": "a", "song": "Hello" },
                    { "key": "b", "song": "World" }
                ]
            }
        });
        let selector = EntrySelector {
            array_path: "data.streams".into(),
            match_field: "key".into(),
            match_value_from: MatchValueFrom::StreamId,
        };
        let empty_map = HashMap::new();

        let entry = select_entry(&json, &selector, "b", &empty_map).unwrap();
        assert_eq!(entry["song"].as_str().unwrap(), "World");
    }
}
