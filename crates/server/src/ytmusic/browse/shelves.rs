//! Shelf renderers, each one shelf, and the page headers above them.

use serde_json::{Value, json};

use super::items::{item, items, two_row_item};
use super::{continuation, renderer, runs, text, thumbnail};
use crate::ytmusic::discover::{DiscoverItem, DiscoverShelf, PageHeader, PageLink, ShelfLayout};

/// A `sectionListRenderer.contents` array.
pub(super) fn sections(contents: &Value) -> Vec<DiscoverShelf> {
    let mut out = Vec::new();
    // Search's All tab sometimes sends each result as its own header-less
    // itemSectionRenderer; runs of those are gathered into one list shelf.
    let mut loose = Vec::new();
    for entry in contents.as_array().into_iter().flatten() {
        let Some((key, r)) = renderer(entry) else {
            continue;
        };
        if key == "itemSectionRenderer" {
            for inner in r["contents"].as_array().into_iter().flatten() {
                match renderer(inner) {
                    Some((
                        "musicResponsiveListItemRenderer" | "musicMultiRowListItemRenderer",
                        _,
                    )) => loose.extend(item(inner)),
                    Some((inner_key, inner_r)) => {
                        flush(&mut loose, &mut out);
                        out.extend(section(inner_key, inner_r));
                    }
                    None => {}
                }
            }
            continue;
        }
        flush(&mut loose, &mut out);
        out.extend(section(key, r));
    }
    flush(&mut loose, &mut out);
    out
}

fn flush(loose: &mut Vec<DiscoverItem>, out: &mut Vec<DiscoverShelf>) {
    if !loose.is_empty() {
        out.push(shelf_of(ShelfLayout::List, std::mem::take(loose)));
    }
}

fn shelf_of(layout: ShelfLayout, items: Vec<DiscoverItem>) -> DiscoverShelf {
    DiscoverShelf {
        title: String::new(),
        strapline: None,
        more: None,
        items,
        layout,
        continuation: None,
        search_filter: None,
    }
}

/// One shelf, or `None` for renderers that carry no items (descriptions,
/// messages, the sign-in prompt) and for shelves that came back empty.
pub(super) fn section(key: &str, r: &Value) -> Option<DiscoverShelf> {
    let shelf = match key {
        "musicCarouselShelfRenderer" | "musicImmersiveCarouselShelfRenderer" => carousel(r),
        "musicShelfRenderer" => list(r),
        "musicPlaylistShelfRenderer" => DiscoverShelf {
            continuation: continuation(r),
            ..shelf_of(ShelfLayout::List, items(&r["contents"]))
        },
        "gridRenderer" => DiscoverShelf {
            title: text(&r["header"]["gridHeaderRenderer"]["title"]).unwrap_or_default(),
            continuation: continuation(r),
            ..shelf_of(ShelfLayout::Grid, items(&r["items"]))
        },
        "musicCardShelfRenderer" => card(r),
        _ => {
            tracing::debug!(renderer = key, "skipping shelf renderer");
            return None;
        }
    };
    (!shelf.items.is_empty() || shelf.continuation.is_some()).then_some(shelf)
}

fn carousel(r: &Value) -> DiscoverShelf {
    let header = match &r["header"]["musicCarouselShelfBasicHeaderRenderer"] {
        basic if basic.is_object() => basic,
        _ => &r["header"]["musicImmersiveCarouselShelfHeaderRenderer"],
    };
    let layout = match r["contents"][0].as_object().and_then(|o| o.keys().next()) {
        Some(key) if key == "musicResponsiveListItemRenderer" => ShelfLayout::TrackGrid,
        Some(key) if key == "musicNavigationButtonRenderer" => ShelfLayout::Grid,
        _ => ShelfLayout::Carousel,
    };
    DiscoverShelf {
        title: text(&header["title"]).unwrap_or_default(),
        strapline: text(&header["strapline"]),
        more: PageLink::endpoint(
            &header["moreContentButton"]["buttonRenderer"]["navigationEndpoint"],
        )
        .or_else(|| title_link(&header["title"])),
        continuation: continuation(r),
        ..shelf_of(layout, items(&r["contents"]))
    }
}

fn list(r: &Value) -> DiscoverShelf {
    DiscoverShelf {
        title: text(&r["title"]).unwrap_or_default(),
        more: PageLink::endpoint(&r["bottomEndpoint"]).or_else(|| title_link(&r["title"])),
        continuation: continuation(r),
        ..shelf_of(ShelfLayout::List, items(&r["contents"]))
    }
}

fn title_link(title: &Value) -> Option<PageLink> {
    PageLink::endpoint(&runs(title).first()?["navigationEndpoint"])
}

/// The search "Top result" card: one big item, then a few rows under it.
fn card(r: &Value) -> DiscoverShelf {
    let endpoint = runs(&r["title"])
        .first()
        .map(|run| &run["navigationEndpoint"])
        .filter(|endpoint| endpoint.is_object())
        .unwrap_or(&r["onTap"]);
    let as_tile = json!({
        "title": r["title"],
        "subtitle": r["subtitle"],
        "navigationEndpoint": endpoint,
        "thumbnailRenderer": r["thumbnail"],
    });
    let mut found: Vec<DiscoverItem> = two_row_item(&as_tile).into_iter().collect();
    found.extend(items(&r["contents"]));
    DiscoverShelf {
        title: text(&r["header"]["musicCardShelfHeaderBasicRenderer"]["title"]).unwrap_or_default(),
        ..shelf_of(ShelfLayout::Hero, found)
    }
}

/// What heads a page.
#[derive(Debug, Default)]
pub(super) struct Header {
    pub kind: PageHeader,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub playback_id: Option<String>,
    pub owner: Option<String>,
    pub plays: Option<String>,
}

pub(super) fn header(v: &Value) -> Option<Header> {
    let (key, r) = renderer(v)?;
    match key {
        "musicImmersiveHeaderRenderer" | "musicVisualHeaderRenderer" => {
            let subscribe = &r["subscriptionButton"]["subscribeButtonRenderer"];
            Some(Header {
                kind: PageHeader::Artist,
                title: text(&r["title"])?,
                subtitle: text(&subscribe["longSubscriberCountText"])
                    .or_else(|| text(&subscribe["subscriberCountText"])),
                description: text(&r["description"]),
                thumbnail: thumbnail(&r["thumbnail"]).or_else(|| thumbnail(&r["foregroundThumbnail"])),
                playback_id: r["playButton"]["buttonRenderer"]["navigationEndpoint"]["watchEndpoint"]
                    ["playlistId"]
                    .as_str()
                    .map(str::to_string),
                ..Header::default()
            })
        }
        "musicResponsiveHeaderRenderer" | "musicDetailHeaderRenderer" => {
            let playback_id = r["buttons"].as_array().into_iter().flatten().find_map(|b| {
                let endpoint = &b["musicPlayButtonRenderer"]["playNavigationEndpoint"];
                endpoint["watchPlaylistEndpoint"]["playlistId"]
                    .as_str()
                    .or_else(|| endpoint["watchEndpoint"]["playlistId"].as_str())
                    .map(str::to_string)
            });
            let subtitle: Vec<String> = [&r["straplineTextOne"], &r["subtitle"]]
                .into_iter()
                .filter_map(text)
                .collect();
            let description = &r["description"];
            let owner = r["facepile"]["avatarStackViewModel"]["text"]["content"]
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .or_else(|| text(&r["straplineTextOne"]));
            let plays = runs(&r["secondSubtitle"])
                .iter()
                .filter_map(|run| run["text"].as_str())
                .map(str::trim)
                .find(|part| crate::ytmusic::is_count(part))
                .map(str::to_string);
            Some(Header {
                kind: PageHeader::Detail,
                title: text(&r["title"])?,
                subtitle: (!subtitle.is_empty()).then(|| subtitle.join(" • ")),
                description: text(&description["musicDescriptionShelfRenderer"]["description"])
                    .or_else(|| text(description)),
                thumbnail: thumbnail(&r["thumbnail"]),
                playback_id,
                owner,
                plays,
            })
        }
        "musicEditablePlaylistDetailHeaderRenderer" => header(&r["header"]),
        "musicHeaderRenderer" => Some(Header {
            kind: PageHeader::Title,
            title: text(&r["title"])?,
            ..Header::default()
        }),
        _ => {
            tracing::debug!(renderer = key, "skipping header renderer");
            None
        }
    }
}
