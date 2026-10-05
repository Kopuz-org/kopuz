//! Album, artist and track listings filter, sort and page in SQL: the filter
//! decides `total`, the order is the one asked for in the direction asked for,
//! and a window is cut only after both.

use db::{
    AlbumQuery, AlbumSort, AlbumSortField, ArtistQuery, ArtistSort, ArtistSortField, Page, Source,
    TrackFilter, TrackSort,
};
use reader::models::{Album, Track, TrackId};

const ALL: Page = Page {
    offset: 0,
    limit: u32::MAX,
};

fn track(key: &str, title: &str, artist: &str, album_id: &str) -> Track {
    Track {
        id: TrackId::Local(std::path::PathBuf::from(key)),
        cover: None,
        album_id: album_id.to_string(),
        title: title.to_string(),
        artist: artist.to_string(),
        album: String::new(),
        duration: 60,
        khz: 44,
        bitrate: 320,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec![artist.to_string()],
        credits: Vec::new(),
        replay_gain: config::ReplayGainInfo::default(),
    }
}

fn album(id: &str, title: &str, artist: &str, genre: &str, year: u16) -> Album {
    Album {
        id: id.to_string(),
        title: title.to_string(),
        artist: artist.to_string(),
        genre: genre.to_string(),
        year,
        cover_path: None,
        manual_cover: false,
        artist_id: None,
        artist_key: None,
    }
}

/// Five albums over three artists; `beta` is lower-case to prove the order folds case.
///
///  album   artist  genre  year  tracks (oldest to newest added)
///  Alpha   Ada     Rock   1991  a1 a2 a3
///  beta    Boris   Jazz   2005  b1 b2
///  Delta   Ada     Rock   2005  d1
///  Epsilon Cyd     -      0     e1
///  100% Pure Cyd   Pop    2020  p1
async fn library() -> (db::Db, tempfile::TempDir, Source) {
    let dir = tempfile::tempdir().unwrap();
    let database = db::init(&dir.path().join("listing.db")).await.unwrap();
    let source = Source::default();
    let tracks = [
        track("/a1.flac", "Charlie", "Ada", "al-alpha"),
        track("/a2.flac", "bravo", "Ada", "al-alpha"),
        track("/a3.flac", "Alfa", "Ada", "al-alpha"),
        track("/b1.flac", "Echo", "Boris", "al-beta"),
        track("/b2.flac", "Delta", "Boris", "al-beta"),
        track("/d1.flac", "Foxtrot", "Ada", "al-delta"),
        track("/e1.flac", "Golf", "Cyd", "al-epsilon"),
        track("/p1.flac", "Hotel", "Cyd", "al-pure"),
    ];
    database.upsert_tracks(&source, &tracks).await.unwrap();
    database
        .upsert_albums(
            &source,
            &[
                album("al-alpha", "Alpha", "Ada", "Rock", 1991),
                album("al-beta", "beta", "Boris", "Jazz", 2005),
                album("al-delta", "Delta", "Ada", "Rock", 2005),
                album("al-epsilon", "Epsilon", "Cyd", "", 0),
                album("al-pure", "100% Pure", "Cyd", "Pop", 2020),
            ],
        )
        .await
        .unwrap();
    // Beta's tracks are the newest, though its rows were filed first.
    let stamps: Vec<(String, i64)> = [
        "/a1.flac", "/a2.flac", "/a3.flac", "/d1.flac", "/e1.flac", "/p1.flac", "/b1.flac",
        "/b2.flac",
    ]
    .iter()
    .enumerate()
    .map(|(n, key)| (key.to_string(), 1_000 + n as i64))
    .collect();
    database.stamp_added_at(&source, &stamps).await.unwrap();
    (database, dir, source)
}

fn albums(source: &Source) -> AlbumQuery {
    AlbumQuery {
        source: source.clone(),
        ..Default::default()
    }
}

fn artists(source: &Source) -> ArtistQuery {
    ArtistQuery {
        source: source.clone(),
        ..Default::default()
    }
}

fn sorted(field: AlbumSortField, descending: bool) -> Vec<AlbumSort> {
    vec![AlbumSort { field, descending }]
}

async fn titles(database: &db::Db, query: &AlbumQuery) -> Vec<String> {
    database
        .albums_page(query, ALL)
        .await
        .unwrap()
        .rows
        .into_iter()
        .map(|album| album.title)
        .collect()
}

#[tokio::test]
async fn albums_list_by_artist_then_title_unless_told_otherwise() {
    let (database, _dir, source) = library().await;

    let listing = database.albums_page(&albums(&source), ALL).await.unwrap();

    assert_eq!(listing.total, 5);
    let titles: Vec<&str> = listing.rows.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Alpha", "Delta", "beta", "100% Pure", "Epsilon"],
        "Ada (Alpha, Delta), Boris, Cyd (100% Pure, Epsilon)"
    );
    assert!(
        listing.rows.iter().all(|a| a.artist_key.is_some()),
        "every album carries the key of the artist it bills"
    );
}

#[tokio::test]
async fn albums_filter_by_search_genre_year_and_artist() {
    let (database, _dir, source) = library().await;

    let search = |text: &str| AlbumQuery {
        search: text.into(),
        ..albums(&source)
    };
    assert_eq!(titles(&database, &search("ALP")).await, ["Alpha"]);
    assert_eq!(
        titles(&database, &search("ada")).await,
        ["Alpha", "Delta"],
        "the billed artist's text is searched too"
    );
    assert_eq!(
        titles(&database, &search("100%")).await,
        ["100% Pure"],
        "a percent sign is not a wildcard"
    );
    assert_eq!(titles(&database, &search("_lpha")).await, [] as [&str; 0]);
    assert_eq!(
        titles(&database, &search("   ")).await.len(),
        5,
        "blank search filters nothing"
    );

    let rock = AlbumQuery {
        genre: Some("Rock".into()),
        ..albums(&source)
    };
    assert_eq!(titles(&database, &rock).await, ["Alpha", "Delta"]);

    let between = |from, to| AlbumQuery {
        year_from: from,
        year_to: to,
        ..albums(&source)
    };
    assert_eq!(
        titles(&database, &between(Some(2005), Some(2005))).await,
        ["Delta", "beta"],
        "both bounds are inclusive"
    );
    assert_eq!(
        titles(&database, &between(None, Some(1999))).await,
        ["Alpha"],
        "an album with no year is in no range"
    );
    assert_eq!(
        titles(&database, &between(Some(2006), None)).await,
        ["100% Pure"]
    );

    let ada = database
        .artists_page(
            &ArtistQuery {
                search: "ada".into(),
                ..artists(&source)
            },
            ALL,
        )
        .await
        .unwrap()
        .rows
        .remove(0);
    let by_ada = AlbumQuery {
        artist_key: Some(ada.key),
        ..albums(&source)
    };
    assert_eq!(titles(&database, &by_ada).await, ["Alpha", "Delta"]);

    let nobody = AlbumQuery {
        artist_key: Some(String::new()),
        ..albums(&source)
    };
    assert!(titles(&database, &nobody).await.is_empty());

    let all_of_them = AlbumQuery {
        search: "a".into(),
        genre: Some("Rock".into()),
        year_from: Some(2000),
        ..albums(&source)
    };
    assert_eq!(
        titles(&database, &all_of_them).await,
        ["Delta"],
        "filters narrow together"
    );
}

#[tokio::test]
async fn albums_are_scoped_to_their_source() {
    let (database, _dir, _source) = library().await;
    let elsewhere = Source::Server("srv-1".into());

    let listing = database
        .albums_page(&albums(&elsewhere), ALL)
        .await
        .unwrap();

    assert_eq!((listing.total, listing.rows.len()), (0, 0));
}

#[tokio::test]
async fn albums_sort_each_field_in_both_directions() {
    let (database, _dir, source) = library().await;
    let ordered = |field, descending| AlbumQuery {
        sort: sorted(field, descending),
        ..albums(&source)
    };

    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Title, false)).await,
        ["100% Pure", "Alpha", "beta", "Delta", "Epsilon"],
        "case does not decide the order"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Title, true)).await,
        ["Epsilon", "Delta", "beta", "Alpha", "100% Pure"]
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Artist, true)).await,
        ["100% Pure", "Epsilon", "beta", "Alpha", "Delta"],
        "Cyd, Boris, Ada; the default order breaks the ties"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Year, false)).await,
        ["Epsilon", "Alpha", "Delta", "beta", "100% Pure"],
        "no year sorts first ascending; 2005 ties by artist"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Year, true)).await,
        ["100% Pure", "Delta", "beta", "Alpha", "Epsilon"]
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::Genre, false)).await,
        ["Epsilon", "beta", "100% Pure", "Alpha", "Delta"],
        "no genre, Jazz, Pop, Rock"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::TrackCount, true)).await,
        ["Alpha", "beta", "Delta", "100% Pure", "Epsilon"],
        "3, 2, then the single-track albums in the default order"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::TrackCount, false)).await,
        ["Delta", "100% Pure", "Epsilon", "beta", "Alpha"]
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::RecentlyAdded, true)).await,
        ["beta", "100% Pure", "Epsilon", "Delta", "Alpha"],
        "an album is as recent as its newest track"
    );
    assert_eq!(
        titles(&database, &ordered(AlbumSortField::RecentlyAdded, false)).await,
        ["Alpha", "Delta", "Epsilon", "100% Pure", "beta"]
    );
}

#[tokio::test]
async fn album_sort_criteria_stack() {
    let (database, _dir, source) = library().await;
    let query = AlbumQuery {
        sort: vec![
            AlbumSort {
                field: AlbumSortField::Year,
                descending: true,
            },
            AlbumSort {
                field: AlbumSortField::Title,
                descending: true,
            },
        ],
        ..albums(&source)
    };

    assert_eq!(
        titles(&database, &query).await,
        ["100% Pure", "Delta", "beta", "Alpha", "Epsilon"],
        "the year decides, then the title within 2005"
    );
}

#[tokio::test]
async fn album_totals_count_the_filtered_set_not_the_window() {
    let (database, _dir, source) = library().await;
    let query = AlbumQuery {
        year_from: Some(1990),
        sort: sorted(AlbumSortField::Year, false),
        ..albums(&source)
    };

    let first = database
        .albums_page(
            &query,
            Page {
                offset: 0,
                limit: 2,
            },
        )
        .await
        .unwrap();
    let rest = database
        .albums_page(
            &query,
            Page {
                offset: 2,
                limit: 2,
            },
        )
        .await
        .unwrap();
    let past_the_end = database
        .albums_page(
            &query,
            Page {
                offset: 40,
                limit: 2,
            },
        )
        .await
        .unwrap();

    assert_eq!(first.total, 4, "the album with no year is filtered out");
    assert_eq!(rest.total, 4);
    let title = |listing: &db::Listing<Album>| -> Vec<String> {
        listing.rows.iter().map(|a| a.title.clone()).collect()
    };
    assert_eq!(title(&first), ["Alpha", "Delta"]);
    assert_eq!(title(&rest), ["beta", "100% Pure"]);
    assert_eq!(past_the_end.total, 4, "a window past the end still totals");
    assert!(past_the_end.rows.is_empty());
}

#[tokio::test]
async fn artists_filter_sort_and_count() {
    let (database, _dir, source) = library().await;
    let names = |listing: db::Listing<db::ArtistRow>| -> Vec<String> {
        listing.rows.into_iter().map(|a| a.name).collect()
    };
    let ordered = |field, descending| ArtistQuery {
        sort: vec![ArtistSort { field, descending }],
        ..artists(&source)
    };

    let by_name = database.artists_page(&artists(&source), ALL).await.unwrap();
    assert_eq!(by_name.total, 3);
    assert_eq!(names(by_name), ["Ada", "Boris", "Cyd"]);
    assert_eq!(
        names(
            database
                .artists_page(&ordered(ArtistSortField::Name, true), ALL)
                .await
                .unwrap()
        ),
        ["Cyd", "Boris", "Ada"]
    );

    let listing = database
        .artists_page(&ordered(ArtistSortField::TrackCount, true), ALL)
        .await
        .unwrap();
    let counts: Vec<(&str, u32, u32)> = listing
        .rows
        .iter()
        .map(|a| (a.name.as_str(), a.tracks, a.albums))
        .collect();
    assert_eq!(
        counts,
        [("Ada", 4, 2), ("Boris", 2, 1), ("Cyd", 2, 2)],
        "Boris and Cyd tie on tracks, so the name decides"
    );
    assert_eq!(
        names(
            database
                .artists_page(&ordered(ArtistSortField::AlbumCount, false), ALL)
                .await
                .unwrap()
        ),
        ["Boris", "Ada", "Cyd"],
        "one album, then Ada and Cyd tied on two by name"
    );

    let found = database
        .artists_page(
            &ArtistQuery {
                search: "BOR".into(),
                ..artists(&source)
            },
            ALL,
        )
        .await
        .unwrap();
    assert_eq!((found.total, names(found)), (1, vec!["Boris".to_string()]));
}

#[tokio::test]
async fn artist_totals_count_the_filtered_set_not_the_window() {
    let (database, _dir, source) = library().await;

    let window = database
        .artists_page(
            &ArtistQuery {
                search: "o".into(),
                sort: vec![ArtistSort {
                    field: ArtistSortField::Name,
                    descending: false,
                }],
                ..artists(&source)
            },
            Page {
                offset: 1,
                limit: 5,
            },
        )
        .await
        .unwrap();

    // Only Boris has an o in its name.
    assert_eq!(window.total, 1);
    assert!(window.rows.is_empty(), "offset 1 is past the one match");

    let wide = database
        .artists_page(
            &artists(&source),
            Page {
                offset: 1,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(wide.total, 3);
    assert_eq!(wide.rows.len(), 1);
    assert_eq!(wide.rows[0].name, "Boris");
}

fn key_of(track: &Track) -> String {
    track.id.key().into_owned()
}

async fn keys(database: &db::Db, filter: &TrackFilter) -> Vec<String> {
    database
        .tracks_page(filter, ALL)
        .await
        .unwrap()
        .iter()
        .map(key_of)
        .collect()
}

#[tokio::test]
async fn tracks_filter_by_download_and_album_year() {
    let (database, _dir, source) = library().await;
    database
        .set_offline_track("/a1.flac", Some("/offline/a1"))
        .await
        .unwrap();
    database
        .set_offline_track("/b2.flac", Some("/offline/b2"))
        .await
        .unwrap();
    let base = TrackFilter {
        sort: TrackSort::Title,
        ..TrackFilter::new(source)
    };
    let with = |edit: &dyn Fn(&mut TrackFilter)| {
        let mut filter = base.clone();
        edit(&mut filter);
        filter
    };

    let downloaded = with(&|f| f.downloaded = Some(true));
    assert_eq!(keys(&database, &downloaded).await, ["/a1.flac", "/b2.flac"]);
    assert_eq!(database.tracks_count(&downloaded).await.unwrap(), 2);
    let missing = with(&|f| f.downloaded = Some(false));
    assert_eq!(keys(&database, &missing).await.len(), 6);
    assert_eq!(database.tracks_count(&missing).await.unwrap(), 6);

    let nineties = with(&|f| f.year_to = Some(1999));
    assert_eq!(
        keys(&database, &nineties).await,
        ["/a3.flac", "/a2.flac", "/a1.flac"],
        "a track is as old as its album"
    );
    assert_eq!(database.tracks_count(&nineties).await.unwrap(), 3);
    let noughties = with(&|f| {
        f.year_from = Some(2000);
        f.year_to = Some(2010);
    });
    assert_eq!(database.tracks_count(&noughties).await.unwrap(), 3);
    let none = with(&|f| f.year_from = Some(2030));
    assert!(keys(&database, &none).await.is_empty());

    let both = with(&|f| {
        f.downloaded = Some(true);
        f.year_from = Some(2000);
    });
    assert_eq!(keys(&database, &both).await, ["/b2.flac"]);
    assert_eq!(database.tracks_count(&both).await.unwrap(), 1);
}

#[tokio::test]
async fn tracks_run_backwards_when_asked() {
    let (database, _dir, source) = library().await;
    let forward = |sort| TrackFilter {
        sort,
        ..TrackFilter::new(source.clone())
    };
    let backward = |sort| TrackFilter {
        reverse: true,
        ..forward(sort)
    };

    for sort in [
        TrackSort::Title,
        TrackSort::ArtistAlbum,
        TrackSort::DateAdded,
        TrackSort::Fields(vec![config::SortCriterion::new(
            config::TrackSortField::Title,
            config::SortDirection::Desc,
        )]),
    ] {
        let mut expected = keys(&database, &forward(sort.clone())).await;
        expected.reverse();
        assert_eq!(keys(&database, &backward(sort.clone())).await, expected);
    }

    let page = database
        .tracks_page(
            &backward(TrackSort::Title),
            Page {
                offset: 1,
                limit: 2,
            },
        )
        .await
        .unwrap();
    let titles: Vec<&str> = page.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Golf", "Foxtrot"],
        "a window is cut after the order flips"
    );
}
