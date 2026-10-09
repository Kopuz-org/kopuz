//! Write-side InnerTube endpoints — like/unlike/dislike a video, add/remove
//! from a playlist. All require the user's cookies + SAPISIDHASH auth.

use serde_json::{Value, json};

use super::actions::{ItemRef, bare_playlist_id, privacy_name};
use super::clients::{ORIGIN_YOUTUBE_MUSIC, WEB_REMIX};
use super::discover::{Privacy, Rating};
use super::innertube::sapisid_hash;

async fn post(endpoint: &str, body: Value, cookies: &str) -> Result<Value, String> {
    let client = WEB_REMIX;
    let auth =
        sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
    let resp = super::innertube::http_client()
        .clone()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/{endpoint}?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{endpoint} HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{endpoint} HTTP {}", resp.status()));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("{endpoint} JSON parse: {e}"))
}

fn ytmusic_context() -> Value {
    json!({
        "client": {
            "clientName": WEB_REMIX.client_name,
            "clientVersion": WEB_REMIX.client_version,
            "hl": "en",
            "gl": "US",
        },
    })
}

/// Add a video to the user's Liked Music auto-playlist.
#[tracing::instrument(name = "yt.like", skip(cookies), fields(video_id = %video_id))]
pub async fn like_video(video_id: &str, cookies: &str) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "target": { "videoId": video_id },
    });
    post("like/like", body, cookies).await.map(|_| ())
}

/// Remove a video from the user's Liked Music auto-playlist (unlike).
#[tracing::instrument(name = "yt.unlike", skip(cookies), fields(video_id = %video_id))]
pub async fn unlike_video(video_id: &str, cookies: &str) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "target": { "videoId": video_id },
    });
    post("like/removelike", body, cookies).await.map(|_| ())
}

/// Dislike a video — the "don't recommend" signal. Same endpoint family as
/// the like, and YouTube drops any existing like as a side effect of taking
/// it, so a caller holding a favorite row for this video must clear it.
#[tracing::instrument(name = "yt.dislike", skip(cookies), fields(video_id = %video_id))]
pub async fn dislike_video(video_id: &str, cookies: &str) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "target": { "videoId": video_id },
    });
    post("like/dislike", body, cookies).await.map(|_| ())
}

/// Add a video to a user playlist. `playlist_id` is the bare ID (no `VL`
/// prefix); `video_id` is the YT video ID.
#[tracing::instrument(name = "yt.playlist_add", skip(cookies), fields(playlist_id = %playlist_id, video_id = %video_id))]
pub async fn add_to_playlist(
    playlist_id: &str,
    video_id: &str,
    cookies: &str,
) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "playlistId": playlist_id,
        "actions": [{
            "action": "ACTION_ADD_VIDEO",
            "addedVideoId": video_id,
        }],
    });
    post("browse/edit_playlist", body, cookies)
        .await
        .map(|_| ())
}

/// Remove a video from a user playlist by video ID. (YT's API also
/// supports remove-by-setVideoId for repeats; we use the simpler
/// by-video-ID form which removes the first occurrence.)
#[tracing::instrument(name = "yt.playlist_remove", skip(cookies), fields(playlist_id = %playlist_id, video_id = %video_id))]
pub async fn remove_from_playlist(
    playlist_id: &str,
    video_id: &str,
    cookies: &str,
) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "playlistId": playlist_id,
        "actions": [{
            "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID",
            "removedVideoId": video_id,
        }],
    });
    post("browse/edit_playlist", body, cookies)
        .await
        .map(|_| ())
}

/// Create a new playlist with an optional initial set of video IDs.
#[tracing::instrument(name = "yt.playlist_create", skip(cookies, video_ids), fields(title = %title, count = video_ids.len()))]
pub async fn create_playlist(
    title: &str,
    video_ids: &[&str],
    cookies: &str,
) -> Result<String, String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "title": title,
        "description": "",
        "privacyStatus": "PRIVATE",
        "videoIds": video_ids,
    });
    let resp = post("playlist/create", body, cookies).await?;
    resp.get("playlistId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "create_playlist: no playlistId in response".to_string())
}

/// One write: the endpoint under `youtubei/v1/` and the body it takes,
/// built apart from the sending so each can be checked without a network.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub endpoint: &'static str,
    pub body: Value,
}

impl Request {
    fn new(endpoint: &'static str, mut fields: Value) -> Self {
        fields["context"] =
            json!({ "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } });
        Self {
            endpoint,
            body: fields,
        }
    }

    async fn send(self, cookies: &str) -> Result<Value, String> {
        post(self.endpoint, self.body, cookies).await
    }
}

/// A like, a dislike or neither, on a video or on a playlist (which is how
/// an album is rated too). A library toggle is not something to rate.
pub fn rate_request(target: ItemRef<'_>, rating: Rating) -> Result<Request, String> {
    let endpoint = match rating {
        Rating::Like => "like/like",
        Rating::Dislike => "like/dislike",
        Rating::Indifferent => "like/removelike",
    };
    let target = match target {
        ItemRef::Video(video_id) => json!({ "videoId": video_id }),
        ItemRef::Playlist(playlist_id) => json!({ "playlistId": bare_playlist_id(playlist_id) }),
        ItemRef::Library { .. } => return Err("a library toggle cannot be rated".to_string()),
    };
    Ok(Request::new(endpoint, json!({ "target": target })))
}

/// Save an album or playlist, or a song, to the library, or take it out.
/// Saving a playlist is a like on it; a song flips with the token for the
/// direction asked.
pub fn save_request(target: ItemRef<'_>, saved: bool) -> Result<Request, String> {
    match target {
        ItemRef::Playlist(_) => rate_request(
            target,
            if saved {
                Rating::Like
            } else {
                Rating::Indifferent
            },
        ),
        ItemRef::Library { add, remove } => Ok(feedback_request(if saved { add } else { remove })),
        ItemRef::Video(_) => Err("a song is saved by its library toggle, not its id".to_string()),
    }
}

pub fn subscribe_request(channel_id: &str, subscribed: bool) -> Request {
    let endpoint = if subscribed {
        "subscription/subscribe"
    } else {
        "subscription/unsubscribe"
    };
    Request::new(endpoint, json!({ "channelIds": [channel_id] }))
}

/// A feedback token sent back: a history removal, or a song's library toggle.
pub fn feedback_request(token: &str) -> Request {
    Request::new("feedback", json!({ "feedbackTokens": [token] }))
}

/// The name, description and privacy an edit names, in one call.
pub fn edit_playlist_request(
    playlist_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    privacy: Option<Privacy>,
) -> Request {
    let mut actions = Vec::new();
    if let Some(name) = name {
        actions.push(json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": name }));
    }
    if let Some(description) = description {
        actions.push(json!({
            "action": "ACTION_SET_PLAYLIST_DESCRIPTION",
            "playlistDescription": description,
        }));
    }
    if let Some(privacy) = privacy {
        actions.push(json!({
            "action": "ACTION_SET_PLAYLIST_PRIVACY",
            "playlistPrivacy": privacy_name(privacy),
        }));
    }
    Request::new(
        "browse/edit_playlist",
        json!({ "playlistId": bare_playlist_id(playlist_id), "actions": actions }),
    )
}

/// Move one entry to sit before `successor`, or to the end when there is
/// none. Both are entry ids (`setVideoId`), not video ids.
pub fn move_playlist_item_request(
    playlist_id: &str,
    set_video_id: &str,
    successor: Option<&str>,
) -> Request {
    let mut action = json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": set_video_id });
    if let Some(successor) = successor {
        action["movedSetVideoIdSuccessor"] = successor.into();
    }
    Request::new(
        "browse/edit_playlist",
        json!({ "playlistId": bare_playlist_id(playlist_id), "actions": [action] }),
    )
}

pub fn delete_playlist_request(playlist_id: &str) -> Request {
    Request::new(
        "playlist/delete",
        json!({ "playlistId": bare_playlist_id(playlist_id) }),
    )
}

#[tracing::instrument(name = "yt.rate", skip(cookies))]
pub async fn rate(target: ItemRef<'_>, rating: Rating, cookies: &str) -> Result<(), String> {
    rate_request(target, rating)?.send(cookies).await.map(drop)
}

#[tracing::instrument(name = "yt.save", skip(cookies))]
pub async fn save(target: ItemRef<'_>, saved: bool, cookies: &str) -> Result<(), String> {
    let request = save_request(target, saved)?;
    let feedback = request.endpoint == "feedback";
    let response = request.send(cookies).await?;
    if feedback {
        check_feedback(&response, "library change")?;
    }
    Ok(())
}

#[tracing::instrument(name = "yt.subscribe", skip(cookies))]
pub async fn subscribe(channel_id: &str, subscribed: bool, cookies: &str) -> Result<(), String> {
    subscribe_request(channel_id, subscribed)
        .send(cookies)
        .await
        .map(drop)
}

#[tracing::instrument(name = "yt.remove_from_history", skip_all)]
pub async fn remove_from_history(token: &str, cookies: &str) -> Result<(), String> {
    let response = feedback_request(token).send(cookies).await?;
    check_feedback(&response, "history removal")
}

#[tracing::instrument(name = "yt.playlist_edit", skip(cookies))]
pub async fn edit_playlist(
    playlist_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    privacy: Option<Privacy>,
    cookies: &str,
) -> Result<(), String> {
    let response = edit_playlist_request(playlist_id, name, description, privacy)
        .send(cookies)
        .await?;
    check_status(&response)
}

#[tracing::instrument(name = "yt.playlist_move", skip(cookies))]
pub async fn move_playlist_item(
    playlist_id: &str,
    set_video_id: &str,
    successor: Option<&str>,
    cookies: &str,
) -> Result<(), String> {
    let response = move_playlist_item_request(playlist_id, set_video_id, successor)
        .send(cookies)
        .await?;
    check_status(&response)
}

#[tracing::instrument(name = "yt.playlist_delete", skip(cookies))]
pub async fn delete_playlist(playlist_id: &str, cookies: &str) -> Result<(), String> {
    delete_playlist_request(playlist_id)
        .send(cookies)
        .await
        .map(drop)
}

/// A feedback call answers 200 whether or not it did anything; whether it
/// did is in the body.
fn check_feedback(response: &Value, what: &str) -> Result<(), String> {
    let processed = response["feedbackResponses"]
        .as_array()
        .is_some_and(|all| !all.is_empty() && all.iter().all(|r| r["isProcessed"] == true));
    if processed {
        Ok(())
    } else {
        Err(format!("YouTube did not accept the {what}"))
    }
}

fn check_status(response: &Value) -> Result<(), String> {
    match response["status"].as_str() {
        None | Some("STATUS_SUCCEEDED") => Ok(()),
        Some(status) => Err(format!("YouTube answered {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_carries_the_client_context() {
        let request = subscribe_request("UCabc", true);
        assert_eq!(
            request.body["context"]["client"]["clientName"],
            WEB_REMIX.client_name
        );
        assert_eq!(request.body["context"]["user"]["lockedSafetyMode"], false);
    }

    #[test]
    fn a_rating_picks_its_endpoint_and_target() {
        let cases = [
            (
                ItemRef::Video("vid"),
                Rating::Dislike,
                "like/dislike",
                json!({ "videoId": "vid" }),
            ),
            (
                ItemRef::Video("vid"),
                Rating::Indifferent,
                "like/removelike",
                json!({ "videoId": "vid" }),
            ),
            (
                ItemRef::Playlist("VLPLx"),
                Rating::Like,
                "like/like",
                json!({ "playlistId": "PLx" }),
            ),
        ];
        for (target, rating, endpoint, expected) in cases {
            let request = rate_request(target, rating).unwrap();
            assert_eq!(request.endpoint, endpoint);
            assert_eq!(request.body["target"], expected);
        }
        let toggle = ItemRef::Library {
            add: "a",
            remove: "r",
        };
        assert!(rate_request(toggle, Rating::Like).is_err());
    }

    #[test]
    fn saving_is_a_like_for_a_playlist_and_a_token_for_a_song() {
        let album = save_request(ItemRef::Playlist("OLAK5uy_x"), true).unwrap();
        assert_eq!(album.endpoint, "like/like");
        assert_eq!(album.body["target"]["playlistId"], "OLAK5uy_x");
        let unsave = save_request(ItemRef::Playlist("OLAK5uy_x"), false).unwrap();
        assert_eq!(unsave.endpoint, "like/removelike");

        let song = ItemRef::Library {
            add: "ADD",
            remove: "REMOVE",
        };
        let add = save_request(song, true).unwrap();
        assert_eq!(add.endpoint, "feedback");
        assert_eq!(add.body["feedbackTokens"], json!(["ADD"]));
        let remove = save_request(song, false).unwrap();
        assert_eq!(remove.body["feedbackTokens"], json!(["REMOVE"]));

        assert!(save_request(ItemRef::Video("vid"), true).is_err());
    }

    #[test]
    fn following_sends_the_channel() {
        let on = subscribe_request("UCabc", true);
        assert_eq!(on.endpoint, "subscription/subscribe");
        assert_eq!(on.body["channelIds"], json!(["UCabc"]));
        assert_eq!(
            subscribe_request("UCabc", false).endpoint,
            "subscription/unsubscribe"
        );
    }

    #[test]
    fn a_history_removal_is_its_token() {
        let request = feedback_request("TOKEN");
        assert_eq!(request.endpoint, "feedback");
        assert_eq!(request.body["feedbackTokens"], json!(["TOKEN"]));
    }

    #[test]
    fn a_playlist_edit_holds_only_what_it_names() {
        let request = edit_playlist_request("VLPLx", None, Some("notes"), Some(Privacy::Unlisted));
        assert_eq!(request.endpoint, "browse/edit_playlist");
        assert_eq!(request.body["playlistId"], "PLx");
        assert_eq!(
            request.body["actions"],
            json!([
                { "action": "ACTION_SET_PLAYLIST_DESCRIPTION", "playlistDescription": "notes" },
                { "action": "ACTION_SET_PLAYLIST_PRIVACY", "playlistPrivacy": "UNLISTED" },
            ])
        );
        let renamed = edit_playlist_request("PLx", Some("New"), None, None);
        assert_eq!(
            renamed.body["actions"],
            json!([{ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": "New" }])
        );
    }

    #[test]
    fn a_move_names_the_entry_and_the_one_it_lands_before() {
        let request = move_playlist_item_request("VLPLx", "AAA", Some("BBB"));
        assert_eq!(request.endpoint, "browse/edit_playlist");
        assert_eq!(request.body["playlistId"], "PLx");
        assert_eq!(
            request.body["actions"],
            json!([{
                "action": "ACTION_MOVE_VIDEO_BEFORE",
                "setVideoId": "AAA",
                "movedSetVideoIdSuccessor": "BBB",
            }])
        );
        let last = move_playlist_item_request("PLx", "AAA", None);
        assert_eq!(
            last.body["actions"],
            json!([{ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": "AAA" }])
        );
    }

    #[test]
    fn a_delete_names_the_bare_playlist() {
        let request = delete_playlist_request("VLPLx");
        assert_eq!(request.endpoint, "playlist/delete");
        assert_eq!(request.body["playlistId"], "PLx");
    }

    #[test]
    fn a_feedback_answer_is_read_for_whether_it_took() {
        let taken = json!({ "feedbackResponses": [{ "isProcessed": true }] });
        let refused = json!({ "feedbackResponses": [{ "isProcessed": false }] });
        assert!(check_feedback(&taken, "x").is_ok());
        assert!(check_feedback(&refused, "x").is_err());
        assert!(check_feedback(&json!({}), "x").is_err());
        assert!(check_status(&json!({ "status": "STATUS_FAILED" })).is_err());
        assert!(check_status(&json!({ "status": "STATUS_SUCCEEDED" })).is_ok());
    }
}
