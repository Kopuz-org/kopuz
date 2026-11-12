//! `kopuz://` links: what one asks for, and doing it. `docs/protocol.md` is the grammar others write against.

use api::QueueContext;
use gpui_kit::{App, Window};
use percent_encoding::percent_decode_str;

use crate::{Chrome, Navigator, Route, actions};

/// Something a link plays, named by the id the active source knows it under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Track(String),
    Album(String),
    Artist(String),
    Playlist(String),
}

/// A source a link offers to add; nothing is stored until the person saves the form it fills in.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SourceOffer {
    pub service: String,
    pub name: Option<String>,
    /// Answers to the service's own form, by field key.
    pub values: Vec<(String, String)>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for SourceOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceOffer")
            .field("service", &self.service)
            .field("name", &self.name)
            .field("values", &self.values)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What a link asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    Open(Route),
    Search(String),
    Play(Item),
    /// Add one track to the end of the queue.
    Queue(String),
    AddSource(SourceOffer),
}

/// The part after `kopuz://`, for a link addressed to this app.
fn addressed(text: &str) -> Option<&str> {
    let (scheme, rest) = text.split_once("://")?;
    scheme.eq_ignore_ascii_case("kopuz").then_some(rest)
}

/// A "Listen on Kopuz" link, which the daemon plays; every other link is this module's.
pub fn is_listen(text: &str) -> bool {
    addressed(text).is_some_and(|rest| rest == "j" || rest.starts_with("j/"))
}

/// Read a link; `None` for anything the grammar does not name.
pub fn parse(text: &str) -> Option<DeepLink> {
    addressed(text)?;
    let url = url::Url::parse(text.trim()).ok()?;
    let verb = url.host_str()?.to_ascii_lowercase();
    let segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .map(|segment| percent_decode_str(segment).decode_utf8().ok())
        .collect::<Option<Vec<_>>>()?;
    let segments: Vec<&str> = segments.iter().map(AsRef::as_ref).collect();
    Some(match (verb.as_str(), segments.as_slice()) {
        ("open", page) if !page.is_empty() => DeepLink::Open(Route::from_name(&page.join("/"))?),
        ("album", [id]) => DeepLink::Open(Route::Album(id.to_string().into())),
        ("artist", [key]) => DeepLink::Open(Route::Artist(key.to_string())),
        ("playlist", [id]) => DeepLink::Open(Route::Playlist(id.to_string().into())),
        ("search", []) => {
            let (_, query) = url.query_pairs().find(|(name, _)| name == "q")?;
            let query = query.trim();
            if query.is_empty() {
                return None;
            }
            DeepLink::Search(query.to_string())
        }
        ("play", [kind, id]) => DeepLink::Play(match *kind {
            "track" => Item::Track(id.to_string()),
            "album" => Item::Album(id.to_string()),
            "artist" => Item::Artist(id.to_string()),
            "playlist" => Item::Playlist(id.to_string()),
            _ => return None,
        }),
        ("queue", ["track", key]) => DeepLink::Queue(key.to_string()),
        ("add-source", [service]) => {
            let mut offer = SourceOffer {
                service: service.to_string(),
                ..SourceOffer::default()
            };
            for (key, value) in url.query_pairs() {
                let value = value.into_owned();
                match key.as_ref() {
                    "name" => offer.name = Some(value),
                    "username" => offer.username = Some(value),
                    "password" => offer.password = Some(value),
                    _ => offer.values.push((key.into_owned(), value)),
                }
            }
            DeepLink::AddSource(offer)
        }
        _ => return None,
    })
}

/// Carry a link out: a page opens, a search runs, the queue changes, or a form opens for the person to save.
pub fn open(link: DeepLink, window: &mut Window, cx: &mut App) {
    match link {
        DeepLink::Open(route) => Navigator::navigate(route, cx),
        DeepLink::Search(query) => {
            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_search(query, cx));
            Navigator::navigate(Route::Discover, cx);
        }
        DeepLink::Play(Item::Track(key)) => actions::play_tracks(vec![key], None, cx),
        DeepLink::Play(Item::Album(id)) => {
            actions::play_context(QueueContext::Album { id }, None, cx)
        }
        DeepLink::Play(Item::Artist(artist)) => {
            actions::play_context(QueueContext::Artist { artist }, None, cx)
        }
        DeepLink::Play(Item::Playlist(id)) => {
            actions::play_context(QueueContext::Playlist { id }, None, cx)
        }
        DeepLink::Queue(key) => actions::append(vec![key], cx),
        DeepLink::AddSource(offer) => crate::ADD_SOURCE_OFFER.open(offer, window, cx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_open_by_name() {
        assert_eq!(
            parse("kopuz://open/library"),
            Some(DeepLink::Open(Route::Library))
        );
        assert_eq!(
            parse("kopuz://open/settings/appearance/"),
            Some(DeepLink::Open(Route::Settings {
                section: Some("appearance".into())
            }))
        );
        assert_eq!(parse("kopuz://open/nowhere"), None);
        assert_eq!(parse("kopuz://open"), None);
    }

    #[test]
    fn an_id_is_decoded_whole() {
        assert_eq!(
            parse("kopuz://album/a%2Fb%20c"),
            Some(DeepLink::Open(Route::Album("a/b c".into())))
        );
        assert_eq!(
            parse("kopuz://play/track/local%3A%2Fmusic%2Fone.flac"),
            Some(DeepLink::Play(Item::Track("local:/music/one.flac".into())))
        );
        assert_eq!(parse("kopuz://album/a/b"), None);
    }

    #[test]
    fn play_and_queue_name_what_they_act_on() {
        assert_eq!(
            parse("kopuz://play/playlist/42"),
            Some(DeepLink::Play(Item::Playlist("42".into())))
        );
        assert_eq!(
            parse("kopuz://queue/track/t1"),
            Some(DeepLink::Queue("t1".into()))
        );
        assert_eq!(parse("kopuz://queue/album/a1"), None);
        assert_eq!(parse("kopuz://play/genre/rock"), None);
        assert_eq!(parse("kopuz://play/track"), None);
    }

    #[test]
    fn a_search_needs_something_to_look_for() {
        assert_eq!(
            parse("kopuz://search?q=blue+monday%21"),
            Some(DeepLink::Search("blue monday!".into()))
        );
        assert_eq!(parse("kopuz://search?q=%20"), None);
        assert_eq!(parse("kopuz://search"), None);
    }

    #[test]
    fn a_source_offer_keeps_its_credentials_apart_from_the_form() {
        let link = "kopuz://add-source/subsonic?name=Home&url=https%3A%2F%2Fmusic.example&username=alice&password=p%26ss";
        let Some(DeepLink::AddSource(offer)) = parse(link) else {
            panic!("not a source offer");
        };
        assert_eq!(offer.service, "subsonic");
        assert_eq!(offer.name.as_deref(), Some("Home"));
        assert_eq!(
            offer.values,
            [("url".to_string(), "https://music.example".to_string())]
        );
        assert_eq!(offer.username.as_deref(), Some("alice"));
        assert_eq!(offer.password.as_deref(), Some("p&ss"));
        assert!(!format!("{offer:?}").contains("p&ss"));
        assert_eq!(parse("kopuz://add-source"), None);
    }

    #[test]
    fn only_kopuz_links_are_read_and_a_listen_link_is_the_daemons() {
        assert_eq!(
            parse("Kopuz://OPEN/home"),
            Some(DeepLink::Open(Route::Home))
        );
        assert_eq!(parse("https://open/home"), None);
        assert!(is_listen("kopuz://j/1/ytmusic/p0/abc"));
        assert!(is_listen("KOPUZ://j/x"));
        assert!(!is_listen("kopuz://jellyfin/x"));
        assert!(!is_listen("https://kopuz.moe/j#1"));
        assert_eq!(parse("kopuz://j/1/ytmusic/p0/abc"), None);
    }
}
