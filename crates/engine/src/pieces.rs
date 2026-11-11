//! The ready-made pieces `promo.add` makes: what there is, and the clips each is built from.
//!
//! A piece is drawn for a canvas whose shorter side is 1080 pixels, around its own centre: a list
//! of [`Part`]s, bottom first, each a shape, a title or a picture with where it sits, how it comes
//! in and when. `promo.add` gives each a clip, scaled and placed for the project's canvas.

use reelforge_model::{
    AnimKind, ClipContent, Counter, Ease, Fill, Inbetween, Karaoke, KaraokeMode, LoopKind, Prop, Shape, ShapeKind,
    ShapeShadow, Stroke, TitleAlign, TitleStyle, TrackKind,
};
use reelforge_time::Tick;

/// What kind of thing a piece is, for the lists it is found in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Category {
    /// Prices and offers.
    Price,
    /// What the product is and does.
    Product,
    /// Headings and numbers.
    Titles,
    /// What to do next, and who is asking.
    Action,
    /// Between shots.
    Rhythm,
}

impl Category {
    /// Every category, in menu order.
    pub const ALL: [Category; 5] =
        [Category::Price, Category::Product, Category::Titles, Category::Action, Category::Rhythm];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            Category::Price => "price",
            Category::Product => "product",
            Category::Titles => "titles",
            Category::Action => "action",
            Category::Rhythm => "rhythm",
        }
    }
}

/// A ready-made piece.
pub struct Promo {
    /// Its id.
    pub id: &'static str,
    /// The list it is found in.
    pub category: Category,
    /// What it is, in English.
    pub en: &'static str,
    /// In Chinese.
    pub zh: &'static str,
    /// The words it shows unless others are given; `{n}` is its number.
    pub text: &'static str,
    /// Its further lines unless others are given: rows of a list, or the smaller words beside
    /// the main ones (`about` says which).
    pub lines: &'static [&'static str],
    /// The number it counts to, if it counts.
    pub value: Option<f32>,
    /// Its further numbers, for a piece that compares.
    pub values: &'static [f32],
    /// Where its counting starts.
    pub from: f32,
    /// Where it is put unless told: offsets from the frame's centre, as shares of its size.
    pub place: [f32; 2],
    /// How long it shows unless told, in thousandths of a second.
    pub millis: i64,
    /// What its parameters mean, in English.
    pub about: &'static str,
    /// Whether it shows a picture of the project's media it is given (`media`).
    pub pictured: bool,
}

const fn piece(
    id: &'static str,
    category: Category,
    en: &'static str,
    zh: &'static str,
    text: &'static str,
    about: &'static str,
) -> Promo {
    Promo {
        id,
        category,
        en,
        zh,
        text,
        lines: &[],
        value: None,
        values: &[],
        from: 0.0,
        place: [0.0, 0.0],
        millis: 3000,
        about,
        pictured: false,
    }
}

use Category::{Action, Price, Product, Rhythm, Titles};

/// The pieces `promo.add` makes.
pub const PROMOS: &[Promo] = &[
    Promo {
        value: Some(99.0),
        ..piece("priceTag", Price, "Price tag", "价格标签", "¥{n}", "`value` is the price it counts up to.")
    },
    piece("discountBadge", Price, "Discount badge", "折扣角标", "-50%", "`text` is the discount."),
    piece("banner", Price, "Sale banner", "促销横幅", "限时特惠", "`text` is the headline."),
    Promo {
        value: Some(10.0),
        ..piece(
            "countdown",
            Price,
            "Countdown",
            "倒计时",
            "{n}",
            "`value` is the seconds it counts down from, a second a second.",
        )
    },
    Promo {
        value: Some(10000.0),
        ..piece("salesCounter", Price, "Sales counter", "销量滚动", "已售 {n}+", "`value` is the count it runs up to.")
    },
    piece("strikePrice", Price, "Struck-out price", "划线原价", "¥199", "`text` is the old price."),
    piece("newArrival", Price, "New arrival stamp", "新品印章", "NEW", "`text` is the stamp's word."),
    Promo {
        value: Some(50.0),
        lines: &["11.11 前有效"],
        ..piece(
            "coupon",
            Price,
            "Coupon",
            "优惠券",
            "满 299 可用",
            "`value` is what it is worth, `text` its condition, `lines[0]` until when it holds.",
        )
    },
    Promo {
        lines: &["满 200 减 30", "满 400 减 80", "满 600 减 150"],
        ..piece(
            "tiers",
            Price,
            "Spend-and-save tiers",
            "满减阶梯",
            "",
            "`lines` are the tiers, two to four, the best last.",
        )
    },
    Promo {
        value: Some(87.0),
        ..piece(
            "flashProgress",
            Price,
            "Flash-sale progress",
            "秒杀进度条",
            "已抢 {n}%",
            "`value` is the share claimed, 0 to 100; the bar fills to it as the number runs.",
        )
    },
    piece("bogo", Price, "Buy-one-get-one badge", "买赠徽章", "买一送一", "`text` is the offer."),
    Promo {
        value: Some(3.0),
        from: 20.0,
        ..piece(
            "lowStock",
            Price,
            "Low stock",
            "库存告急",
            "仅剩 {n} 件",
            "It counts down `from` a number to `value`, what is left.",
        )
    },
    Promo {
        lines: &["市场价", "本店价"],
        values: &[299.0, 199.0],
        ..piece(
            "priceCompare",
            Price,
            "Price comparison",
            "价格对比条",
            "立省 ¥{n}",
            "`lines` name the two prices and `values` are them, the dearer first; `text` shows the difference as {n}.",
        )
    },
    Promo {
        lines: &["包邮", "正品保障", "七天无理由"],
        ..piece("trustBadges", Price, "Trust badges", "信任角标组", "", "`lines` are the promises, one to four.")
    },
    piece("hotBurst", Price, "Best-seller burst", "热卖爆炸贴", "热卖", "`text` is the word on the burst."),
    Promo {
        ..piece(
            "callout",
            Product,
            "Feature callout",
            "卖点标注",
            "316 不锈钢内胆",
            "`x` and `y` are the point on the product it points at; `text` is the label, on the `side` (right or left) it leans to: away from the middle of the frame, unless told or short of room.",
        )
    },
    Promo {
        lines: &["持久保湿 12 小时", "温和不含香精", "敏感肌适用"],
        ..piece("checklist", Product, "Feature checklist", "卖点清单", "", "`lines` are the features, two to five.")
    },
    Promo {
        lines: &["容量：500 ml", "材质：316 不锈钢", "保温：12 小时"],
        ..piece(
            "specTable",
            Product,
            "Spec table",
            "规格参数表",
            "",
            "`lines` are the rows, two to five, each a name and a value with a colon between.",
        )
    },
    Promo {
        value: Some(4.9),
        lines: &["2.3 万条好评"],
        ..piece(
            "rating",
            Product,
            "Star rating",
            "五星评分",
            "{n} 分",
            "`value` is the rating out of 5, `lines[0]` what it rests on.",
        )
    },
    Promo {
        lines: &["小鹿"],
        ..piece(
            "reviewBubble",
            Product,
            "Review bubble",
            "评价气泡",
            "用了两周，真的不卡粉，回购第三瓶了！",
            "`text` is what the buyer said, `lines[0]` who said it.",
        )
    },
    Promo {
        lines: &["新品上市 · 限时尝鲜价"],
        place: [0.0, 0.3],
        ..piece(
            "lowerThird",
            Titles,
            "Title bar",
            "标题条",
            "春季焕新系列",
            "`text` is the heading, `lines[0]` the line under it.",
        )
    },
    piece("hookQuestion", Titles, "Hook question", "钩子问句", "还在用旧款？", "`text` is the question, typed in."),
    Promo {
        value: Some(98.0),
        lines: &["用户回购率"],
        ..piece(
            "bigNumber",
            Titles,
            "Big number",
            "数字强调",
            "{n}%",
            "`value` is the number it runs up to, `text` the number with its unit, `lines[0]` what it measures.",
        )
    },
    Promo {
        value: Some(1.0),
        ..piece(
            "indexPlate",
            Titles,
            "Numbered heading",
            "卖点序号牌",
            "一键速热",
            "`value` is the number (1 is shown as 01), `text` the heading.",
        )
    },
    Promo {
        place: [0.0, 0.36],
        ..piece("buyButton", Action, "Buy button", "购买按钮", "立即抢购", "`text` is what the button says.")
    },
    Promo {
        place: [0.0, 0.3],
        ..piece(
            "arrowHint",
            Action,
            "Pointing arrow",
            "箭头指引",
            "点击下方链接",
            "`text` is the hint; `side` is where the arrow points: down, up, left or right.",
        )
    },
    Promo {
        lines: &["关注", "收藏", "加购"],
        place: [0.0, 0.3],
        ..piece("followPrompt", Action, "Follow prompt", "关注收藏提示", "", "`lines` are what to do, one to four.")
    },
    Promo {
        lines: &["地址：人民路 88 号", "营业：10:00–22:00", "电话：400-000-0000"],
        ..piece(
            "storeCard",
            Action,
            "Store card",
            "店铺名片",
            "鹿角巷旗舰店",
            "`text` is the shop's name, `lines` what to know about it, one to four.",
        )
    },
    Promo {
        lines: &["活动截至 11 月 11 日 24:00"],
        ..piece(
            "endCard",
            Action,
            "End card",
            "结尾卡",
            "好物不等人",
            "`text` is the closing line, `lines[0]` the small print under it; the brand's logo, if it has one, sits above.",
        )
    },
    Promo {
        millis: 900,
        ..piece(
            "colorSweep",
            Rhythm,
            "Colour sweep",
            "品牌色扫屏",
            "",
            "Three bands of colour cross the frame, to the `side` asked (right, left, up or down); put its middle on a cut to cover it.",
        )
    },
    Promo {
        value: Some(129.0),
        lines: &["立即抢购"],
        millis: 4000,
        pictured: true,
        ..piece(
            "productCard",
            Product,
            "Product card",
            "商品卡",
            "保温杯 Pro",
            "A card that swings in and rocks, carrying a picture, a name, a price and a button. `text` is the name, `value` the price it counts to, `lines[0]` the button's words; `media` is the picture on it (an image or video of the project).",
        )
    },
    Promo {
        lines: &["24 小时长效保温"],
        millis: 4000,
        pictured: true,
        ..piece(
            "productStage",
            Product,
            "Product stage",
            "商品展台",
            "保温杯 Pro",
            "A product lifted off the backdrop of its photograph and stood in a pool of light, its shadow on the floor, its name under it. `media` is the photograph (an image or video of the project, taken against a plain backdrop: see clip.setCutout), `text` the name, `lines[0]` a line under the name; `color` is the light's.",
        )
    },
    Promo {
        lines: &["到手价 ¥99"],
        millis: 4000,
        ..piece(
            "flipCard",
            Price,
            "Flip card",
            "翻转卡",
            "原价 ¥199",
            "A card that shows `text`, then turns over to show `lines[0]` on its back: the old price and the new, a question and its answer.",
        )
    },
    piece(
        "posterWords",
        Titles,
        "Poster words",
        "大字报",
        "全场 五折 起",
        "Very large words that pop in one after another on a block of colour; a space in `text` starts the next line.",
    ),
    Promo {
        place: [0.0, 0.3],
        ..piece(
            "karaokeCaption",
            Titles,
            "Highlighted caption",
            "高亮字幕",
            "今晚八点 直播间见",
            "A caption whose words take the main colour as they are said, over the piece's length.",
        )
    },
    piece(
        "elasticTitle",
        Titles,
        "Springy title",
        "弹性标题",
        "新品上市",
        "A title whose characters spring up one after another, over a line that is drawn under it; `lines[0]` is the smaller line below.",
    ),
    piece(
        "handwritten",
        Titles,
        "Hand-written title",
        "手写标题",
        "Thank you",
        "A title written on by a pen, with a stroke drawn under it.",
    ),
];

/// Which way a piece leans or points.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Right,
    Left,
    Up,
    Down,
}

impl Side {
    pub(crate) const NAMES: [&'static str; 4] = ["right", "left", "up", "down"];

    pub(crate) fn parse(name: &str) -> Option<Side> {
        Some(match name {
            "right" => Side::Right,
            "left" => Side::Left,
            "up" => Side::Up,
            "down" => Side::Down,
            _ => return None,
        })
    }
}

/// What a piece is asked to say and look like.
pub(crate) struct Ask {
    pub text: String,
    pub lines: Vec<String>,
    pub from: f32,
    pub value: Option<f32>,
    pub values: Vec<f32>,
    /// Its main colour, and the one beside it.
    pub accent: [f32; 4],
    pub second: [f32; 4],
    pub side: Option<Side>,
    pub dur: Tick,
    /// The canvas in the piece's own pixels: its shorter side is 1080.
    pub canvas: [f32; 2],
    /// Where the piece goes, as shares of the frame from its centre, and how large it is made.
    pub place: [f32; 2],
    pub scale: f32,
    /// The brand's mark, an image among the media.
    pub logo: Option<String>,
    /// The picture the piece was given to show, an image or video among the media.
    pub media: Option<String>,
}

/// Where a property is, moment by moment: a share of the part's length in thousandths, the value
/// there, and how it moves on to the next.
pub(crate) type Moves = Vec<(i64, f32, Ease)>;

/// One clip of a piece, before it has a place.
pub(crate) struct Part {
    pub track: TrackKind,
    pub content: ClipContent,
    /// Offset from the piece's centre, in the piece's pixels, and turn.
    pub at: [f32; 2],
    pub rotation: f32,
    /// How it comes in, and for how long (0.6 s when none is said).
    pub enter: Option<AnimKind>,
    pub enter_for: Option<Tick>,
    pub repeat: Option<LoopKind>,
    /// How long after the piece starts it does.
    pub delay: Tick,
    /// A title set from the left in a box this wide, centred on `at`.
    pub boxed: Option<f32>,
    /// A centred title whose lines wrap at this width.
    pub wrapped: Option<f32>,
    /// Keys, each at a share of the part's length in thousandths. X and Y are in the piece's
    /// pixels, from where the part sits.
    pub keys: Vec<(Prop, Moves)>,
    /// A picture's size, as a share of the frame it would fill.
    pub fit: Option<f32>,
    /// The part of the piece that carries it, by its place in the list (one before it): `at` is
    /// then counted from where that part sits, and it moves, grows and turns with it.
    pub carried: Option<usize>,
    /// Smeared along its way while it moves (motion blur).
    pub blur: bool,
    /// A picture lifted off its plain backdrop and stood on the ground, its shadow under it.
    pub stood: bool,
}

fn secs(s: f64) -> Tick {
    Tick::from_seconds(s)
}

impl Part {
    fn new(track: TrackKind, content: ClipContent) -> Part {
        Part {
            track,
            content,
            at: [0.0; 2],
            rotation: 0.0,
            enter: None,
            enter_for: None,
            repeat: None,
            delay: Tick::ZERO,
            boxed: None,
            wrapped: None,
            keys: Vec::new(),
            fit: None,
            carried: None,
            blur: false,
            stood: false,
        }
    }

    fn stood(mut self) -> Part {
        self.stood = true;
        self
    }

    /// A picture of the project's media, `fit` of the frame it would fill.
    fn picture(media: &str, fit: f32) -> Part {
        let mut part = Part::new(
            TrackKind::Video,
            ClipContent::Media {
                media: media.to_string(),
                source_in: Tick::ZERO,
                speed: 1.0,
                reversed: false,
                curve: vec![],
                silent: true,
                frames: Inbetween::default(),
            },
        );
        part.fit = Some(fit);
        part
    }

    /// Carried by the part at `i` of the list.
    fn on(mut self, i: usize) -> Part {
        self.carried = Some(i);
        self
    }

    fn blurred(mut self) -> Part {
        self.blur = true;
        self
    }

    fn shape(shape: Shape) -> Part {
        Part::new(TrackKind::Video, ClipContent::Shape { shape })
    }

    /// A heavy title on one line.
    fn text(text: impl Into<String>, size: f32, color: [f32; 4]) -> Part {
        let style = TitleStyle { weight: 900, wrap: 0.0, ..TitleStyle::default() };
        Part::new(TrackKind::Text, ClipContent::Text { text: text.into(), size, color, style })
    }

    fn at(mut self, x: f32, y: f32) -> Part {
        self.at = [x, y];
        self
    }

    fn turned(mut self, degrees: f32) -> Part {
        self.rotation = degrees;
        self
    }

    fn enter(mut self, kind: AnimKind) -> Part {
        self.enter = Some(kind);
        self
    }

    fn over(mut self, seconds: f64) -> Part {
        self.enter_for = Some(secs(seconds));
        self
    }

    fn after(mut self, seconds: f64) -> Part {
        self.delay = secs(seconds);
        self
    }

    fn looped(mut self, kind: LoopKind) -> Part {
        self.repeat = Some(kind);
        self
    }

    fn boxed(mut self, width: f32) -> Part {
        self.boxed = Some(width);
        self
    }

    fn wrapped(mut self, width: f32) -> Part {
        self.wrapped = Some(width);
        self
    }

    fn styled(mut self, f: impl FnOnce(&mut TitleStyle)) -> Part {
        if let ClipContent::Text { style, .. } = &mut self.content {
            f(style);
        }
        self
    }

    fn counting(self, counter: Option<Counter>) -> Part {
        self.styled(|s| s.counter = counter)
    }

    fn keyed(mut self, prop: Prop, keys: Moves) -> Part {
        self.keys.push((prop, keys));
        self
    }
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const INK: [f32; 4] = [0.1, 0.1, 0.13, 1.0];
const GREY: [f32; 4] = [0.45, 0.46, 0.5, 1.0];
const GOLD: [f32; 4] = [1.0, 0.78, 0.12, 1.0];
const DARK: [f32; 4] = [0.07, 0.07, 0.1, 0.78];
/// The main colour and the one beside it, for a project with no brand.
pub(crate) const RED: [f32; 4] = [0.93, 0.16, 0.2, 1.0];
pub(crate) const YELLOW: [f32; 4] = [1.0, 0.8, 0.2, 1.0];

/// The colour words are set in over `c`: white, or ink when `c` is light.
fn over(c: [f32; 4]) -> [f32; 4] {
    if 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] > 0.62 { INK } else { WHITE }
}

fn lighter(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0] + (1.0 - c[0]) * k, c[1] + (1.0 - c[1]) * k, c[2] + (1.0 - c[2]) * k, c[3]]
}

fn faded(c: [f32; 4], alpha: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * alpha]
}

fn shadow() -> Option<ShapeShadow> {
    Some(ShapeShadow { color: [0.0, 0.0, 0.0, 0.35], offset: [0.0, 10.0], blur: 14.0 })
}

/// A box with rounded corners.
fn plate(size: [f32; 2], radius: f32, color: [f32; 4]) -> Shape {
    Shape { radius, ..Shape::new(ShapeKind::Rectangle, size, color) }
}

/// A box that runs from a lighter shade of `color` into it.
fn glossy(size: [f32; 2], radius: f32, color: [f32; 4], angle: f32) -> Shape {
    Shape {
        fill: Some(Fill::Linear { from: lighter(color, 0.3), to: color, angle }),
        shadow: shadow(),
        ..plate(size, radius, color)
    }
}

/// A stroke `length` long and `width` thick, its ends round.
fn bar(length: f32, width: f32, color: [f32; 4]) -> Shape {
    Shape { stroke: Some(Stroke { color, width }), ..Shape::new(ShapeKind::Line, [length, 0.0], color) }
}

/// About how wide `text` is set at `size`: a full em for a wide character, less for a narrow one.
fn wide(text: &str, size: f32) -> f32 {
    // as bold letters run; a little over rather than under, so that words stay on their plate
    let em = |c: char| match c {
        ' ' => 0.26,
        'i' | 'j' | 'l' | 'I' | '.' | ',' | ':' | ';' | '!' | '\'' | '|' => 0.31,
        'f' | 't' | 'r' | '(' | ')' | '-' | '/' => 0.42,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.92,
        'A'..='Z' => 0.68,
        c if c.is_ascii() => 0.6,
        _ => 1.0,
    };
    text.chars().map(em).sum::<f32>() * size
}

/// The letter size, `size` or less, at which `text` is about `most` wide: long words are set
/// smaller before they run off their plate, down to a little over half the size.
fn fit(text: &str, size: f32, most: f32) -> f32 {
    let needs = wide(text, size);
    if needs <= most { size } else { (size * most / needs).max(size * 0.55) }
}

/// The digits after the point a number is shown with.
fn decimals(v: f32) -> u8 {
    if v.fract() == 0.0 {
        0
    } else if (v * 10.0).fract().abs() < 1e-3 {
        1
    } else {
        2
    }
}

/// Things `width` wide each, laid in rows no wider than `most` with `gap` between: where the
/// centre of each goes, from the centre of them all; rows are `row` apart.
fn flow(widths: &[f32], most: f32, gap: f32, row: f32) -> Vec<[f32; 2]> {
    let mut rows: Vec<Vec<usize>> = vec![vec![]];
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let last = rows.last_mut().expect("a row");
        if !last.is_empty() && used + gap + w > most {
            rows.push(vec![i]);
            used = *w;
        } else {
            used += if last.is_empty() { *w } else { gap + w };
            last.push(i);
        }
    }
    let mut out = vec![[0.0; 2]; widths.len()];
    let n = rows.len() as f32;
    for (r, items) in rows.iter().enumerate() {
        let total: f32 = items.iter().map(|&i| widths[i]).sum::<f32>() + gap * (items.len() as f32 - 1.0);
        let mut x = -total / 2.0;
        for &i in items {
            out[i] = [x + widths[i] / 2.0, (r as f32 - (n - 1.0) / 2.0) * row];
            x += widths[i] + gap;
        }
    }
    out
}

/// Where row `i` of `n` goes, `step` apart, around the centre.
fn row(i: usize, n: usize, step: f32) -> f32 {
    (i as f32 - (n as f32 - 1.0) / 2.0) * step
}

/// The clips of piece `id`, bottom first.
pub(crate) fn parts(id: &str, ask: &Ask) -> Option<Vec<Part>> {
    let (accent, second, dur) = (ask.accent, ask.second, ask.dur);
    let text = || ask.text.clone();
    let line = |i: usize| ask.lines.get(i).cloned().unwrap_or_default();
    let counter = |to: f32, group: bool, clock: bool, ease: Ease, over: Option<Tick>| Counter {
        from: ask.from,
        to,
        over,
        ease,
        decimals: decimals(to),
        group,
        clock,
    };
    let quick = Some(secs(1.2).min(dur));
    let n = ask.lines.len();
    Some(match id {
        "priceTag" => vec![
            Part::shape(Shape {
                radius: 28.0,
                fill: Some(Fill::Linear { from: lighter(accent, 0.25), to: accent, angle: 90.0 }),
                shadow: shadow(),
                ..Shape::new(ShapeKind::Tag, [560.0, 210.0], accent)
            })
            .enter(AnimKind::Pop),
            Part::text(text(), 120.0, over(accent)).at(40.0, 0.0).enter(AnimKind::ZoomIn).counting(ask.value.map(
                |v| Counter {
                    decimals: if v.fract() != 0.0 { 2 } else { 0 },
                    ..counter(v, false, false, Ease::OUT, quick)
                },
            )),
        ],
        "discountBadge" => {
            let mut star = Shape {
                points: 16,
                inner: 0.82,
                stroke: Some(Stroke { color: WHITE, width: 8.0 }),
                shadow: shadow(),
                ..Shape::new(ShapeKind::Star, [380.0, 380.0], accent)
            };
            star.fill = Some(Fill::Radial { from: lighter(accent, 0.3), to: accent });
            vec![
                Part::shape(star).enter(AnimKind::Pop).looped(LoopKind::Heartbeat).turned(-12.0),
                Part::text(text(), fit(&ask.text, 104.0, 300.0), over(accent))
                    .enter(AnimKind::Pop)
                    .looped(LoopKind::Heartbeat)
                    .turned(-12.0),
            ]
        }
        "banner" => vec![
            Part::shape(Shape {
                fill: Some(Fill::Linear { from: lighter(accent, 0.35), to: accent, angle: 0.0 }),
                shadow: shadow(),
                ..plate([980.0, 180.0], 90.0, accent)
            })
            .enter(AnimKind::SlideRight),
            Part::text(text(), fit(&ask.text, 92.0, 860.0), over(accent)).enter(AnimKind::CharPop),
        ],
        "countdown" => {
            // it counts down from its value to nothing, at the speed of a clock
            let start = ask.value.unwrap_or(10.0);
            let clock = Counter {
                from: start,
                to: (start - dur.seconds() as f32).max(0.0),
                ..counter(0.0, false, true, Ease::Linear, None)
            };
            vec![
                Part::shape(Shape {
                    stroke: Some(Stroke { color: accent, width: 8.0 }),
                    shadow: shadow(),
                    ..plate([520.0, 200.0], 36.0, [0.08, 0.08, 0.1, 0.88])
                })
                .enter(AnimKind::Draw),
                Part::text(text(), 112.0, WHITE).enter(AnimKind::Fade).counting(Some(clock)),
            ]
        }
        "salesCounter" => vec![
            Part::shape(Shape { shadow: shadow(), ..plate([700.0, 160.0], 80.0, WHITE) }).enter(AnimKind::ZoomIn),
            Part::text(text(), 78.0, accent).enter(AnimKind::Fade).counting(ask.value.map(|v| Counter {
                decimals: if v.fract() != 0.0 { 2 } else { 0 },
                ..counter(v, true, false, Ease::OUT, Some(secs(2.0).min(dur)))
            })),
        ],
        "strikePrice" => {
            let width = ask.text.chars().count() as f32 * 72.0 * 0.62 + 30.0;
            vec![
                Part::text(text(), 72.0, [0.6, 0.6, 0.62, 1.0]).enter(AnimKind::Fade),
                Part::shape(bar(width, 7.0, accent)).at(0.0, 4.0).turned(-8.0).enter(AnimKind::Draw),
            ]
        }
        "newArrival" => vec![
            Part::shape(Shape {
                stroke: Some(Stroke { color: accent, width: 12.0 }),
                fill: Some(Fill::Solid { color: [accent[0], accent[1], accent[2], 0.18] }),
                ..Shape::new(ShapeKind::Ellipse, [320.0, 320.0], accent)
            })
            .enter(AnimKind::Draw),
            Part::text(text(), fit(&ask.text, 104.0, 250.0), accent).enter(AnimKind::CharPop),
        ],
        "coupon" => {
            let mut v = vec![
                Part::shape(glossy([980.0, 300.0], 30.0, accent, 0.0)).enter(AnimKind::SlideRight),
                Part::shape(bar(210.0, 4.0, faded(over(accent), 0.55)))
                    .at(-110.0, 0.0)
                    .turned(90.0)
                    .enter(AnimKind::Draw)
                    .after(0.2),
                Part::text("¥{n}", 150.0, over(accent))
                    .at(-300.0, 0.0)
                    .enter(AnimKind::Pop)
                    .after(0.15)
                    .counting(Some(counter(ask.value.unwrap_or(50.0), false, false, Ease::OUT, quick))),
                Part::text(text(), fit(&ask.text, 62.0, 520.0), over(accent))
                    .at(190.0, if n > 0 { -38.0 } else { 0.0 })
                    .enter(AnimKind::Fade)
                    .after(0.3),
            ];
            if n > 0 {
                v.push(
                    Part::text(line(0), fit(&line(0), 40.0, 520.0), faded(over(accent), 0.85))
                        .at(190.0, 48.0)
                        .enter(AnimKind::Fade)
                        .after(0.4),
                );
            }
            v
        }
        "tiers" => {
            let mut v = vec![];
            for (i, words) in ask.lines.iter().enumerate() {
                let (best, wait) = (i + 1 == n && n > 1, i as f64 * 0.25);
                let y = row(i, n, 164.0);
                let pill = if best {
                    glossy([800.0, 140.0], 70.0, second, 0.0)
                } else {
                    glossy([760.0, 130.0], 65.0, accent, 0.0)
                };
                let mut plate = Part::shape(pill).at(0.0, y).enter(AnimKind::Pop).after(wait);
                let mut label =
                    Part::text(words.clone(), if best { 78.0 } else { 70.0 }, over(if best { second } else { accent }))
                        .at(0.0, y)
                        .enter(AnimKind::Fade)
                        .after(wait);
                if best {
                    plate = plate.looped(LoopKind::Heartbeat);
                    label = label.looped(LoopKind::Heartbeat);
                }
                v.push(plate);
                v.push(label);
            }
            v
        }
        "flashProgress" => {
            let share = (ask.value.unwrap_or(87.0) / 100.0).clamp(0.0, 1.0);
            let mut filled = bar(800.0, 44.0, accent);
            filled.trim = [0.0, share];
            vec![
                Part::shape(bar(800.0, 44.0, faded(WHITE, 0.3))).at(0.0, 40.0).enter(AnimKind::Fade),
                Part::shape(filled).at(0.0, 40.0).enter(AnimKind::Draw).over(1.2),
                Part::text(text(), fit(&ask.text, 64.0, 900.0), WHITE)
                    .at(0.0, -50.0)
                    .enter(AnimKind::Fade)
                    .counting(ask.value.map(|v| counter(v, false, false, Ease::OUT, quick))),
            ]
        }
        "bogo" => {
            let mut star = Shape {
                points: 20,
                inner: 0.86,
                stroke: Some(Stroke { color: second, width: 10.0 }),
                shadow: shadow(),
                ..Shape::new(ShapeKind::Star, [440.0, 440.0], accent)
            };
            star.fill = Some(Fill::Radial { from: lighter(accent, 0.3), to: accent });
            vec![
                Part::shape(star).enter(AnimKind::Pop).looped(LoopKind::Swing).turned(-10.0),
                Part::text(text(), fit(&ask.text, 84.0, 350.0), over(accent))
                    .enter(AnimKind::Pop)
                    .looped(LoopKind::Swing)
                    .turned(-10.0),
            ]
        }
        "lowStock" => {
            let to = ask.value.unwrap_or(3.0);
            let size = fit(&ask.text, 80.0, 800.0);
            vec![
                Part::shape(Shape {
                    stroke: Some(Stroke { color: accent, width: 6.0 }),
                    shadow: shadow(),
                    ..plate([wide(&ask.text, size) + 150.0, 150.0], 75.0, [0.08, 0.08, 0.1, 0.9])
                })
                .enter(AnimKind::ZoomIn)
                .looped(LoopKind::Heartbeat),
                Part::text(text(), size, lighter(accent, 0.25))
                    .enter(AnimKind::Fade)
                    .looped(LoopKind::Heartbeat)
                    .counting(Some(counter(to, false, false, Ease::OUT, Some(secs(1.5).min(dur))))),
            ]
        }
        "priceCompare" => {
            let (a, b) = (ask.values.first().copied().unwrap_or(299.0), ask.values.get(1).copied().unwrap_or(199.0));
            let most = a.abs().max(b.abs()).max(1e-3);
            let price = |v: f32| if v.fract() == 0.0 { format!("¥{v:.0}") } else { format!("¥{v:.2}") };
            let mut v = vec![];
            for (i, (value, colour, tall)) in
                [(a, [0.62, 0.63, 0.68, 1.0], 40.0), (b, accent, 52.0)].into_iter().enumerate()
            {
                let (y, wait) = (-150.0 + i as f32 * 130.0, i as f64 * 0.35);
                let length = (520.0 * value.abs() / most).max(40.0);
                v.push(
                    Part::text(line(i), fit(&line(i), 50.0, 220.0), WHITE)
                        .boxed(230.0)
                        .at(-355.0, y)
                        .enter(AnimKind::Fade)
                        .after(wait),
                );
                v.push(
                    Part::shape(bar(length, tall, colour))
                        .at(-220.0 + length / 2.0, y)
                        .enter(AnimKind::Draw)
                        .over(0.7)
                        .after(wait),
                );
                v.push(
                    Part::text(
                        price(value),
                        if i == 0 { 50.0 } else { 62.0 },
                        if i == 0 { faded(WHITE, 0.8) } else { WHITE },
                    )
                    .boxed(230.0)
                    .at(-220.0 + length + 150.0, y)
                    .enter(AnimKind::Fade)
                    .after(wait + 0.5),
                );
            }
            v.push(
                Part::text(text(), fit(&ask.text, 100.0, 900.0), second)
                    .at(0.0, 140.0)
                    .enter(AnimKind::Pop)
                    .after(1.0)
                    .counting(Some(counter(a - b, false, false, Ease::OUT, Some(secs(0.8).min(dur))))),
            );
            v
        }
        "trustBadges" | "followPrompt" => {
            let follow = id == "followPrompt";
            let size = if follow { 58.0 } else { 46.0 };
            let words: Vec<String> =
                ask.lines.iter().map(|l| if follow { format!("+ {l}") } else { format!("✓ {l}") }).collect();
            let widths: Vec<f32> = words.iter().map(|w| wide(w, size) + size * 1.7).collect();
            let tall = size * 2.1;
            let mut v = vec![];
            // pills that bounce need more room between rows
            let apart = tall + if follow { 64.0 } else { 26.0 };
            for (i, at) in flow(&widths, 1000.0, 22.0, apart).into_iter().enumerate() {
                let wait = i as f64 * 0.2;
                let pill = if follow {
                    glossy([widths[i], tall], tall / 2.0, accent, 0.0)
                } else {
                    Shape { shadow: shadow(), ..plate([widths[i], tall], tall / 2.0, faded(WHITE, 0.96)) }
                };
                let mut plate = Part::shape(pill).at(at[0], at[1]).enter(AnimKind::Pop).after(wait);
                let mut label = Part::text(words[i].clone(), size, if follow { over(accent) } else { accent })
                    .at(at[0], at[1])
                    .enter(AnimKind::Fade)
                    .after(wait);
                if follow {
                    plate = plate.looped(LoopKind::Bounce);
                    label = label.looped(LoopKind::Bounce);
                }
                v.push(plate);
                v.push(label);
            }
            v
        }
        "hotBurst" => {
            let mut star = Shape {
                points: 14,
                inner: 0.72,
                stroke: Some(Stroke { color: second, width: 8.0 }),
                shadow: shadow(),
                ..Shape::new(ShapeKind::Star, [380.0, 380.0], accent)
            };
            star.fill = Some(Fill::Radial { from: lighter(accent, 0.3), to: accent });
            vec![
                Part::shape(star).enter(AnimKind::Spin).looped(LoopKind::Heartbeat),
                Part::text(text(), fit(&ask.text, 112.0, 250.0), second)
                    .enter(AnimKind::Pop)
                    .looped(LoopKind::Heartbeat)
                    .turned(-8.0),
            ]
        }
        "callout" => {
            let (reach, rise) = (130.0, 116.0);
            let length = f32::hypot(reach, rise);
            let letters = fit(&ask.text, 46.0, 600.0);
            let w = wide(&ask.text, letters) + 70.0;
            // it leans away from the middle of the frame, unless its label would run off the edge
            let left = match ask.side {
                Some(side) => side == Side::Left,
                None => {
                    let (half, x) = (ask.canvas[0] / 2.0 / ask.scale, ask.place[0] * ask.canvas[0] / ask.scale);
                    let fits = |dir: f32| (x + dir * (reach + w - 12.0)).abs() <= half;
                    if x < 0.0 { fits(-1.0) || !fits(1.0) } else { !fits(1.0) && fits(-1.0) }
                }
            };
            let dir = if left { -1.0 } else { 1.0 };
            let turn = f32::atan2(rise, reach).to_degrees();
            vec![
                Part::shape(bar(length, 6.0, WHITE))
                    .at(dir * reach / 2.0, -rise / 2.0)
                    .turned(if left { 180.0 + turn } else { -turn })
                    .enter(AnimKind::Draw)
                    .over(0.35)
                    .after(0.2),
                Part::shape(Shape {
                    stroke: Some(Stroke { color: WHITE, width: 7.0 }),
                    shadow: shadow(),
                    ..Shape::new(ShapeKind::Ellipse, [40.0, 40.0], accent)
                })
                .enter(AnimKind::Pop)
                .over(0.3)
                .looped(LoopKind::Heartbeat),
                Part::shape(Shape { shadow: shadow(), ..plate([w, 92.0], 46.0, WHITE) })
                    .at(dir * (reach + w / 2.0 - 12.0), -rise)
                    .enter(AnimKind::ZoomIn)
                    .over(0.3)
                    .after(0.45),
                Part::text(text(), letters, INK)
                    .at(dir * (reach + w / 2.0 - 12.0), -rise)
                    .enter(AnimKind::Fade)
                    .over(0.3)
                    .after(0.55),
            ]
        }
        "checklist" => {
            // a row of long words is set smaller, and past that takes the rows it needs
            let rows: Vec<(String, f32, f32)> = ask
                .lines
                .iter()
                .map(|words| {
                    let said = format!("✓  {words}");
                    let size = fit(&said, 58.0, 715.0).max(44.0);
                    let wraps = (wide(&said, size) / 715.0).ceil().clamp(1.0, 3.0);
                    (said, size, 112.0 + (wraps - 1.0) * size * 1.3)
                })
                .collect();
            let tall: f32 = rows.iter().map(|r| r.2).sum();
            let mut v = vec![Part::shape(plate([840.0, tall + 64.0], 36.0, DARK)).enter(AnimKind::Fade)];
            let mut top = -tall / 2.0;
            for (i, (said, size, high)) in rows.into_iter().enumerate() {
                v.push(
                    Part::text(said, size, WHITE)
                        .boxed(730.0)
                        .at(0.0, top + high / 2.0)
                        .enter(AnimKind::SlideRight)
                        .after(0.2 + i as f64 * 0.22),
                );
                top += high;
            }
            v
        }
        "specTable" => {
            let step = 104.0;
            let mut v = vec![
                Part::shape(Shape {
                    shadow: shadow(),
                    ..plate([880.0, n as f32 * step + 56.0], 30.0, faded(WHITE, 0.97))
                })
                .enter(AnimKind::ZoomIn),
            ];
            for (i, words) in ask.lines.iter().enumerate() {
                let (name, value) = match words.split_once(['：', ':']) {
                    Some((name, value)) => (name.trim(), value.trim()),
                    None => ("", words.trim()),
                };
                let (y, wait) = (row(i, n, step), 0.2 + i as f64 * 0.18);
                v.push(Part::text(name, 44.0, GREY).boxed(260.0).at(-270.0, y).enter(AnimKind::Fade).after(wait));
                v.push(
                    Part::text(value, 50.0, INK).boxed(500.0).at(130.0, y).enter(AnimKind::SlideUp).after(wait + 0.05),
                );
            }
            v
        }
        "rating" => {
            let score = ask.value.unwrap_or(4.9).clamp(0.0, 5.0);
            let full = score.round().clamp(0.0, 5.0) as usize;
            let stars: String = "★".repeat(full) + &"☆".repeat(5 - full);
            let mut v = vec![
                Part::text(text(), 130.0, WHITE)
                    .at(0.0, -120.0)
                    .enter(AnimKind::ZoomIn)
                    .counting(Some(Counter { decimals: 1, ..counter(score, false, false, Ease::OUT, quick) })),
                Part::text(stars, 100.0, GOLD)
                    .at(0.0, 20.0)
                    .enter(AnimKind::CharPop)
                    .over(1.0)
                    .after(0.2)
                    .styled(|s| s.tracking = 0.08),
            ];
            if n > 0 {
                v.push(Part::text(line(0), 46.0, faded(WHITE, 0.88)).at(0.0, 122.0).enter(AnimKind::Fade).after(0.9));
            }
            v
        }
        "reviewBubble" => {
            let inner = 740.0;
            let rows = (wide(&ask.text, 50.0) / inner).ceil().clamp(1.0, 6.0);
            let (name_room, pitch) = (if n > 0 { 78.0 } else { 0.0 }, 68.0);
            let tall = rows * pitch + name_room + 84.0;
            let mut v = vec![
                Part::shape(Shape {
                    points: 3,
                    shadow: shadow(),
                    ..Shape::new(ShapeKind::Polygon, [70.0, 60.0], WHITE)
                })
                .at(-300.0, tall / 2.0 + 8.0)
                .turned(180.0)
                .enter(AnimKind::Pop),
                Part::shape(Shape { shadow: shadow(), ..plate([860.0, tall], 44.0, WHITE) }).enter(AnimKind::Pop),
            ];
            if n > 0 {
                v.push(
                    Part::text(format!("@{}  ★★★★★", line(0)), 42.0, accent)
                        .boxed(inner)
                        .at(0.0, -tall / 2.0 + 42.0 + 36.0)
                        .enter(AnimKind::Fade)
                        .after(0.2),
                );
            }
            v.push(
                Part::text(text(), 50.0, INK)
                    .boxed(inner)
                    .at(0.0, name_room / 2.0)
                    .enter(AnimKind::Fade)
                    .after(0.35)
                    .styled(|s| s.weight = 700),
            );
            v
        }
        "lowerThird" => {
            let mut v = vec![
                Part::shape(plate([900.0, 160.0], 14.0, DARK)).enter(AnimKind::SlideRight),
                Part::shape(plate([14.0, 160.0], 7.0, accent)).at(-443.0, 0.0).enter(AnimKind::SlideUp),
                Part::text(text(), fit(&ask.text, 66.0, 790.0), WHITE)
                    .boxed(800.0)
                    .at(10.0, if n > 0 { -30.0 } else { 0.0 })
                    .enter(AnimKind::SlideRight)
                    .after(0.15),
            ];
            if n > 0 {
                v.push(
                    Part::text(line(0), fit(&line(0), 40.0, 790.0), faded(WHITE, 0.85))
                        .boxed(800.0)
                        .at(10.0, 38.0)
                        .enter(AnimKind::SlideRight)
                        .after(0.28)
                        .styled(|s| s.weight = 700),
                );
            }
            v
        }
        "hookQuestion" => {
            let typing = (0.08 * ask.text.chars().count() as f64).clamp(0.5, 1.6);
            // the mark sits off the corner of the words' box, however many rows they take
            let rows = (wide(&ask.text, 104.0) / 930.0).ceil().clamp(1.0, 4.0);
            vec![
                Part::text("?", 300.0, second)
                    .at(380.0, -190.0 - (rows - 1.0) * 66.0)
                    .turned(14.0)
                    .enter(AnimKind::Pop)
                    .after(typing)
                    .looped(LoopKind::Swing),
                Part::text(text(), 104.0, WHITE).wrapped(930.0).enter(AnimKind::Typewriter).over(typing).styled(|s| {
                    s.background = Some(DARK);
                    s.radius = 0.3;
                }),
            ]
        }
        "bigNumber" => {
            let mut v = vec![
                Part::text(text(), 300.0, second)
                    .at(0.0, -60.0)
                    .enter(AnimKind::ZoomIn)
                    .counting(ask.value.map(|v| counter(v, false, false, Ease::OUT, quick))),
                Part::shape(bar(340.0, 10.0, accent)).at(0.0, 128.0).enter(AnimKind::Draw).after(0.3),
            ];
            if n > 0 {
                v.push(
                    Part::text(line(0), fit(&line(0), 66.0, 960.0), WHITE)
                        .at(0.0, 210.0)
                        .enter(AnimKind::SlideUp)
                        .after(0.5),
                );
            }
            v
        }
        "indexPlate" => {
            let number = format!("{:02}", ask.value.unwrap_or(1.0).round().clamp(0.0, 99.0) as u32);
            vec![
                Part::shape(Shape { shadow: shadow(), ..plate([900.0, 176.0], 26.0, WHITE) })
                    .enter(AnimKind::SlideRight),
                Part::shape(plate([190.0, 176.0], 26.0, accent)).at(-355.0, 0.0).enter(AnimKind::SlideRight),
                Part::text(number, 112.0, over(accent))
                    .at(-355.0, 0.0)
                    .enter(AnimKind::Pop)
                    .after(0.15)
                    .styled(|s| s.italic = true),
                Part::text(text(), fit(&ask.text, 66.0, 610.0), INK)
                    .boxed(620.0)
                    .at(100.0, 0.0)
                    .enter(AnimKind::Fade)
                    .after(0.25),
            ]
        }
        "buyButton" => {
            let size = fit(&ask.text, 70.0, 760.0);
            let w = (wide(&ask.text, size) + 220.0).max(460.0);
            vec![
                Part::shape(glossy([w, 150.0], 75.0, accent, 0.0)).enter(AnimKind::Pop).looped(LoopKind::Heartbeat),
                Part::text(text(), size, over(accent)).enter(AnimKind::Pop).looped(LoopKind::Heartbeat),
            ]
        }
        "arrowHint" => {
            let side = ask.side.unwrap_or(Side::Down);
            let (turn, to, prop) = match side {
                Side::Right => (0.0, [1.0, 0.0], Prop::X),
                Side::Left => (180.0, [-1.0, 0.0], Prop::X),
                Side::Up => (-90.0, [0.0, -1.0], Prop::Y),
                Side::Down => (90.0, [0.0, 1.0], Prop::Y),
            };
            let across = to[0] != 0.0;
            let off = if across { wide(&ask.text, 54.0) / 2.0 + 150.0 } else { 200.0 };
            // the arrow nudges the way it points, about every 0.7 s for as long as it shows
            let rounds = (dur.seconds() / 0.7).round().clamp(1.0, 60.0) as i64;
            let mut nudge = vec![];
            for r in 0..rounds {
                nudge.push((r * 1000 / rounds, 0.0, Ease::OUT));
                nudge.push(((2 * r + 1) * 500 / rounds, 30.0 * (to[0] + to[1]), Ease::IN));
            }
            nudge.push((1000, 0.0, Ease::Linear));
            vec![
                Part::shape(Shape { shadow: shadow(), ..Shape::new(ShapeKind::Arrow, [190.0, 120.0], accent) })
                    .at(to[0] * off / 2.0, to[1] * off / 2.0)
                    .turned(turn)
                    .enter(AnimKind::Pop)
                    .keyed(prop, nudge),
                Part::text(text(), 54.0, WHITE)
                    .at(-to[0] * off / 2.0, -to[1] * off / 2.0)
                    .enter(AnimKind::Fade)
                    .after(0.15)
                    .styled(|s| {
                        s.background = Some(DARK);
                        s.radius = 0.5;
                    }),
            ]
        }
        "storeCard" => {
            let step = 76.0;
            let tall = 210.0 + n as f32 * step;
            let top = -tall / 2.0;
            let mut v = vec![
                Part::shape(Shape { shadow: shadow(), ..plate([900.0, tall], 38.0, WHITE) }).enter(AnimKind::SlideUp),
                Part::text(text(), fit(&ask.text, 74.0, 770.0), INK)
                    .boxed(780.0)
                    .at(0.0, top + 92.0)
                    .enter(AnimKind::Fade)
                    .after(0.15),
                Part::shape(bar(120.0, 10.0, accent)).at(-330.0, top + 152.0).enter(AnimKind::Draw).after(0.25),
            ];
            for (i, words) in ask.lines.iter().enumerate() {
                v.push(
                    Part::text(words.clone(), fit(words, 44.0, 770.0), GREY)
                        .boxed(780.0)
                        .at(0.0, top + 214.0 + i as f32 * step)
                        .enter(AnimKind::SlideUp)
                        .after(0.35 + i as f64 * 0.15)
                        .styled(|s| s.weight = 700),
                );
            }
            v
        }
        "endCard" => {
            let mut v = vec![];
            if let Some(media) = &ask.logo {
                let mut mark = Part::new(
                    TrackKind::Video,
                    ClipContent::Media {
                        media: media.clone(),
                        source_in: Tick::ZERO,
                        speed: 1.0,
                        reversed: false,
                        curve: vec![],
                        silent: false,
                        frames: Inbetween::default(),
                    },
                )
                .at(0.0, -300.0)
                .enter(AnimKind::ZoomIn);
                mark.fit = Some(0.3);
                v.push(mark);
            }
            // words a little too long are set smaller; longer ones take rows, and the bar above
            // and the line below make room for them
            let size = fit(&ask.text, 112.0, 940.0).max(92.0);
            let more = ((wide(&ask.text, size) / 940.0).ceil().clamp(1.0, 4.0) - 1.0) * size * 0.66;
            v.push(Part::shape(bar(160.0, 10.0, second)).at(0.0, -110.0 - more).enter(AnimKind::Draw).after(0.2));
            v.push(Part::text(text(), size, WHITE).wrapped(950.0).enter(AnimKind::CharRise).over(0.9).after(0.1));
            if n > 0 {
                let small = fit(&line(0), 46.0, 900.0);
                v.push(
                    Part::text(line(0), small, over(accent))
                        .at(0.0, 150.0 + more)
                        .enter(AnimKind::Pop)
                        .after(0.8)
                        .styled(|s| {
                            s.background = Some(accent);
                            s.radius = 0.5;
                        }),
                );
            }
            v
        }
        "colorSweep" => {
            let side = ask.side.unwrap_or(Side::Right);
            // a touch more than the frame, so that nothing of a band is left on it
            let (prop, far) = match side {
                Side::Right => (Prop::X, 1.02 * ask.canvas[0]),
                Side::Left => (Prop::X, -1.02 * ask.canvas[0]),
                Side::Down => (Prop::Y, 1.02 * ask.canvas[1]),
                Side::Up => (Prop::Y, -1.02 * ask.canvas[1]),
            };
            // each band comes from off the frame, covers it, and leaves by the other side, the
            // next a moment behind: the first covers the frame at the middle of the piece
            let bands = [(accent, 0, 760), (second, 120, 880), (WHITE, 240, 1000)];
            bands
                .into_iter()
                .rev()
                .map(|(colour, from, to)| {
                    let mid = (from + to) / 2;
                    Part::shape(plate([ask.canvas[0] + 4.0, ask.canvas[1] + 4.0], 0.0, colour)).keyed(
                        prop,
                        vec![
                            (0, -far, Ease::Linear),
                            (from, -far, Ease::OUT),
                            (mid, 0.0, Ease::IN),
                            (to, far, Ease::Linear),
                            (1000, far, Ease::Linear),
                        ],
                    )
                })
                .collect()
        }
        "productStage" => {
            // light behind the thing and on the floor under it, both wide and soft: it stands
            // in them wherever its foot is in its photograph
            let light = |size: [f32; 2], strength: f32| Shape {
                fill: Some(Fill::Radial { from: faded(lighter(accent, 0.55), strength), to: faded(accent, 0.0) }),
                ..Shape::new(ShapeKind::Ellipse, size, accent)
            };
            let mut v = vec![
                Part::shape(light([1060.0, 1060.0], 0.5)).at(0.0, -110.0).enter(AnimKind::Fade).over(0.8),
                Part::shape(light([900.0, 230.0], 0.7)).at(0.0, 250.0).enter(AnimKind::ZoomIn).over(0.8),
            ];
            match &ask.media {
                Some(media) => {
                    v.push(Part::picture(media, 0.6).at(0.0, -80.0).stood().enter(AnimKind::Pop).over(0.7).after(0.2));
                }
                None => v.push(
                    Part::shape(Shape {
                        fill: Some(Fill::Linear { from: lighter(accent, 0.6), to: lighter(accent, 0.25), angle: 90.0 }),
                        shadow: shadow(),
                        ..plate([300.0, 520.0], 60.0, accent)
                    })
                    .at(0.0, -40.0)
                    .enter(AnimKind::Pop)
                    .over(0.7)
                    .after(0.2),
                ),
            }
            v.push(
                Part::text(text(), fit(&ask.text, 78.0, 920.0), WHITE)
                    .at(0.0, 430.0)
                    .enter(AnimKind::CharRise)
                    .after(0.5),
            );
            if !line(0).is_empty() {
                let words = line(0);
                let size = fit(&words, 44.0, 920.0);
                v.push(Part::text(words, size, faded(WHITE, 0.85)).at(0.0, 520.0).enter(AnimKind::Fade).after(0.8));
            }
            v
        }
        "productCard" => {
            // everything on the card is carried by it, so it swings in and rocks as one object
            let (w, h) = (620.0, 860.0);
            let mut v = vec![
                Part::shape(Shape { shadow: shadow(), ..plate([w, h], 44.0, WHITE) })
                    .enter(AnimKind::FlipY)
                    .over(0.8)
                    .looped(LoopKind::Tilt)
                    .blurred(),
            ];
            // where the picture goes: the picture, or a block of the piece's colour
            match &ask.media {
                Some(media) => v.push(Part::picture(media, 0.5).at(0.0, -150.0).on(0).blurred()),
                None => v.push(
                    Part::shape(Shape {
                        fill: Some(Fill::Linear {
                            from: lighter(accent, 0.45),
                            to: lighter(accent, 0.15),
                            angle: 90.0,
                        }),
                        ..plate([w - 80.0, 460.0], 28.0, accent)
                    })
                    .at(0.0, -160.0)
                    .on(0)
                    .blurred(),
                ),
            }
            v.push(Part::text(text(), fit(&ask.text, 58.0, w - 90.0), INK).at(0.0, 130.0).on(0).blurred());
            if let Some(price) = ask.value {
                v.push(Part::text("¥{n}", 96.0, accent).at(0.0, 235.0).on(0).blurred().counting(Some(counter(
                    price,
                    false,
                    false,
                    Ease::OUT,
                    quick,
                ))));
            }
            if !line(0).is_empty() {
                let words = line(0);
                let wide_ = (wide(&words, 46.0) + 110.0).clamp(240.0, w - 80.0);
                let at = v.len();
                v.push(
                    Part::shape(glossy([wide_, 92.0], 46.0, accent, 90.0))
                        .at(0.0, 350.0)
                        .on(0)
                        .enter(AnimKind::Pop)
                        .after(0.9)
                        .looped(LoopKind::Heartbeat)
                        .blurred(),
                );
                v.push(Part::text(words, 46.0, over(accent)).on(at).after(0.9).enter(AnimKind::Fade).blurred());
            }
            v
        }
        "flipCard" => {
            // two faces on one spot: the front turns away edge on, and the back turns in from there
            let (w, h) = (680.0, 300.0);
            let half = 450;
            let face = |fill: [f32; 4]| Shape { shadow: shadow(), ..plate([w, h], 36.0, fill) };
            let hold = |a: f32, b: f32, c: f32| {
                vec![(0, a, Ease::Linear), (half - 80, a, Ease::IN), (half, b, Ease::OUT), (half + 80, c, Ease::Linear)]
            };
            let seen = |a: f32, b: f32| vec![(0, a, Ease::Hold), (half, b, Ease::Hold)];
            vec![
                Part::shape(face(WHITE))
                    .enter(AnimKind::Pop)
                    .keyed(Prop::RotationY, hold(0.0, 90.0, 90.0))
                    .keyed(Prop::Opacity, seen(1.0, 0.0))
                    .blurred(),
                // what a face carries turns with it but does not fade with it
                Part::text(text(), fit(&ask.text, 96.0, w - 80.0), GREY)
                    .on(0)
                    .enter(AnimKind::Fade)
                    .over(0.3)
                    .keyed(Prop::Opacity, seen(1.0, 0.0))
                    .blurred(),
                Part::shape(Shape {
                    fill: Some(Fill::Linear { from: lighter(accent, 0.25), to: accent, angle: 90.0 }),
                    ..face(accent)
                })
                .keyed(Prop::RotationY, hold(-90.0, -90.0, 0.0))
                .keyed(Prop::Opacity, seen(0.0, 1.0))
                .blurred(),
                Part::text(line(0), fit(&line(0), 104.0, w - 80.0), over(accent))
                    .on(2)
                    .keyed(Prop::Opacity, seen(0.0, 1.0))
                    .blurred(),
            ]
        }
        "posterWords" => {
            // a line a word, each as large as the block lets it be
            let words: Vec<&str> = ask.text.split_whitespace().collect();
            let rows = words.len().max(1) as f32;
            let size = (760.0 / rows).min(300.0);
            let step = size * 1.08;
            let tall = step * rows + 110.0;
            let mut v = vec![
                Part::shape(Shape { shadow: shadow(), ..plate([900.0, tall], 30.0, accent) })
                    .turned(-3.0)
                    .enter(AnimKind::Pop)
                    .over(0.4),
            ];
            for (i, word) in words.iter().enumerate() {
                let colour = if i % 2 == 1 { second } else { over(accent) };
                v.push(
                    Part::text(*word, fit(word, size, 800.0), colour)
                        .at(0.0, row(i, words.len(), step))
                        .turned(-3.0)
                        .enter(AnimKind::WordPop)
                        .over(0.5)
                        .after(0.15 + i as f64 * 0.22)
                        .blurred(),
                );
            }
            v
        }
        "karaokeCaption" => {
            let size = fit(&ask.text, 68.0, 900.0);
            let wide_ = (wide(&ask.text, size) + 90.0).min(1000.0);
            vec![
                Part::shape(plate([wide_, size * 1.9], size * 0.5, DARK)).enter(AnimKind::Fade).over(0.3),
                Part::text(text(), size, WHITE).enter(AnimKind::Fade).over(0.3).styled(|s| {
                    s.weight = 700;
                    s.karaoke = Some(Karaoke {
                        color: lighter(accent, 0.25),
                        mode: KaraokeMode::Sweep,
                        start: secs(0.3).min(Tick(dur.0 / 4)),
                        duration: Some(Tick(dur.0 * 7 / 10)),
                    });
                }),
            ]
        }
        "elasticTitle" => {
            let size = fit(&ask.text, 150.0, 940.0);
            let under = (wide(&ask.text, size) * 0.8).clamp(160.0, 900.0);
            let mut v = vec![
                Part::text(text(), size, WHITE).enter(AnimKind::CharElastic).over(0.9),
                Part::shape(bar(under, 14.0, accent)).at(0.0, size * 0.72).enter(AnimKind::Draw).over(0.5).after(0.5),
            ];
            if !line(0).is_empty() {
                v.push(
                    Part::text(line(0), fit(&line(0), 52.0, 900.0), WHITE)
                        .at(0.0, size * 0.72 + 80.0)
                        .enter(AnimKind::WordRise)
                        .after(0.7)
                        .styled(|s| s.weight = 500),
                );
            }
            v
        }
        "handwritten" => {
            let size = fit(&ask.text, 170.0, 940.0);
            let under = (wide(&ask.text, size) * 0.9).clamp(200.0, 940.0);
            vec![
                Part::text(text(), size, WHITE).enter(AnimKind::Draw).over(1.6).styled(|s| {
                    s.weight = 500;
                    s.italic = true;
                }),
                // a quick stroke under it, a little askew
                Part::shape(bar(under, 12.0, accent))
                    .at(0.0, size * 0.74)
                    .turned(-2.0)
                    .enter(AnimKind::Draw)
                    .over(0.45)
                    .after(1.5),
            ]
        }
        _ => return None,
    })
}

/// Sets a title to wrap in a box `width` of the piece's pixels wide, on a canvas `canvas` of them
/// wide, from the left if `left`.
pub(crate) fn boxed(style: &mut TitleStyle, width: f32, canvas: f32, left: bool) {
    if left {
        style.align = TitleAlign::Left;
    }
    style.wrap = (width / canvas).clamp(0.01, 1.0);
}

impl Promo {
    /// How many clips the piece is made of, as it comes.
    pub fn clips(&self) -> usize {
        let ask = Ask {
            text: self.text.to_string(),
            lines: self.lines.iter().map(|l| l.to_string()).collect(),
            from: self.from,
            value: self.value,
            values: self.values.to_vec(),
            accent: RED,
            second: YELLOW,
            side: None,
            dur: Tick::from_seconds(self.millis as f64 / 1000.0),
            canvas: [1080.0, 1920.0],
            place: self.place,
            scale: 1.0,
            logo: None,
            media: None,
        };
        parts(self.id, &ask).map_or(0, |p| p.len())
    }
}
