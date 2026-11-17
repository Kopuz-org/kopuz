//! Hepburn readings of Japanese lines, kanji included, through MeCab's IPADIC.
//!
//! Kanji needs a word dictionary to read, and IPADIC is too large to ship in
//! the binary, so the daemon downloads it the first time a Japanese song asks
//! for a reading, checks it against a pinned hash, compiles it once with
//! vibrato and keeps the compiled form beside the library. Until it is ready,
//! [`read`] answers `None` and kana-only lines are read without it.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use sha2::Digest as _;
use vibrato::{Dictionary, SystemDictionaryBuilder, Tokenizer};
use wana_kana::ConvertJapanese as _;

use super::{is_cjk_punctuation, is_han, is_kana};

/// The IPADIC Lindera repackages as UTF-8; MeCab's own release is EUC-JP.
const SOURCE_URL: &str = "https://Lindera.dev/mecab-ipadic-2.7.0-20250920.tar.gz";
const SOURCE_SHA256: &str = "a7ba9f645ffe7094e56ae1c4a81d100df8fbb1e28bbe1792622e9728e162db3d";
/// Named for the source and the compiler, so a newer vibrato's format is a cache miss rather than a bad read.
const COMPILED: &str = "ipadic-2.7.0-20250920.vibrato-0.5.dic";
/// The unpacked source is about 65 MB; anything far past that is not the archive that was pinned.
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A failed fetch is tried again on a later Japanese song, but no sooner than this.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();
static PREPARING: AtomicBool = AtomicBool::new(false);
static LAST_FAILURE: Mutex<Option<Instant>> = Mutex::new(None);

/// Where the dictionary is kept; until this is called no download is ever started.
pub fn set_dictionary_dir(dir: PathBuf) {
    if DIRECTORY.set(dir).is_err() {
        tracing::debug!("japanese dictionary directory was already set");
    }
}

/// The line's reading once the dictionary is ready; `None` before then, which starts it getting ready.
pub(super) fn read(text: &str) -> Option<String> {
    match TOKENIZER.get() {
        Some(tokenizer) => Some(read_with(tokenizer, text)),
        None => {
            prepare();
            None
        }
    }
}

fn read_with(tokenizer: &Tokenizer, text: &str) -> String {
    let mut worker = tokenizer.new_worker();
    worker.reset_sentence(text);
    worker.tokenize();
    let tokens: Vec<(String, String)> = (0..worker.num_tokens())
        .map(|index| {
            let token = worker.token(index);
            (token.surface().to_owned(), token.feature().to_owned())
        })
        .collect();
    join(
        tokens
            .iter()
            .map(|(surface, feature)| word(surface, feature)),
    )
}

/// How one token reads, and whether it belongs to the word before it.
struct Word {
    romaji: String,
    attach: bool,
}

/// One token: IPADIC's feature columns are part of speech, three subcategories, conjugation, base form, reading, pronunciation.
fn word(surface: &str, feature: &str) -> Word {
    let fields: Vec<&str> = feature.split(',').collect();
    let pos = fields.first().copied().unwrap_or("");
    let sub = fields.get(1).copied().unwrap_or("");
    let reading = fields.get(7).copied().filter(|reading| *reading != "*");
    // The particles keep their kana but are said wa, e and o, which is how lyrics are written out.
    let particle = pos == "助詞" && matches!(surface, "は" | "へ" | "を");
    let romaji = if particle {
        match surface {
            "は" => "wa".to_owned(),
            "へ" => "e".to_owned(),
            _ => "o".to_owned(),
        }
    } else if let Some(reading) = reading {
        reading.to_romaji()
    } else if surface.chars().any(is_kana) {
        surface.to_romaji()
    } else if surface.chars().any(|c| is_han(c) || is_cjk_punctuation(c)) {
        any_ascii::any_ascii(surface)
    } else {
        surface.to_owned()
    };
    let punctuation = !surface.chars().any(char::is_alphanumeric);
    // Inflections and auxiliaries read as one word with what they follow: 置いて, しない, 来た.
    let attach = punctuation
        || pos == "助動詞"
        || (pos == "助詞" && sub == "接続助詞" && matches!(surface, "て" | "で" | "ば"))
        || matches!(sub, "非自立" | "接尾");
    Word { romaji, attach }
}

fn join(words: impl Iterator<Item = Word>) -> String {
    let mut out = String::new();
    for word in words {
        let romaji = word.romaji.trim();
        if romaji.is_empty() {
            continue;
        }
        if !out.is_empty() && !word.attach {
            out.push(' ');
        }
        out.push_str(romaji);
    }
    out
}

/// Start loading or fetching the dictionary on its own thread, unless that is already under way or failed lately.
fn prepare() {
    let Some(dir) = DIRECTORY.get() else {
        return;
    };
    let recently_failed = LAST_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some_and(|at| at.elapsed() < RETRY_AFTER);
    if recently_failed || PREPARING.swap(true, Ordering::AcqRel) {
        return;
    }
    let dir = dir.clone();
    let spawned = std::thread::Builder::new()
        .name("kopuz-japanese-dictionary".into())
        .spawn(move || {
            match load_or_build(&dir) {
                Ok(tokenizer) => {
                    if TOKENIZER.set(tokenizer).is_err() {
                        tracing::debug!("japanese dictionary was loaded twice");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not prepare the japanese dictionary; kanji lines stay unread");
                    *LAST_FAILURE
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
                }
            }
            PREPARING.store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start loading the japanese dictionary");
        PREPARING.store(false, Ordering::Release);
    }
}

#[derive(Debug, thiserror::Error)]
enum DictionaryError {
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),
    #[error("downloaded archive does not match the pinned hash")]
    Hash,
    #[error("archive is missing {0}")]
    Missing(&'static str),
    #[error("archive unpacks larger than expected")]
    TooLarge,
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("dictionary: {0}")]
    Vibrato(#[from] vibrato::errors::VibratoError),
}

fn load_or_build(dir: &Path) -> Result<Tokenizer, DictionaryError> {
    let compiled = dir.join(COMPILED);
    if compiled.is_file() {
        match std::fs::File::open(&compiled)
            .map_err(DictionaryError::from)
            .and_then(|file| Ok(Dictionary::read(std::io::BufReader::new(file))?))
        {
            Ok(dictionary) => return tokenizer(dictionary),
            Err(error) => {
                tracing::warn!(%error, "compiled japanese dictionary is unreadable; building it again");
                std::fs::remove_file(&compiled).ok();
            }
        }
    }
    tracing::info!(
        url = SOURCE_URL,
        "downloading the japanese dictionary for lyric readings"
    );
    let archive = utils::http::blocking_builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?
        .get(SOURCE_URL)
        .send()?
        .error_for_status()?
        .bytes()?;
    let digest: String = sha2::Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != SOURCE_SHA256 {
        return Err(DictionaryError::Hash);
    }
    let source = unpack(&archive)?;
    let dictionary = SystemDictionaryBuilder::from_readers(
        source.lexicon.as_slice(),
        source.matrix.as_slice(),
        source.chars.as_slice(),
        source.unknown.as_slice(),
    )?;
    std::fs::create_dir_all(dir)?;
    // Written aside and renamed into place, so a crash mid-write never leaves a half file to read.
    let partial = dir.join(format!("{COMPILED}.partial"));
    dictionary.write(std::io::BufWriter::new(std::fs::File::create(&partial)?))?;
    std::fs::rename(&partial, &compiled)?;
    tracing::info!(path = %compiled.display(), "japanese dictionary ready");
    tokenizer(dictionary)
}

fn tokenizer(dictionary: Dictionary) -> Result<Tokenizer, DictionaryError> {
    Ok(Tokenizer::new(dictionary).ignore_space(true)?)
}

/// The four inputs MeCab's dictionary is built from, out of its source archive.
struct Source {
    lexicon: Vec<u8>,
    matrix: Vec<u8>,
    chars: Vec<u8>,
    unknown: Vec<u8>,
}

fn unpack(archive: &[u8]) -> Result<Source, DictionaryError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let (mut lexicon, mut matrix, mut chars, mut unknown) = (Vec::new(), None, None, None);
    let mut unpacked = 0u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let wanted = name.ends_with(".csv")
            || matches!(name.as_str(), "matrix.def" | "char.def" | "unk.def");
        if !wanted {
            continue;
        }
        unpacked += entry.size();
        if unpacked > MAX_UNPACKED {
            return Err(DictionaryError::TooLarge);
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        entry.read_to_end(&mut bytes)?;
        match name.as_str() {
            "matrix.def" => matrix = Some(bytes),
            "char.def" => chars = Some(bytes),
            "unk.def" => unknown = Some(bytes),
            _ => {
                lexicon.extend_from_slice(&bytes);
                if !lexicon.ends_with(b"\n") {
                    lexicon.push(b'\n');
                }
            }
        }
    }
    if lexicon.is_empty() {
        return Err(DictionaryError::Missing("its word lists"));
    }
    Ok(Source {
        lexicon,
        matrix: matrix.ok_or(DictionaryError::Missing("matrix.def"))?,
        chars: chars.ok_or(DictionaryError::Missing("char.def"))?,
        unknown: unknown.ok_or(DictionaryError::Missing("unk.def"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reads(tokens: &[(&str, &str)]) -> String {
        join(
            tokens
                .iter()
                .map(|(surface, feature)| word(surface, feature)),
        )
    }

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, body.as_bytes())
                .expect("append");
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar.into_inner().expect("tar")).expect("gzip");
        gz.finish().expect("gzip")
    }

    #[test]
    fn words_read_from_ipadic_with_inflections_attached() {
        let line = reads(&[
            (
                "置い",
                "動詞,自立,*,*,五段・カ行イ音便,連用タ接続,置く,オイ,オイ",
            ),
            ("て", "助詞,接続助詞,*,*,*,*,て,テ,テ"),
            ("かれ", "動詞,非自立,*,*,一段,連用形,かれる,カレ,カレ"),
            ("た", "助動詞,*,*,*,特殊・タ,基本形,た,タ,タ"),
            ("と", "助詞,格助詞,引用,*,*,*,と,ト,ト"),
            ("思い", "名詞,一般,*,*,*,*,思い,オモイ,オモイ"),
            ("は", "助詞,係助詞,*,*,*,*,は,ハ,ワ"),
            ("し", "動詞,自立,*,*,サ変・スル,未然形,する,シ,シ"),
            ("ない", "助動詞,*,*,*,特殊・ナイ,基本形,ない,ナイ,ナイ"),
            ("けど", "助詞,接続助詞,*,*,*,*,けど,ケド,ケド"),
        ]);
        assert_eq!(line, "oitekareta to omoi wa shinai kedo");
    }

    #[test]
    fn compounds_and_punctuation() {
        let line = reads(&[
            (
                "東京",
                "名詞,固有名詞,地域,一般,*,*,東京,トウキョウ,トーキョー",
            ),
            ("、", "記号,読点,*,*,*,*,、,、,、"),
            ("baby", "名詞,固有名詞,組織,*,*,*,*"),
        ]);
        assert_eq!(line, "toukyou, baby");
    }

    #[test]
    fn a_token_without_a_reading_falls_back_to_its_kana() {
        assert_eq!(reads(&[("ぴえん", "感動詞,*,*,*,*,*,*")]), "pien");
    }

    #[test]
    fn unpacking_takes_the_word_lists_and_the_three_definitions() {
        let source = unpack(&archive(&[
            ("ipadic/Noun.csv", "東京,1,1,1,名詞"),
            ("ipadic/Verb.csv", "置く,1,1,1,動詞\n"),
            ("ipadic/matrix.def", "1 1\n0 0 0\n"),
            ("ipadic/char.def", "DEFAULT 0 1 0\n"),
            ("ipadic/unk.def", "DEFAULT,0,0,0,記号\n"),
            ("ipadic/README", "ignored"),
        ]))
        .expect("unpack");
        assert_eq!(
            source.lexicon,
            "東京,1,1,1,名詞\n置く,1,1,1,動詞\n".as_bytes()
        );
        assert_eq!(source.matrix, b"1 1\n0 0 0\n");
        assert_eq!(source.unknown, "DEFAULT,0,0,0,記号\n".as_bytes());
    }

    #[test]
    fn a_missing_definition_is_an_error() {
        let result = unpack(&archive(&[("Noun.csv", "東京,1,1,1,名詞\n")]));
        assert!(matches!(result, Err(DictionaryError::Missing(_))));
    }

    // Downloads and compiles the real dictionary, so it only runs when asked: `cargo test -- --ignored`.
    #[test]
    #[ignore = "downloads IPADIC"]
    fn the_real_dictionary_reads_lyrics() {
        let dir = std::env::temp_dir().join(format!("kopuz-ipadic-{}", std::process::id()));
        let built = load_or_build(&dir).expect("dictionary");
        // The second time reads the compiled file instead of downloading again.
        let reloaded = load_or_build(&dir).expect("reload");
        std::fs::remove_dir_all(&dir).ok();
        for (line, reading) in [
            (
                "置いてかれたと思いはしないけど",
                "oite kareta to omoi wa shinai kedo",
            ),
            (
                "こんな顔して歩けない 失礼極まりない",
                "konna kao shite arukenai shitsurei kiwamarinai",
            ),
            (
                "季節を裏切るあたしはいらないから",
                "kisetsu o uragiru atashi wa iranai kara",
            ),
            ("東京", "toukyou"),
        ] {
            assert_eq!(read_with(&built, line), reading);
            assert_eq!(read_with(&reloaded, line), reading);
        }
    }
}
