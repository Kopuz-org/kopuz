//! A Latin reading of lyric lines written in another script.
//!
//! Hangul follows Revised Romanization with its sound changes between
//! syllables, kana becomes Hepburn romaji, Chinese characters pinyin with tone
//! marks, and every other script goes through a general transliteration table.
//! A song with kana anywhere is Japanese, so its kanji is read through MeCab's
//! dictionary (see [`japanese`]) rather than as Chinese; until that dictionary
//! is ready, its kanji lines are left unread.

mod japanese;

pub use japanese::set_dictionary_dir;

use pinyin::ToPinyin as _;
use unicode_script::{Script, UnicodeScript as _};
use wana_kana::ConvertJapanese as _;

/// A reading for each line, `None` where the line is already Latin or no
/// trustworthy reading exists.
pub fn romanize_lines<S: AsRef<str>>(lines: &[S]) -> Vec<Option<String>> {
    let japanese = lines.iter().any(|line| line.as_ref().chars().any(is_kana));
    lines
        .iter()
        .map(|line| romanize_line(line.as_ref(), japanese))
        .collect()
}

/// One line's reading; `japanese` says the song is Japanese, so kanji is not read as Chinese.
fn romanize_line(text: &str, japanese: bool) -> Option<String> {
    if !text.chars().any(needs_reading) {
        return None;
    }
    if japanese {
        if let Some(out) = japanese::read(text) {
            return (!out.is_empty() && out != text.trim()).then_some(out);
        }
        // A Chinese reading of Japanese kanji is worse than none.
        if text.chars().any(is_han) {
            return None;
        }
    }

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 2);
    // Pinyin is one syllable per character, so a Latin word right after one needs a space of its own.
    let mut after_syllable = false;
    let mut carried_initial: Option<&'static str> = None;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if let Some(syllable) = Syllable::of(c) {
            let next = chars.get(index + 1).copied().and_then(Syllable::of);
            let (coda, next_initial) = match next {
                Some(next) => syllable.link(next),
                None => (FINALS[syllable.coda], None),
            };
            out.push_str(carried_initial.take().unwrap_or(INITIALS[syllable.onset]));
            out.push_str(MEDIALS[syllable.nucleus]);
            out.push_str(coda);
            carried_initial = next_initial;
            after_syllable = false;
            index += 1;
        } else if is_kana(c) {
            let end = chars[index..]
                .iter()
                .position(|&c| !is_kana(c))
                .map_or(chars.len(), |offset| index + offset);
            let run: String = chars[index..end].iter().collect();
            out.push_str(&run.as_str().to_romaji());
            after_syllable = false;
            index = end;
        } else if is_han(c) {
            match c.to_pinyin() {
                Some(reading) => {
                    // Chinese punctuation carries its own spacing, which the ASCII stand-in does not.
                    if out
                        .chars()
                        .last()
                        .is_some_and(|last| last.is_alphanumeric() || ",.!?;:".contains(last))
                    {
                        out.push(' ');
                    }
                    out.push_str(reading.with_tone());
                    after_syllable = true;
                }
                None => out.push_str(any_ascii::any_ascii_char(c)),
            }
            index += 1;
        } else {
            if after_syllable && c.is_alphanumeric() {
                out.push(' ');
            }
            if needs_reading(c) || is_cjk_punctuation(c) {
                out.push_str(any_ascii::any_ascii_char(c));
            } else {
                out.push(c);
            }
            after_syllable = false;
            index += 1;
        }
    }

    let out = out.trim();
    (!out.is_empty() && out != text.trim()).then(|| out.to_owned())
}

/// A letter in a script other than Latin.
fn needs_reading(c: char) -> bool {
    !matches!(
        c.script(),
        Script::Latin | Script::Common | Script::Inherited | Script::Unknown
    )
}

fn is_kana(c: char) -> bool {
    matches!(c.script(), Script::Hiragana | Script::Katakana)
        // The long-vowel mark is script Common, but only ever lengthens kana.
        || c == 'ー'
}

fn is_han(c: char) -> bool {
    c.script() == Script::Han
}

/// Ideographic punctuation and fullwidth forms, which read better as their ASCII counterparts.
fn is_cjk_punctuation(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303F}' | '\u{FF01}'..='\u{FF5E}')
}

const INITIALS: [&str; 19] = [
    "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p",
    "h",
];
const MEDIALS: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we",
    "wi", "yu", "eu", "ui", "i",
];
const FINALS: [&str; 28] = [
    "", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p",
    "t", "t", "ng", "t", "t", "k", "t", "p", "t",
];

// Initial consonant indices.
const I_G: usize = 0;
const I_N: usize = 2;
const I_D: usize = 3;
const I_R: usize = 5;
const I_M: usize = 6;
const I_S: usize = 9;
const I_SILENT: usize = 11;
const I_J: usize = 12;
const I_H: usize = 18;
const V_I: usize = 20;

/// A precomposed Hangul syllable, split into its jamo indices.
#[derive(Clone, Copy)]
struct Syllable {
    onset: usize,
    nucleus: usize,
    coda: usize,
}

impl Syllable {
    fn of(c: char) -> Option<Self> {
        let offset = (c as u32).checked_sub(0xAC00).filter(|&o| o < 11172)? as usize;
        Some(Self {
            onset: offset / (21 * 28),
            nucleus: offset % (21 * 28) / 28,
            coda: offset % 28,
        })
    }

    /// How this syllable's final and the next syllable's initial are pronounced
    /// together: the final as written here, and the next initial when it changes.
    fn link(self, next: Syllable) -> (&'static str, Option<&'static str>) {
        let coda = self.coda;
        let plain = (FINALS[coda], None);
        if coda == 0 {
            return plain;
        }
        let k = matches!(coda, 1 | 2 | 3 | 9 | 24);
        let t = matches!(coda, 7 | 19 | 20 | 22 | 23 | 25 | 27);
        let p = matches!(coda, 14 | 17 | 18 | 26);
        let palatal = next.nucleus == V_I;
        match next.onset {
            // A final before a silent initial is carried over and sounded there.
            I_SILENT => match coda {
                1 => ("", Some("g")),
                2 => ("", Some("kk")),
                3 => ("k", Some("s")),
                4 | 6 => ("", Some("n")),
                5 => ("n", Some("j")),
                7 if palatal => ("", Some("j")),
                7 => ("", Some("d")),
                8 | 15 => ("", Some("r")),
                9 => ("l", Some("g")),
                10 => ("l", Some("m")),
                11 => ("l", Some("b")),
                12 => ("l", Some("s")),
                13 if palatal => ("l", Some("ch")),
                13 => ("l", Some("t")),
                14 => ("l", Some("p")),
                16 => ("", Some("m")),
                17 => ("", Some("b")),
                18 => ("p", Some("s")),
                19 => ("", Some("s")),
                20 => ("", Some("ss")),
                22 => ("", Some("j")),
                23 => ("", Some("ch")),
                24 => ("", Some("k")),
                25 if palatal => ("", Some("ch")),
                25 => ("", Some("t")),
                26 => ("", Some("p")),
                27 => ("", Some("")),
                _ => plain,
            },
            // A stop before ㅎ is aspirated into it.
            I_H => match coda {
                9 => ("l", Some("k")),
                14 => ("l", Some("p")),
                5 => ("n", Some("ch")),
                22 | 23 => ("", Some("ch")),
                _ if k => ("", Some("k")),
                _ if p => ("", Some("p")),
                27 => ("", Some("h")),
                _ if t => ("", Some("t")),
                _ => plain,
            },
            // ㅎ aspirates the stop after it.
            I_G | I_D | I_J if matches!(coda, 6 | 15 | 27) => {
                let kept = match coda {
                    6 => "n",
                    15 => "l",
                    _ => "",
                };
                let aspirated = match next.onset {
                    I_G => "k",
                    I_D => "t",
                    _ => "ch",
                };
                (kept, Some(aspirated))
            }
            I_S if coda == 27 => ("", Some("ss")),
            // Nasal assimilation, and ㄹ turning a ㄴ next to it into ㄹ.
            I_N | I_M => match coda {
                8 | 11 | 12 | 13 | 15 if next.onset == I_N => ("l", Some("l")),
                10 => ("m", None),
                6 | 27 => ("n", None),
                _ if k => ("ng", None),
                _ if t => ("n", None),
                _ if p => ("m", None),
                _ => plain,
            },
            I_R => match coda {
                4 | 8 => ("l", Some("l")),
                16 | 21 => (FINALS[coda], Some("n")),
                _ if k => ("ng", Some("n")),
                _ if p => ("m", Some("n")),
                _ if t => ("n", Some("n")),
                _ => plain,
            },
            _ => plain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Option<String> {
        romanize_lines(&[text]).remove(0)
    }

    #[test]
    fn latin_lines_have_no_reading() {
        assert_eq!(one("Hello, world"), None);
        assert_eq!(one("Café déjà vu"), None);
        assert_eq!(one("1, 2, 3!"), None);
        assert_eq!(one(""), None);
    }

    #[test]
    fn hangul_reads_as_revised_romanization() {
        assert_eq!(one("안녕하세요").as_deref(), Some("annyeonghaseyo"));
        assert_eq!(one("사랑해 baby").as_deref(), Some("saranghae baby"));
        assert_eq!(one("한국어").as_deref(), Some("hangugeo"));
    }

    #[test]
    fn hangul_follows_the_sound_changes_between_syllables() {
        assert_eq!(one("신라").as_deref(), Some("silla"));
        assert_eq!(one("종로").as_deref(), Some("jongno"));
        assert_eq!(one("좋아").as_deref(), Some("joa"));
        assert_eq!(one("같이").as_deref(), Some("gachi"));
        assert_eq!(one("먹는").as_deref(), Some("meongneun"));
        assert_eq!(one("좋고").as_deref(), Some("joko"));
        assert_eq!(one("몰라").as_deref(), Some("molla"));
        assert_eq!(one("밖에").as_deref(), Some("bakke"));
    }

    #[test]
    fn kana_reads_as_hepburn() {
        assert_eq!(one("ありがとう").as_deref(), Some("arigatou"));
        assert_eq!(one("カタカナ").as_deref(), Some("katakana"));
        assert_eq!(one("ラーメン").as_deref(), Some("raamen"));
        assert_eq!(one("ちょっと").as_deref(), Some("chotto"));
    }

    #[test]
    fn japanese_kanji_waits_for_the_dictionary() {
        // No dictionary directory is set in tests, so nothing is fetched and kanji stays unread.
        let song = romanize_lines(&["さよなら", "東京"]);
        assert_eq!(song, vec![Some("sayonara".to_owned()), None]);
    }

    #[test]
    fn chinese_reads_as_pinyin_with_tones() {
        assert_eq!(one("我爱你").as_deref(), Some("wǒ ài nǐ"));
        assert_eq!(one("你好 world").as_deref(), Some("nǐ hǎo world"));
        assert_eq!(one("你好，世界").as_deref(), Some("nǐ hǎo, shì jiè"));
        assert_eq!(one("我爱你baby").as_deref(), Some("wǒ ài nǐ baby"));
    }

    #[test]
    fn other_scripts_are_transliterated() {
        assert_eq!(one("Привет мир").as_deref(), Some("Privet mir"));
        assert_eq!(one("Καλημέρα").as_deref(), Some("Kalimera"));
        assert_eq!(one("Ελλάδα").as_deref(), Some("Ellada"));
    }
}
