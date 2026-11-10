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
    ("成长", &["cheng2", "zhang3"]),
    ("增长", &["zeng1", "zhang3"]),
    ("生长", &["sheng1", "zhang3"]),
    ("长得", &["zhang3", "de5"]),
    ("长辈", &["zhang3", "bei4"]),
    ("家长", &["jia1", "zhang3"]),
    ("校长", &["xiao4", "zhang3"]),
    ("队长", &["dui4", "zhang3"]),
    ("班长", &["ban1", "zhang3"]),
    ("部长", &["bu4", "zhang3"]),
    ("市长", &["shi4", "zhang3"]),
    ("组长", &["zu3", "zhang3"]),
    ("店长", &["dian4", "zhang3"]),
    ("董事长", &["dong3", "shi4", "zhang3"]),
    ("院长", &["yuan4", "zhang3"]),
    ("船长", &["chuan2", "zhang3"]),
    ("村长", &["cun1", "zhang3"]),
    ("长相", &["zhang3", "xiang4"]),
    ("获得", &["huo4", "de2"]),
    ("取得", &["qu3", "de2"]),
    ("得分", &["de2", "fen1"]),
    ("得意", &["de2", "yi4"]),
    ("难得", &["nan2", "de2"]),
    ("心得", &["xin1", "de2"]),
    ("得出", &["de2", "chu1"]),
    ("只得", &["zhi3", "de2"]),
    ("得奖", &["de2", "jiang3"]),
    ("得罪", &["de2", "zui4"]),
    ("得失", &["de2", "shi1"]),
    ("赢得", &["ying2", "de2"]),
    ("懂得", &["dong3", "de5"]),
    ("便宜", &["pian2", "yi5"]),
    ("调整", &["tiao2", "zheng3"]),
    ("调节", &["tiao2", "jie2"]),
    ("调色", &["tiao2", "se4"]),
    ("空调", &["kong1", "tiao2"]),
    ("调皮", &["tiao2", "pi2"]),
    ("协调", &["xie2", "tiao2"]),
    ("调料", &["tiao2", "liao4"]),
    ("调和", &["tiao2", "he2"]),
    ("处理", &["chu3", "li3"]),
    ("处于", &["chu3", "yu2"]),
    ("相处", &["xiang1", "chu3"]),
    ("处境", &["chu3", "jing4"]),
    ("干净", &["gan1", "jing4"]),
    ("干燥", &["gan1", "zao4"]),
    ("饼干", &["bing3", "gan1"]),
    ("干杯", &["gan1", "bei1"]),
    ("干脆", &["gan1", "cui4"]),
    ("成为", &["cheng2", "wei2"]),
    ("作为", &["zuo4", "wei2"]),
    ("认为", &["ren4", "wei2"]),
    ("以为", &["yi3", "wei2"]),
    ("称为", &["cheng1", "wei2"]),
    ("行为", &["xing2", "wei2"]),
    ("为止", &["wei2", "zhi3"]),
    ("因为", &["yin1", "wei4"]),
    ("种植", &["zhong4", "zhi2"]),
    ("种地", &["zhong4", "di4"]),
    ("种树", &["zhong4", "shu4"]),
    ("种菜", &["zhong4", "cai4"]),
    ("教我", &["jiao1", "wo3"]),
    ("教你", &["jiao1", "ni3"]),
    ("教大家", &["jiao1", "da4", "jia1"]),
    ("教他", &["jiao1", "ta1"]),
    ("应用", &["ying4", "yong4"]),
    ("回应", &["hui2", "ying4"]),
    ("答应", &["da1", "ying4"]),
    ("反应", &["fan3", "ying4"]),
    ("适应", &["shi4", "ying4"]),
    ("效应", &["xiao4", "ying4"]),
    ("供应", &["gong1", "ying4"]),
    ("爱好", &["ai4", "hao4"]),
    ("好奇", &["hao4", "qi2"]),
    ("好学", &["hao4", "xue2"]),
    ("少年", &["shao4", "nian2"]),
    ("少女", &["shao4", "nv3"]),
    ("有空", &["you3", "kong4"]),
    ("空闲", &["kong4", "xian2"]),
    ("空白", &["kong4", "bai2"]),
    ("商量", &["shang1", "liang5"]),
    ("测量", &["ce4", "liang2"]),
    ("背包", &["bei1", "bao1"]),
    ("背着", &["bei1", "zhe5"]),
    ("西藏", &["xi1", "zang4"]),
    ("宝藏", &["bao3", "zang4"]),
    ("薄荷", &["bo4", "he5"]),
    ("假期", &["jia4", "qi1"]),
    ("放假", &["fang4", "jia4"]),
    ("暑假", &["shu3", "jia4"]),
    ("寒假", &["han2", "jia4"]),
    ("请假", &["qing3", "jia4"]),
    ("假日", &["jia4", "ri4"]),
    ("度假", &["du4", "jia4"]),
    ("结实", &["jie1", "shi5"]),
    ("积累", &["ji1", "lei3"]),
    ("累计", &["lei3", "ji4"]),
    ("头发", &["tou2", "fa5"]),
    ("理发", &["li3", "fa4"]),
    ("暖和", &["nuan3", "huo5"]),
    ("出差", &["chu1", "chai1"]),
    ("相机", &["xiang4", "ji1"]),
    ("照相", &["zhao4", "xiang4"]),
    ("真相", &["zhen1", "xiang4"]),
    ("适当", &["shi4", "dang4"]),
    ("上当", &["shang4", "dang4"]),
    ("当作", &["dang4", "zuo4"]),
    ("当成", &["dang4", "cheng2"]),
    ("部分", &["bu4", "fen5"]),
    ("更新", &["geng1", "xin1"]),
    ("更改", &["geng1", "gai3"]),
    ("变更", &["bian4", "geng1"]),
    ("灾难", &["zai1", "nan4"]),
    ("中奖", &["zhong4", "jiang3"]),
    ("打中", &["da3", "zhong4"]),
    ("重新", &["chong2", "xin1"]),
    ("重复", &["chong2", "fu4"]),
    ("重来", &["chong2", "lai2"]),
    ("重做", &["chong2", "zuo4"]),
    ("行情", &["hang2", "qing2"]),
    ("乐器", &["yue4", "qi4"]),
    ("乐队", &["yue4", "dui4"]),
    ("着急", &["zhao2", "ji2"]),
    ("睡着", &["shui4", "zhao2"]),
    ("着火", &["zhao2", "huo3"]),
    ("着手", &["zhuo2", "shou3"]),
    ("着陆", &["zhuo2", "lu4"]),
    ("了不起", &["liao3", "bu4", "qi3"]),
    ("受不了", &["shou4", "bu4", "liao3"]),
    ("不得了", &["bu4", "de2", "liao3"]),
    ("首都", &["shou3", "du1"]),
    ("都市", &["du1", "shi4"]),
    ("成都", &["cheng2", "du1"]),
    ("一只", &["yi4", "zhi1"]),
    ("两只", &["liang3", "zhi1"]),
    ("慢慢地", &["man4", "man4", "de5"]),
    ("认真地", &["ren4", "zhen1", "de5"]),
    ("轻轻地", &["qing1", "qing1", "de5"]),
    ("默默地", &["mo4", "mo4", "de5"]),
    ("渐渐地", &["jian4", "jian4", "de5"]),
    ("悄悄地", &["qiao1", "qiao1", "de5"]),
    ("不断地", &["bu4", "duan4", "de5"]),
    ("静静地", &["jing4", "jing4", "de5"]),
    ("什么样", &["shen2", "me5", "yang4"]),
    ("为什么", &["wei4", "shen2", "me5"]),
];

/// Readings that differ from the pinyin table's first one: the commoner one in speech.
const SINGLE: &[(char, &str)] = &[('得', "de5"), ('长', "chang2")];

/// `text` as the phoneme symbols Kokoro was trained on (misaki's Mandarin: pinyin readings as IPA,
/// the tone after the vowel, syllables of a word together and words apart), sentence marks
/// included. Tones are the dictionary's, without sandhi, as in the training data.
pub fn phonemes(text: &str) -> String {
    let text = read_numbers(text);
    let starts: std::collections::HashSet<usize> = JIEBA.cut(&text, true).iter().map(|t| t.byte_start).collect();
    let mut out = String::new();
    let add = |out: &mut String, at: usize, reading: &str| {
        // a new word is set apart; a syllable inside one follows on
        if starts.contains(&at) && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(&syllable(reading));
    };
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some((phrase, readings)) =
            PHRASES.iter().filter(|(p, _)| rest.starts_with(p)).max_by_key(|(p, _)| p.len())
        {
            for ((i, _), r) in phrase.char_indices().zip(readings.iter()) {
                add(&mut out, at + i, r);
            }
            at += phrase.len();
            continue;
        }
        let ch = rest.chars().next().expect("not at the end");
        if let Some((_, r)) = SINGLE.iter().find(|(c, _)| *c == ch) {
            add(&mut out, at, r);
        } else if let Some(p) = ch.to_pinyin() {
            add(&mut out, at, p.with_tone_num_end());
        } else if ch.is_ascii_alphabetic() {
            for r in letter(ch) {
                add(&mut out, at, r);
            }
        } else if let Some(m) = mark(ch) {
            let trimmed = out.trim_end().len();
            out.truncate(trimmed);
            if m != ' ' && !out.is_empty() && !out.ends_with(['.', ',', '!', '?', ';', ':']) {
                out.push(m);
            }
            out.push(' ');
        }
        at += ch.len_utf8();
    }
    let mut out = out.trim().to_string();
    if !out.is_empty() && !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// Words, for where the spaces go.
static JIEBA: std::sync::LazyLock<jieba_rs::Jieba> = std::sync::LazyLock::new(jieba_rs::Jieba::new);

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

/// One pinyin syllable with its tone number (`hao3`; none or 5 is neutral) as misaki writes it.
fn syllable(reading: &str) -> String {
    let (base, tone) = match reading.chars().last() {
        Some(c) if c.is_ascii_digit() => (&reading[..reading.len() - 1], c as u8 - b'0'),
        _ => (reading, 5),
    };
    let arrow = match tone {
        1 => "→",
        2 => "↗",
        3 => "↓",
        4 => "↘",
        _ => "",
    };
    let base = base.replace('ü', "v");
    // syllables that are a vowel or a consonant alone
    let alone = match base.as_str() {
        "er" => Some("ɚ0"),
        "o" => Some("ɔ0"),
        "ê" => Some("ɛ0"),
        "io" => Some("jɔ0"),
        "m" => Some("m0"),
        "n" => Some("n0"),
        "ng" => Some("ŋ0"),
        "hm" => Some("hm0"),
        "hng" => Some("hŋ0"),
        _ => None,
    };
    if let Some(a) = alone {
        return a.replace('0', arrow);
    }
    let (initial, rest) =
        ["zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s"]
            .iter()
            .find_map(|i| base.strip_prefix(i).map(|r| (*i, r)))
            .unwrap_or(("", base.as_str()));
    // y and w spell a medial: yan is ian, wei is uei, yu is ü
    let rest = match initial {
        "" => match rest {
            "yi" | "yin" | "ying" => rest[1..].to_string(),
            "yu" | "yue" | "yuan" | "yun" => format!("v{}", &rest[2..]),
            "wu" => "u".into(),
            r if r.starts_with('y') => format!("i{}", &r[1..]),
            r if r.starts_with('w') => format!("u{}", &r[1..]),
            r => r.to_string(),
        },
        // after j, q, x a written u is ü
        "j" | "q" | "x" if rest.starts_with('u') => format!("v{}", &rest[1..]),
        _ => rest.to_string(),
    };
    let fin = match (initial, rest.as_str()) {
        ("zh" | "ch" | "sh" | "r" | "z" | "c" | "s", "i") => "ɨ0",
        (_, f) => rhyme(f),
    };
    format!("{}{}", onset(initial), fin.replace('0', arrow))
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
        "h" => "x",
        "j" => "ʨ",
        "q" => "ʨʰ",
        "x" => "ɕ",
        "zh" => "ꭧ",
        "ch" => "ꭧʰ",
        "sh" => "ʂ",
        "r" => "ɻ",
        "z" => "ʦ",
        "c" => "ʦʰ",
        "s" => "s",
        _ => "",
    }
}

/// A final with `0` where the tone goes.
fn rhyme(f: &str) -> &'static str {
    match f {
        "a" => "a0",
        "ai" => "ai0",
        "an" => "a0n",
        "ang" => "a0ŋ",
        "ao" => "au0",
        "e" => "ɤ0",
        "ei" => "ei0",
        "en" => "ə0n",
        "eng" => "ə0ŋ",
        "i" => "i0",
        "ia" => "ja0",
        "ian" => "jɛ0n",
        "iang" => "ja0ŋ",
        "iao" => "jau0",
        "ie" => "je0",
        "in" => "i0n",
        "iu" | "iou" => "jou0",
        "ing" => "i0ŋ",
        "iong" => "jʊ0ŋ",
        "ong" => "ʊ0ŋ",
        "ou" => "ou0",
        "u" => "u0",
        "ui" | "uei" => "wei0",
        "ua" => "wa0",
        "uai" => "wai0",
        "uan" => "wa0n",
        "un" | "uen" => "wə0n",
        "uang" => "wa0ŋ",
        "ueng" => "wə0ŋ",
        "uo" | "o" => "wo0",
        "v" => "y0",
        "ve" => "ɥe0",
        "van" => "ɥɛ0n",
        "vn" => "y0n",
        _ => "",
    }
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
        // a word's syllables together, the tone after the vowel, h as x
        assert_eq!(phonemes("你好"), "ni↓xau↓.");
        // the dictionary's tones, as Kokoro was trained on
        assert_eq!(phonemes("不对"), "pu↘ twei↘.");
        // the phrase table wins over the per-character reading
        assert_eq!(phonemes("重庆"), "ꭧʰʊ↗ŋʨʰi↘ŋ.");
        // punctuation is a pause, and a sentence ends with one; words are set apart
        assert_eq!(phonemes("你好，世界！"), "ni↓xau↓, ʂɨ↘ʨje↘!");
        assert_eq!(phonemes("我们剪视频"), "wo↓mən ʨjɛ↓n ʂɨ↘pʰi↗n.");
        // j/q/x + u is ü; zi, ci, si have ɨ
        assert_eq!(phonemes("去"), "ʨʰy↘.");
        assert_eq!(phonemes("自己"), "ʦɨ↘ʨi↓.");
        assert_eq!(phonemes("音乐"), "i→nɥe↘.");
        // Latin letters are spelt
        assert!(phonemes("AI").contains("ei→"));
        assert_eq!(phonemes("🙂"), "");
        // common polyphones read as they are said
        assert_eq!(phonemes("便宜"), "pʰjɛ↗ni.");
        assert!(phonemes("跑得快").contains("tɤ kʰwai↘") || phonemes("跑得快").contains("tɤkʰwai↘"));
        assert_eq!(phonemes("得到"), "tɤ↗tau↘.");
        assert!(phonemes("很长").ends_with("ꭧʰa↗ŋ."));
        assert!(phonemes("长大").starts_with("ꭧa↓ŋ"));
        assert!(phonemes("调整").starts_with("tʰjau↗"));
    }
}
