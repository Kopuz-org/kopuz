//! What the signed-in account has done to an item, read from the menus and
//! header buttons a page draws for it, and the refs the mutations take.
//!
//! A ref is handed to a frontend and comes back unread, so it says which
//! endpoint it is for: a bare id is a video (a track's key is one), a
//! `playlist:` ref is a playlist or an album's audio playlist, and a
//! `library:` ref holds the pair of feedback tokens that save a song to the
//! library and take it out again.

use serde_json::Value;

use super::discover::{ItemActions, Privacy, Rating};

const PLAYLIST: &str = "playlist:";
const LIBRARY: &str = "library:";

/// What a ref names, once read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemRef<'a> {
    Video(&'a str),
    Playlist(&'a str),
    /// A song's library toggle: the token that adds it and the one that removes it.
    Library {
        add: &'a str,
        remove: &'a str,
    },
}

/// The bare id, without the `VL` a browse id carries.
pub fn bare_playlist_id(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

pub fn playlist_ref(id: &str) -> String {
    format!("{PLAYLIST}{}", bare_playlist_id(id))
}

/// Feedback tokens are base64, so `|` never occurs in one.
fn library_ref(add: &str, remove: &str) -> String {
    format!("{LIBRARY}{add}|{remove}")
}

pub fn read_ref(item_ref: &str) -> Option<ItemRef<'_>> {
    let item_ref = item_ref.trim();
    if item_ref.is_empty() {
        return None;
    }
    if let Some(id) = item_ref.strip_prefix(PLAYLIST) {
        return (!id.is_empty()).then_some(ItemRef::Playlist(id));
    }
    if let Some(tokens) = item_ref.strip_prefix(LIBRARY) {
        let (add, remove) = tokens.split_once('|')?;
        return (!add.is_empty() && !remove.is_empty()).then_some(ItemRef::Library { add, remove });
    }
    Some(ItemRef::Video(item_ref))
}

pub fn rating(like_status: &str) -> Option<Rating> {
    match like_status {
        "LIKE" => Some(Rating::Like),
        "DISLIKE" => Some(Rating::Dislike),
        "INDIFFERENT" => Some(Rating::Indifferent),
        _ => None,
    }
}

pub fn privacy(status: &str) -> Option<Privacy> {
    match status {
        "PUBLIC" => Some(Privacy::Public),
        "UNLISTED" => Some(Privacy::Unlisted),
        "PRIVATE" => Some(Privacy::Private),
        _ => None,
    }
}

pub fn privacy_name(privacy: Privacy) -> &'static str {
    match privacy {
        Privacy::Public => "PUBLIC",
        Privacy::Unlisted => "UNLISTED",
        Privacy::Private => "PRIVATE",
    }
}

fn menu_items(menu: &Value) -> impl Iterator<Item = &Value> {
    menu["menuRenderer"]["items"]
        .as_array()
        .into_iter()
        .flatten()
}

/// The like state a row's menu shows. `None` where it has no like button.
fn like_status(menu: &Value) -> Option<Rating> {
    menu["menuRenderer"]["topLevelButtons"]
        .as_array()?
        .iter()
        .find_map(|button| rating(button["likeButtonRenderer"]["likeStatus"].as_str()?))
}

/// The "Remove from history" token in a History row's menu. Rows elsewhere
/// carry feedback tokens too, for "Not interested" and the like.
fn history_token(menu: &Value) -> Option<String> {
    menu_items(menu)
        .map(|item| &item["menuServiceItemRenderer"])
        .filter(|item| item["icon"]["iconType"] == "REMOVE_FROM_HISTORY")
        .find_map(|item| {
            item["serviceEndpoint"]["feedbackEndpoint"]["feedbackToken"]
                .as_str()
                .map(str::to_string)
        })
}

/// Whether the toggle's resting state is the saved one.
fn saved_icon(toggle: &Value) -> Option<bool> {
    match toggle["defaultIcon"]["iconType"].as_str()? {
        "BOOKMARK_BORDER" | "LIBRARY_ADD" => Some(false),
        "BOOKMARK" | "LIBRARY_SAVED" | "LIBRARY_REMOVE" => Some(true),
        _ => None,
    }
}

/// A song's "Save to library" toggle: whether it is saved, and the ref that
/// flips it. Signed out, the save half opens a sign-in prompt instead of
/// carrying a token, and there is no ref.
fn library_toggle(menu: &Value) -> Option<(bool, String)> {
    menu_items(menu)
        .map(|item| &item["toggleMenuServiceItemRenderer"])
        .find_map(|toggle| {
            let saved = saved_icon(toggle)?;
            let token = |endpoint: &str| {
                toggle[endpoint]["feedbackEndpoint"]["feedbackToken"]
                    .as_str()
                    .map(str::to_string)
            };
            let (now, other) = (
                token("defaultServiceEndpoint")?,
                token("toggledServiceEndpoint")?,
            );
            let (add, remove) = if saved { (other, now) } else { (now, other) };
            Some((saved, library_ref(&add, &remove)))
        })
}

/// An album or playlist card's "Save to library" toggle, which is a like on
/// its playlist: whether it is saved, and that playlist's id when the
/// toggle names it.
fn playlist_toggle(menu: &Value) -> Option<(bool, Option<String>)> {
    menu_items(menu)
        .map(|item| &item["toggleMenuServiceItemRenderer"])
        .find_map(|toggle| {
            let saved = saved_icon(toggle)?;
            let id = ["defaultServiceEndpoint", "toggledServiceEndpoint"]
                .into_iter()
                .find_map(|endpoint| {
                    toggle[endpoint]["likeEndpoint"]["target"]["playlistId"].as_str()
                })
                .map(str::to_string);
            let is_save =
                id.is_some() || toggle["defaultServiceEndpoint"]["modalEndpoint"].is_object();
            is_save.then_some((saved, id))
        })
}

/// A song or video row: its rating, its library toggle and, in History, the
/// token that takes it out.
pub fn track(menu: &Value, video_id: &str) -> ItemActions {
    let library = library_toggle(menu);
    ItemActions {
        rate_ref: Some(video_id.to_string()),
        rating: like_status(menu),
        saved: library.as_ref().map(|(saved, _)| *saved),
        save_ref: library.map(|(_, save_ref)| save_ref),
        history_token: history_token(menu),
        ..ItemActions::default()
    }
}

/// An album or playlist card or row. `playlist_id` is the playlist that plays
/// it, which for an album is not its browse id; the menu's own toggle names
/// it more reliably where it is there.
pub fn playlist(menu: &Value, playlist_id: Option<&str>) -> ItemActions {
    let toggle = playlist_toggle(menu);
    let id = toggle
        .as_ref()
        .and_then(|(_, id)| id.as_deref())
        .or(playlist_id)
        .map(playlist_ref);
    ItemActions {
        rate_ref: id.clone(),
        rating: like_status(menu),
        saved: toggle.map(|(saved, _)| saved),
        save_ref: id,
        ..ItemActions::default()
    }
}

/// An artist row or card, which is followed by its channel.
pub fn artist(channel_id: &str) -> ItemActions {
    ItemActions {
        follow_ref: Some(channel_id.to_string()),
        ..ItemActions::default()
    }
}

/// The playlist a card's play button starts.
pub fn overlay_playlist_id(overlay: &Value) -> Option<&str> {
    let endpoint = &overlay["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
        ["playNavigationEndpoint"];
    endpoint["watchPlaylistEndpoint"]["playlistId"]
        .as_str()
        .or_else(|| endpoint["watchEndpoint"]["playlistId"].as_str())
}

/// An artist header's subscribe button.
pub fn subscription(header: &Value) -> ItemActions {
    let subscribe = &header["subscriptionButton"]["subscribeButtonRenderer"];
    match subscribe["channelId"].as_str() {
        Some(channel) => ItemActions {
            followed: subscribe["subscribed"].as_bool(),
            ..artist(channel)
        },
        None => ItemActions::default(),
    }
}

/// An album, playlist or podcast header: its save toggle, and the playlist
/// it plays. A playlist the account owns has no toggle, being its own.
pub fn detail_header(header: &Value, playback_id: Option<&str>) -> ItemActions {
    let buttons = header["buttons"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let toggle = buttons
        .iter()
        .map(|button| &button["toggleButtonRenderer"])
        .find(|toggle| saved_icon(toggle).is_some());
    let Some(toggle) = toggle else {
        return ItemActions {
            rate_ref: playback_id.map(playlist_ref),
            ..ItemActions::default()
        };
    };
    let id = ["defaultServiceEndpoint", "toggledServiceEndpoint"]
        .into_iter()
        .find_map(|endpoint| toggle[endpoint]["likeEndpoint"]["target"]["playlistId"].as_str())
        .or(playback_id)
        .map(playlist_ref);
    ItemActions {
        rate_ref: id.clone(),
        save_ref: id,
        saved: toggle["isToggled"].as_bool(),
        ..ItemActions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_ref_reads_back_as_what_it_was_made_from() {
        assert_eq!(read_ref("dQw4w9WgXcQ"), Some(ItemRef::Video("dQw4w9WgXcQ")));
        assert_eq!(
            read_ref(&playlist_ref("VLPLabc")),
            Some(ItemRef::Playlist("PLabc"))
        );
        assert_eq!(
            read_ref(&library_ref("AB_c-1=", "XY_z-2=")),
            Some(ItemRef::Library {
                add: "AB_c-1=",
                remove: "XY_z-2="
            })
        );
        assert_eq!(read_ref(""), None);
        assert_eq!(read_ref("playlist:"), None);
        assert_eq!(read_ref("library:onlyone"), None);
    }

    fn signed_in_menu(saved: bool) -> Value {
        let (default, toggled) = if saved {
            ("REMOVE_TOKEN", "ADD_TOKEN")
        } else {
            ("ADD_TOKEN", "REMOVE_TOKEN")
        };
        json!({"menuRenderer": {
            "items": [
                {"menuServiceItemRenderer": {
                    "icon": {"iconType": "REMOVE_FROM_HISTORY"},
                    "serviceEndpoint": {"feedbackEndpoint": {"feedbackToken": "HISTORY_TOKEN"}}
                }},
                {"toggleMenuServiceItemRenderer": {
                    "defaultIcon": {"iconType": if saved { "LIBRARY_SAVED" } else { "LIBRARY_ADD" }},
                    "defaultServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": default}},
                    "toggledServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": toggled}}
                }}
            ],
            "topLevelButtons": [{"likeButtonRenderer": {"likeStatus": "DISLIKE"}}]
        }})
    }

    /// The add and remove tokens swap places with the toggle's state, so the
    /// ref has to keep them by role, not by position.
    #[test]
    fn a_history_row_carries_its_tokens_by_role() {
        for saved in [false, true] {
            let actions = track(&signed_in_menu(saved), "vid");
            assert_eq!(actions.rate_ref.as_deref(), Some("vid"));
            assert_eq!(actions.rating, Some(Rating::Dislike));
            assert_eq!(actions.saved, Some(saved));
            assert_eq!(actions.history_token.as_deref(), Some("HISTORY_TOKEN"));
            assert_eq!(
                actions.save_ref.as_deref().and_then(read_ref),
                Some(ItemRef::Library {
                    add: "ADD_TOKEN",
                    remove: "REMOVE_TOKEN"
                })
            );
        }
    }

    #[test]
    fn a_signed_out_menu_offers_no_library_ref() {
        let menu = json!({"menuRenderer": {"items": [{"toggleMenuServiceItemRenderer": {
            "defaultIcon": {"iconType": "BOOKMARK_BORDER"},
            "defaultServiceEndpoint": {"modalEndpoint": {}},
            "toggledServiceEndpoint": {"feedbackEndpoint": {"feedbackToken": "T"}}
        }}]}});
        let actions = track(&menu, "vid");
        assert_eq!(actions.save_ref, None);
        assert_eq!(actions.saved, None);
        assert_eq!(actions.history_token, None);
    }

    #[test]
    fn a_card_is_saved_through_the_playlist_its_toggle_names() {
        let menu = json!({"menuRenderer": {"items": [{"toggleMenuServiceItemRenderer": {
            "defaultIcon": {"iconType": "LIBRARY_SAVED"},
            "defaultServiceEndpoint": {"likeEndpoint": {"status": "INDIFFERENT", "target": {"playlistId": "OLAK5uy_x"}}},
            "toggledServiceEndpoint": {"likeEndpoint": {"status": "LIKE", "target": {"playlistId": "OLAK5uy_x"}}}
        }}]}});
        let actions = playlist(&menu, Some("OLAK5uy_other"));
        assert_eq!(actions.saved, Some(true));
        assert_eq!(actions.save_ref.as_deref(), Some("playlist:OLAK5uy_x"));
        assert_eq!(actions.rate_ref, actions.save_ref);
    }
}
