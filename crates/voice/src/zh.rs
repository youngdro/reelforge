//! Mandarin text → the phoneme symbols Kokoro v1.0 was trained on for Chinese (eSpeak's `cmn`
//! style: IPA syllables with a tone arrow), in pure Rust: numbers read out in Chinese, characters to
//! pinyin with a few phrase readings, the 一 / 不 and third-tone sandhi, and punctuation kept as
//! pauses. Latin letters are spelt out; anything else unspeakable is dropped.
//!
//! The syllable tables follow the MIT / Apache-2.0 `any-tts` crate's compatibility frontend.

use pinyin::ToPinyin;

/// Readings the per-character table gets wrong.
const PHRASES: &[(&str, &[&str])] = &[
    ("重庆", &["chong2", "qing4"]),
    ("银行", &["yin2", "hang2"]),
    ("音乐", &["yin1", "yue4"]),
    ("重要", &["zhong4", "yao4"]),
    ("长大", &["zhang3", "da4"]),
    ("长城", &["chang2", "cheng2"]),
    ("还是", &["hai2", "shi4"]),
    ("还有", &["hai2", "you3"]),
    ("觉得", &["jue2", "de5"]),
    ("睡觉", &["shui4", "jiao4"]),
    ("一个", &["yi2", "ge4"]),
    ("什么", &["shen2", "me5"]),
    ("怎么", &["zen3", "me5"]),
    ("这么", &["zhe4", "me5"]),
    ("那么", &["na4", "me5"]),
    ("为了", &["wei4", "le5"]),
    ("了解", &["liao3", "jie3"]),
    ("行业", &["hang2", "ye4"]),
    ("视频", &["shi4", "pin2"]),
    ("剪辑", &["jian3", "ji2"]),
    ("地方", &["di4", "fang1"]),
    ("东西", &["dong1", "xi5"]),
    ("朋友", &["peng2", "you5"]),
    ("喜欢", &["xi3", "huan5"]),
    ("大家", &["da4", "jia1"]),
    ("得到", &["de2", "dao4"]),
    ("不得不", &["bu4", "de2", "bu4"]),
];

#[derive(Clone, Debug, PartialEq)]
enum Unit {
    Syllable {
        base: String,
        tone: u8,
    },
    /// A pause or a space: `,` `.` `!` `?` `;` `:` or ` `.
    Mark(char),
}

/// `text` as Kokoro phoneme symbols, sentence marks included.
pub fn phonemes(text: &str) -> String {
    let text = read_numbers(text);
    let mut units = units(&text);
    sandhi(&mut units);
    let mut out = String::new();
    for u in units {
        match u {
            Unit::Syllable { base, tone } => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&syllable(&base, tone));
            }
            Unit::Mark(' ') => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            Unit::Mark(m) => {
                let trimmed = out.trim_end().len();
                out.truncate(trimmed);
                if !out.is_empty() && !out.ends_with(['.', ',', '!', '?', ';', ':']) {
                    out.push(m);
                }
                out.push(' ');
            }
        }
    }
    let mut out = out.trim().to_string();
    if !out.is_empty() && !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

fn mark(ch: char) -> Option<char> {
    Some(match ch {
        '.' | '。' => '.',
        ',' | '，' | '、' => ',',
        '!' | '！' => '!',
        '?' | '？' => '?',
        ';' | '；' => ';',
        ':' | '：' => ':',
        '…' | '—' | '–' | '~' | '～' => ',',
        c if c.is_whitespace() => ' ',
        _ => return None,
    })
}

/// Pinyin of a letter's name, for spelling Latin words out.
fn letter(ch: char) -> &'static [&'static str] {
    match ch.to_ascii_lowercase() {
        'a' => &["ei1"],
        'b' => &["bi4"],
        'c' => &["xi1"],
        'd' => &["di4"],
        'e' => &["yi4"],
        'f' => &["ai2", "fu5"],
        'g' => &["ji4"],
        'h' => &["ai2", "qi1"],
        'i' => &["ai4"],
        'j' => &["zhei4"],
        'k' => &["kai4"],
        'l' => &["ai2", "lu5"],
        'm' => &["ai2", "mu5"],
        'n' => &["en1"],
        'o' => &["ou1"],
        'p' => &["pi4"],
        'q' => &["kiu4"],
        'r' => &["a4"],
        's' => &["ai2", "si5"],
        't' => &["ti4"],
        'u' => &["you1"],
        'v' => &["wei4"],
        'w' => &["da2", "bu5", "liu5"],
        'x' => &["ai2", "ke4", "si5"],
        'y' => &["wai4"],
        'z' => &["zei4"],
        _ => &[],
    }
}

fn push_reading(units: &mut Vec<Unit>, reading: &str) {
    let (base, tone) = match reading.chars().last() {
        Some(c) if c.is_ascii_digit() => (reading[..reading.len() - 1].to_string(), c as u8 - b'0'),
        _ => (reading.to_string(), 5),
    };
    units.push(Unit::Syllable { base, tone });
}

fn units(text: &str) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some((phrase, readings)) =
            PHRASES.iter().filter(|(p, _)| rest.starts_with(p)).max_by_key(|(p, _)| p.len())
        {
            for r in *readings {
                push_reading(&mut out, r);
            }
            at += phrase.len();
            continue;
        }
        let ch = rest.chars().next().expect("not at the end");
        at += ch.len_utf8();
        if let Some(p) = ch.to_pinyin() {
            push_reading(&mut out, p.with_tone_num_end());
        } else if ch.is_ascii_alphabetic() {
            for r in letter(ch) {
                push_reading(&mut out, r);
            }
        } else if let Some(m) = mark(ch) {
            out.push(Unit::Mark(m));
        }
    }
    out
}

fn sandhi(units: &mut [Unit]) {
    let tone = |u: &Unit| match u {
        Unit::Syllable { tone, .. } => Some(*tone),
        Unit::Mark(_) => None,
    };
    // 一 and 不 before another syllable
    for i in 0..units.len().saturating_sub(1) {
        let next = tone(&units[i + 1]);
        if let Unit::Syllable { base, tone } = &mut units[i] {
            match (base.as_str(), *tone, next) {
                ("yi", 1, Some(4)) => *tone = 2,
                ("yi", 1, Some(n)) if n != 5 => *tone = 4,
                ("bu", 4, Some(4)) => *tone = 2,
                _ => {}
            }
        }
    }
    // a run of third tones: all but the last become second
    let mut i = 0;
    while i < units.len() {
        let mut j = i;
        while j < units.len() && tone(&units[j]) == Some(3) {
            j += 1;
        }
        if j > i + 1 {
            for u in &mut units[i..j - 1] {
                if let Unit::Syllable { tone, .. } = u {
                    *tone = 2;
                }
            }
        }
        i = j.max(i + 1);
    }
}

fn syllable(base: &str, tone: u8) -> String {
    let base = base.replace('ü', "v");
    let whole = match base.as_str() {
        "zhi" => Some("ʧɻ"),
        "chi" => Some("ʧʰɻ"),
        "shi" => Some("ʃɻ"),
        "ri" => Some("ɻ"),
        "zi" => Some("ʦz"),
        "ci" => Some("ʦʰz"),
        "si" => Some("sz"),
        "yi" => Some("i"),
        "ya" => Some("ja"),
        "yan" => Some("jɛn"),
        "yang" => Some("jɑŋ"),
        "yao" => Some("jau"),
        "ye" => Some("jɛ"),
        "yin" => Some("in"),
        "ying" => Some("iŋ"),
        "yong" => Some("jʊŋ"),
        "you" => Some("jou"),
        "yu" => Some("y"),
        "yue" => Some("yɛ"),
        "yuan" => Some("yɛn"),
        "yun" => Some("yn"),
        "wu" => Some("u"),
        "wa" => Some("wa"),
        "wai" => Some("wai"),
        "wan" => Some("wan"),
        "wang" => Some("wɑŋ"),
        "wei" => Some("wei"),
        "wen" => Some("wən"),
        "weng" => Some("wəŋ"),
        "wo" => Some("wo"),
        _ => None,
    };
    let body = match whole {
        Some(s) => s.to_string(),
        None => {
            let (initial, rest) = [
                "zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c",
                "s",
            ]
            .iter()
            .find_map(|i| base.strip_prefix(i).map(|r| (*i, r)))
            .unwrap_or(("", base.as_str()));
            // after j, q, x a written u is ü
            let rest = if matches!(initial, "j" | "q" | "x") && rest.starts_with('u') {
                format!("v{}", &rest[1..])
            } else {
                rest.to_string()
            };
            format!("{}{}", onset(initial), rhyme(&rest))
        }
    };
    let arrow = match tone {
        1 => "→",
        2 => "↗",
        3 => "↓",
        4 => "↘",
        _ => "",
    };
    format!("{body}{arrow}")
}

fn onset(i: &str) -> &'static str {
    match i {
        "b" => "p",
        "p" => "pʰ",
        "m" => "m",
        "f" => "f",
        "d" => "t",
        "t" => "tʰ",
        "n" => "n",
        "l" => "l",
        "g" => "k",
        "k" => "kʰ",
        "h" => "χ",
        "j" => "ʨ",
        "q" => "ʨʰ",
        "x" => "ɕ",
        "zh" => "ʧ",
        "ch" => "ʧʰ",
        "sh" => "ʃ",
        "r" => "ɻ",
        "z" => "ʦ",
        "c" => "ʦʰ",
        "s" => "s",
        _ => "",
    }
}

fn rhyme(f: &str) -> String {
    match f {
        "a" => "a",
        "ai" => "ai",
        "an" => "an",
        "ang" => "ɑŋ",
        "ao" => "au",
        "e" => "ɤ",
        "ei" => "ei",
        "en" => "ən",
        "eng" => "əŋ",
        "er" => "ɚ",
        "i" => "i",
        "ia" => "ja",
        "ian" => "jɛn",
        "iang" => "jɑŋ",
        "iao" => "jau",
        "ie" => "jɛ",
        "in" => "in",
        "ing" => "iŋ",
        "iong" => "jʊŋ",
        "iu" => "jou",
        "o" => "o",
        "ong" => "ʊŋ",
        "ou" => "ou",
        "u" => "u",
        "ua" => "wa",
        "uai" => "wai",
        "uan" => "wan",
        "uang" => "wɑŋ",
        "ui" => "wei",
        "un" => "wən",
        "uo" => "wo",
        "v" => "y",
        "ve" => "yɛ",
        "van" => "yɛn",
        "vn" => "yn",
        other => other,
    }
    .to_string()
}

const DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// A whole number read in Chinese: 2025 → 两千零二十五. Up to 万亿.
fn number(n: u64) -> String {
    if n == 0 {
        return "零".into();
    }
    fn below_10000(n: u64, out: &mut String, leading: bool) {
        let units = [(1000, '千'), (100, '百'), (10, '十')];
        // a 零 for a gap inside the group; the one before it is the caller's
        let mut zero = false;
        let mut seen = false;
        for (u, name) in units {
            let d = n / u % 10;
            if d == 0 {
                zero = seen;
                continue;
            }
            if zero {
                out.push('零');
                zero = false;
            }
            // 十二, not 一十二, at the head of a number
            if !(u == 10 && d == 1 && leading && !seen) {
                out.push(DIGITS[d as usize]);
            }
            out.push(name);
            seen = true;
        }
        let d = n % 10;
        if d != 0 {
            if zero {
                out.push('零');
            }
            out.push(DIGITS[d as usize]);
        }
    }
    let mut out = String::new();
    let groups = [(1_0000_0000_0000u64, "万亿"), (1_0000_0000, "亿"), (1_0000, "万"), (1, "")];
    let mut started = false;
    let mut gap = false;
    for (scale, name) in groups {
        let g = n / scale % 10000;
        if g == 0 {
            gap = started;
            continue;
        }
        if started && (gap || g < 1000) {
            out.push('零');
        }
        below_10000(g, &mut out, !started);
        out.push_str(name);
        started = true;
        gap = false;
    }
    // 两 before a measure-like unit reads better: 2000 → 两千, 2万 → 两万
    if out.starts_with("二千") || out.starts_with("二万") || out.starts_with("二亿") || out.starts_with("二百")
    {
        out.replace_range(0..'二'.len_utf8(), "两");
    }
    out
}

/// Digits written out: whole numbers by place, decimals digit by digit, `%` as 百分之.
fn read_numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_ascii_digit()
                || chars[i] == ',' && chars.get(i + 1).is_some_and(char::is_ascii_digit) && i > start)
        {
            i += 1;
        }
        let whole: String = chars[start..i].iter().filter(|c| c.is_ascii_digit()).collect();
        let mut decimals = String::new();
        if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                decimals.push(chars[i]);
                i += 1;
            }
        }
        let percent = i < chars.len() && (chars[i] == '%' || chars[i] == '％');
        if percent {
            out.push_str("百分之");
            i += 1;
        }
        // long digit strings (phone numbers, years written as codes) are read digit by digit
        match whole.parse::<u64>() {
            Ok(n) if whole.len() <= 13 && !(whole.len() > 1 && whole.starts_with('0')) => out.push_str(&number(n)),
            _ => out.extend(whole.chars().map(|c| DIGITS[(c as u8 - b'0') as usize])),
        }
        if !decimals.is_empty() {
            out.push('点');
            out.extend(decimals.chars().map(|c| DIGITS[(c as u8 - b'0') as usize]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_in_chinese() {
        assert_eq!(number(0), "零");
        assert_eq!(number(12), "十二");
        assert_eq!(number(105), "一百零五");
        assert_eq!(number(2025), "两千零二十五");
        assert_eq!(number(10_010), "一万零一十");
        assert_eq!(number(30_000), "三万");
        assert_eq!(number(1_0005_0000), "一亿零五万");
        assert_eq!(read_numbers("3.5% 和 007"), "百分之三点五 和 零零七");
        assert_eq!(read_numbers("第12集"), "第十二集");
    }

    #[test]
    fn mandarin_becomes_tone_marked_syllables() {
        assert_eq!(phonemes("你好"), "ni↗ χau↓.");
        // 一 before a fourth tone rises; 不 before a fourth tone rises
        assert_eq!(phonemes("一个"), "i↗ kɤ↘.");
        assert_eq!(phonemes("不对"), "pu↗ twei↘.");
        // the phrase table wins over the per-character reading
        assert!(phonemes("重庆").starts_with("ʧʰʊŋ↗"));
        // punctuation is a pause, and a sentence ends with one
        assert_eq!(phonemes("你好，世界！"), "ni↗ χau↓, ʃɻ↘ ʨjɛ↘!");
        // j/q/x + u is ü
        assert_eq!(phonemes("去"), "ʨʰy↘.");
        // Latin letters are spelt
        assert!(phonemes("AI").contains("ei→"));
        assert_eq!(phonemes("🙂"), "");
    }
}
