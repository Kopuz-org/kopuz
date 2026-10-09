use super::*;

#[test]
fn smb_locations_keep_share_and_subfolder_separate() {
    let location = Location::parse("smb://nas/Music/Artist%20Name/%C3%96zel/").unwrap();
    assert_eq!(location.address, "nas:445");
    assert_eq!(location.share, "Music");
    assert_eq!(location.path("").unwrap(), "Artist Name/Özel");
    assert_eq!(
        location.path("Disc 1/01.flac").unwrap(),
        "Artist Name/Özel/Disc 1/01.flac"
    );
    let ipv6 = Location::parse("smb://[::1]:1445/Music").unwrap();
    assert_eq!(ipv6.address, "[::1]:1445");
    assert_eq!(ipv6.path("song.mp3").unwrap(), "song.mp3");
}

#[test]
fn smb_locations_reject_ambiguous_roots_and_embedded_credentials() {
    for value in [
        "https://nas/Music",
        "smb://nas",
        "smb://nas/",
        "smb://nas:0/Music",
        "smb://alice:secret@nas/Music",
        "smb://alice@nas/Music",
        "smb://nas/Music?password=secret",
        "smb://nas/Music#folder",
        "smb://nas/Music/../Private",
        "smb://nas/Music/%2e%2e/Private",
        "smb://nas/Music/./Album",
        "smb://nas/Music//Album",
        "smb://nas/Music/Album%2fOther",
        "smb://nas/Music/Album%5cOther",
        "smb://nas/Music/%00",
        "smb://nas/Music/%FF",
    ] {
        assert!(Location::parse(value).is_err(), "accepted {value}");
    }
}

#[test]
fn smb_file_paths_stay_under_the_selected_root() {
    let location = Location::parse("smb://nas/Music/Albums").unwrap();
    for path in [
        "../secret",
        "/secret",
        "a/../../secret",
        "a\\secret",
        "a//b",
        "a/./b",
        "a\0b",
    ] {
        assert!(location.path(path).is_err(), "accepted {path}");
    }
}

#[tokio::test]
async fn smb_errors_do_not_expose_connection_details() {
    let auth = call::<()>(async {
        Err(smb2::Error::Auth {
            message: "private credentials".into(),
        })
    })
    .await;
    assert_eq!(auth, Err(SourceError::Auth));
    let io =
        call::<()>(async { Err(smb2::Error::Io(std::io::Error::other("private address"))) }).await;
    assert_eq!(io, Err(SourceError::Connectivity));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires KOPUZ_SMB_TEST_URL, KOPUZ_SMB_TEST_USER, KOPUZ_SMB_TEST_PASSWORD and KOPUZ_SMB_TEST_FILE"]
async fn smb_live_read_and_seek() {
    use std::io::{Read, Seek, SeekFrom};

    let url = std::env::var("KOPUZ_SMB_TEST_URL").expect("test share URL");
    let user = std::env::var("KOPUZ_SMB_TEST_USER").expect("test share username");
    let password = std::env::var("KOPUZ_SMB_TEST_PASSWORD").expect("test share password");
    let path = std::env::var("KOPUZ_SMB_TEST_FILE").expect("nonempty test file relative to URL");
    let mut session = Location::parse(&url)
        .unwrap()
        .connect(&user, &password)
        .await
        .unwrap();
    assert!(!session.list("").await.unwrap().is_empty());
    let mut stream = session.open(&path).await.unwrap();
    drop(session);
    tokio::task::spawn_blocking(move || {
        let size = stream.length();
        assert!(size > 0);
        let mut head = vec![0; size.min(1024) as usize];
        stream.read_exact(&mut head).unwrap();
        stream.seek(SeekFrom::End(-1)).unwrap();
        stream.read_exact(&mut [0]).unwrap();
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        stream.rewind().unwrap();
        let mut repeated = vec![0; head.len()];
        stream.read_exact(&mut repeated).unwrap();
        assert_eq!(repeated, head);
    })
    .await
    .unwrap();
}
