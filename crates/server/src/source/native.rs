//! The official plugins that run in-process.
//!
//! These backends need what the sandbox does not give a component: V8 for
//! YouTube's signatures and PoTokens, a Widevine CDM, a browser for Spotify's
//! playback, raw sockets for SMB, the filesystem for folders and Nextcloud's
//! cover cache. They register in the plugin registry like any other plugin,
//! with a manifest, and the registry dispatches to them; their code is kopuz's
//! own. A component installed with one's id replaces it.

use config::Source;
use db::Db;

use super::{
    AppleMusicSource, LocalSource, MediaSource, NextcloudSource, SmbSource, SpotifySource, YtSource,
};
use crate::server_ops::ServerConn;

pub(crate) struct Native {
    pub(crate) manifest: &'static str,
    open: fn(Db, Source, &ServerConn) -> Box<dyn MediaSource>,
}

macro_rules! manifest {
    ($id:literal, $name:literal, $description:literal, [$($service:literal),*]) => {
        concat!(
            "id = \"", $id, "\"\n",
            "name = \"", $name, "\"\n",
            "version = \"", env!("CARGO_PKG_VERSION"), "\"\n",
            "api = \"0.1\"\n",
            "description = \"", $description, "\"\n",
            $("[[service]]\nid = \"", $service, "\"\n",)*
        )
    };
}

pub(crate) const PLUGINS: &[Native] = &[
    Native {
        manifest: manifest!(
            "ytmusic",
            "YouTube Music",
            "YouTube Music, signed in or anonymous.",
            ["ytmusic"]
        ),
        open: |db, source, conn| Box::new(YtSource::new(db, source, conn)),
    },
    Native {
        manifest: manifest!(
            "applemusic",
            "Apple Music",
            "Apple Music with a subscription.",
            ["applemusic"]
        ),
        open: |db, source, conn| {
            // The Widevine CDM will be wanted; fetch it before a track waits on it.
            crate::applemusic::widevine::fetch::prefetch();
            Box::new(AppleMusicSource::new(
                db,
                source,
                crate::applemusic::AppleMusicApi::new(
                    Some(conn.token.clone()),
                    &conn.apple_music_storefront,
                    &conn.apple_music_language,
                ),
            ))
        },
    },
    Native {
        manifest: manifest!(
            "spotify",
            "Spotify",
            "Spotify, played through its web player.",
            ["spotify"]
        ),
        open: |db, source, conn| Box::new(SpotifySource::new(db, source, conn)),
    },
    Native {
        manifest: manifest!(
            "nextcloud",
            "Nextcloud",
            "Music files in a Nextcloud.",
            ["nextcloud"]
        ),
        open: |db, source, conn| Box::new(NextcloudSource::new(db, source, conn)),
    },
    Native {
        manifest: manifest!("smb", "SMB / NAS", "Music on an SMB share.", ["smb"]),
        open: |db, source, conn| Box::new(SmbSource::new(db, source, conn)),
    },
    Native {
        manifest: manifest!(
            "folders",
            "Folders",
            "Music files in folders on this computer.",
            ["folders"]
        ),
        open: |db, source, _| Box::new(LocalSource { db, source }),
    },
];

/// The native plugin with `id`, opened on `conn`.
pub(crate) fn open(
    id: &str,
    db: Db,
    source: Source,
    conn: &ServerConn,
) -> Option<Box<dyn MediaSource>> {
    let manifest = |native: &&Native| {
        plugin_host::manifest::Manifest::parse(native.manifest)
            .is_ok_and(|manifest| manifest.id == id)
    };
    PLUGINS
        .iter()
        .find(manifest)
        .map(|native| (native.open)(db, source, conn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::MusicService;

    #[test]
    fn every_native_manifest_parses_and_names_a_built_in_service() {
        for native in PLUGINS {
            let manifest =
                plugin_host::manifest::Manifest::parse(native.manifest).expect("valid manifest");
            for service in &manifest.services {
                assert!(
                    MusicService::from_id(&service.id).is_some() || service.id == "folders",
                    "{}",
                    service.id
                );
            }
        }
    }
}
