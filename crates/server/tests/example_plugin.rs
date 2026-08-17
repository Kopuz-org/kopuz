//! The example plugin that ships in `examples/plugins/example`, driven through
//! the real runtime.
//!
//! It is a deliverable, not a fixture: it is what a plugin author copies and
//! reads, and `docs/plugins.md` points at it. A silent break in it is worse than
//! a break in a test-local script, so it gets exercised here.
//!
//! Its own binary rather than another test in `lua_plugin.rs`: both set the
//! process-wide `KOPUZ_PLUGIN_PATH`, and cargo gives each integration test file
//! its own process.

use std::path::{Path, PathBuf};

use server::plugin::{self, AuthPrompt, AuthValues, PluginError, registry};
use server::source::{AlbumType, ArtistView, FavoritesSync, PlaylistOps};

const ID: &str = "example";

/// Signing in writes through `kopuz.store`, and a plugin's data directory
/// resolves against the real config directory with no override. So the test
/// borrows that file and puts it back: a developer with the example genuinely
/// installed and signed in keeps their session.
struct StoreGuard {
    dir: PathBuf,
    store: PathBuf,
    saved: Option<Vec<u8>>,
    dir_existed: bool,
}

impl StoreGuard {
    fn take() -> Self {
        let dir = plugin::manifest::data_dir_for(ID);
        let store = dir.join("store.json");
        Self {
            dir_existed: dir.exists(),
            saved: std::fs::read(&store).ok(),
            dir,
            store,
        }
    }
}

impl Drop for StoreGuard {
    fn drop(&mut self) {
        match &self.saved {
            Some(bytes) => {
                let _ = std::fs::write(&self.store, bytes);
            }
            None if self.dir_existed => {
                let _ = std::fs::remove_file(&self.store);
            }
            None => {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/plugins")
        .canonicalize()
        .expect("the shipped examples directory must exist")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shipped_example_plugin_works() {
    let _store = StoreGuard::take();
    // SAFETY: the only test in this binary, and nothing has read the environment
    // yet because the registry is untouched.
    unsafe { std::env::set_var("KOPUZ_PLUGIN_PATH", examples_root()) };

    let registry = registry();
    registry.rescan();

    // The user's own plugins directory stays in the search path, so this asserts
    // by id and never by count.
    let Some(manifest) = registry.manifests().into_iter().find(|m| m.id == ID) else {
        panic!("the shipped example must be discoverable");
    };
    assert_eq!(manifest.api, plugin::API_VERSION);

    // Loading proves the entry chunk parses under the sandbox and that its
    // top-level `require("lib.catalog")` resolved.
    let example = registry
        .instance(ID)
        .await
        .expect("the shipped example must load");

    // The capabilities the plugin declares are the ones its file claims. A
    // capability it does not declare must not appear.
    let caps = registry
        .cached_capabilities(ID)
        .expect("setup() must reach the registry");
    assert!(caps.sync);
    assert_eq!(caps.playlists, PlaylistOps::None);
    assert_eq!(caps.artist_view, ArtistView::Library);
    assert_eq!(caps.albums, AlbumType::Standard);
    assert_eq!(caps.favorites_sync, FavoritesSync::Instant);
    assert!(!caps.discover && !caps.radio && !caps.downloads);

    // Deliberately not exported, and documented as such in the plugin.
    for absent in ["discover_home", "discover_continuation", "start_radio"] {
        assert!(
            !example.exports(absent),
            "{absent} is documented as left out"
        );
    }

    sign_in(ID).await;

    let library: server::plugin::LibraryResult = example
        .call("fetch_library", ())
        .await
        .expect("fetch_library must answer once signed in");
    assert_eq!(library.albums.len(), 3);
    assert_eq!(library.tracks.len(), 3);
    assert!(!library.artist_images.is_empty());

    let found: server::plugin::SearchResult = example
        .call("search", ("chopin".to_string(), plugin::SEARCH_LIMIT))
        .await
        .expect("search must answer");
    assert!(
        found.tracks.iter().any(|t| t.item_id == "chopin-op27-1"),
        "searching for chopin must find the nocturne, got {:?}",
        found.tracks.iter().map(|t| &t.item_id).collect::<Vec<_>>()
    );

    // An empty query is the plugin's own invalid_input case.
    assert!(matches!(
        example
            .call::<_, server::plugin::SearchResult>("search", ("  ".to_string(), 10u32))
            .await,
        Err(PluginError::InvalidInput(_))
    ));

    // Favorites round-trip through the plugin's own store.
    example
        .call_unit("push_favorite", ("chopin-op27-1".to_string(), true))
        .await
        .expect("favoriting a real track must succeed");
    let favorites: Vec<String> = example
        .call("fetch_favorites", ())
        .await
        .expect("fetch_favorites must answer");
    assert_eq!(favorites, ["chopin-op27-1"]);
    assert!(
        matches!(
            example
                .call_unit("push_favorite", ("no-such-track".to_string(), true))
                .await,
            Err(PluginError::InvalidInput(_))
        ),
        "favoriting an unknown id must be rejected"
    );

    walk_the_playlist(&example).await;

    let stream: server::plugin::StreamResult = example
        .call("resolve_stream", "chopin-op27-1".to_string())
        .await
        .expect("resolve_stream must answer");
    assert!(
        stream.url.starts_with("https://"),
        "a stream URL must be fetchable, got {}",
        stream.url
    );
    assert!(stream.content_length.is_some_and(|n| n > 0));
    assert!(stream.user_agent.is_some());

    plugin::shutdown_all().await;
}

/// The wizard the settings page drives: a form, then an acknowledgement, then
/// done. Goes through the same entry points the UI uses.
async fn sign_in(id: &str) {
    let AuthPrompt::Form { fields, .. } = plugin::auth_begin(id).await.expect("auth_begin") else {
        panic!("the example opens its wizard with a form");
    };
    let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["user", "password"]);
    assert!(
        fields.iter().any(|f| f.key == "password" && f.secret),
        "a password field must be marked secret"
    );

    let mut values = AuthValues::new();
    values.insert("user".to_string(), "tester".to_string());
    values.insert("password".to_string(), "anything".to_string());
    let AuthPrompt::Message { .. } = plugin::auth_submit(id, values)
        .await
        .expect("submitting credentials")
    else {
        panic!("the example confirms the sign-in with a message step");
    };

    assert_eq!(
        plugin::auth_submit(id, AuthValues::new())
            .await
            .expect("acknowledging the message"),
        AuthPrompt::Done
    );

    // An empty username is the plugin's own failure path, and it must not end the
    // wizard on the host's side.
    let mut blank = AuthValues::new();
    blank.insert("user".to_string(), "   ".to_string());
    assert!(matches!(
        plugin::auth_begin(id).await.expect("restarting the wizard"),
        AuthPrompt::Form { .. }
    ));
    assert!(matches!(
        plugin::auth_submit(id, blank)
            .await
            .expect("blank username"),
        AuthPrompt::Failed { .. }
    ));
}

/// The example pages one track at a time on purpose, so the host's cursor loop
/// gets walked rather than short-circuited by a single full page.
async fn walk_the_playlist(example: &server::plugin::PluginInstance) {
    let playlists: Vec<server::plugin::PluginPlaylistMeta> = example
        .call("fetch_playlists", ())
        .await
        .expect("fetch_playlists must answer");
    let Some(first) = playlists.first() else {
        panic!("the example ships a playlist");
    };

    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();
    loop {
        let page: server::plugin::TrackPage = example
            .call(
                "fetch_playlist_entries_page",
                (first.playlist_id.clone(), cursor.clone()),
            )
            .await
            .expect("a playlist page must answer");
        seen.extend(page.tracks.iter().map(|t| t.item_id.clone()));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(seen.len() < 16, "the cursor walk must terminate");
    }
    assert!(
        seen.len() > 1,
        "the example pages one track at a time, so the walk must take several pages"
    );
    assert!(
        seen.iter().all(|id| !id.contains('/')),
        "a plugin only ever sees its own bare refs, got {seen:?}"
    );
}
