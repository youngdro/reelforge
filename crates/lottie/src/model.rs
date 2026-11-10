//! A Lottie file as it is read: compositions of layers, their transforms, masks and contents.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::geom::{Affine, Cap, Join, P};
use crate::raster::{Bitmap, Rule};
use crate::value::{Numbers, Path, number};

/// What is being read, and what has been met that is not drawn.
pub(crate) struct Reading {
    /// The Bodymovin version that wrote the file.
    version: [u32; 3],
    pub notes: BTreeSet<&'static str>,
}

impl Reading {
    pub fn new(root: &Value) -> Reading {
        let mut version = [5, 0, 0];
        if let Some(v) = root.get("v").and_then(Value::as_str) {
            for (k, part) in v.split('.').take(3).enumerate() {
                version[k] = part.trim().parse().unwrap_or(0);
            }
        }
        Reading { version, notes: BTreeSet::new() }
    }

    fn before(&self, version: [u32; 3]) -> bool {
        self.version < version
    }

    fn note(&mut self, what: &'static str) {
        self.notes.insert(what);
    }

    /// A value, with a note when it is driven by an expression (which is not run: the keys or
    /// the fixed value beside it are used).
    fn numbers(&mut self, v: Option<&Value>, or: &[f32]) -> Numbers {
        if v.and_then(|v| v.get("x")).is_some_and(Value::is_string) {
            self.note("expressions");
        }
        Numbers::read(v).unwrap_or_else(|| Numbers::fixed(or))
    }

    fn colour(&mut self, v: Option<&Value>) -> Numbers {
        let c = self.numbers(v, &[0.0, 0.0, 0.0, 1.0]);
        // colours were bytes once
        if self.before([4, 1, 9]) { c.scaled(1.0 / 255.0) } else { c }
    }
}

fn flag(v: Option<&Value>) -> bool {
    v.is_some_and(|v| v.as_bool().unwrap_or_else(|| number(v).is_some_and(|n| n != 0.0)))
}

/// Where something is: anchor, position, scale, turn, opacity and skew.
#[derive(Clone, Debug)]
pub(crate) struct Transform {
    pub anchor: Numbers,
    pub position: Numbers,
    /// A position whose x and y are keyed apart.
    pub split: Option<(Numbers, Numbers)>,
    pub scale: Numbers,
    pub rotation: Numbers,
    pub opacity: Numbers,
    pub skew: Numbers,
    pub skew_axis: Numbers,
}

impl Transform {
    fn read(r: &mut Reading, v: Option<&Value>) -> Transform {
        let get = |k: &str| v.and_then(|v| v.get(k));
        let p = get("p");
        let split =
            p.filter(|p| flag(p.get("s"))).map(|p| (r.numbers(p.get("x"), &[0.0]), r.numbers(p.get("y"), &[0.0])));
        Transform {
            anchor: r.numbers(get("a"), &[0.0, 0.0]),
            position: r.numbers(p.filter(|_| split.is_none()), &[0.0, 0.0]),
            split,
            scale: r.numbers(get("s"), &[100.0, 100.0]),
            // a layer in space keeps its turn in the plane under `rz`
            rotation: r.numbers(get("r").or(get("rz")), &[0.0]),
            opacity: r.numbers(get("o"), &[100.0]),
            skew: r.numbers(get("sk"), &[0.0]),
            skew_axis: r.numbers(get("sa"), &[0.0]),
        }
    }

    pub fn position(&self, frame: f32) -> P {
        match &self.split {
            Some((x, y)) => [x.one(frame), y.one(frame)],
            None => self.position.two(frame),
        }
    }

    /// The map at `frame`.
    pub fn matrix(&self, frame: f32) -> Affine {
        let (a, p, s) = (self.anchor.two(frame), self.position(frame), self.scale.two(frame));
        let (sk, sa) = (self.skew.one(frame), self.skew_axis.one(frame));
        let mut m = Affine::translate(p[0], p[1]).then_after(Affine::rotate(self.rotation.one(frame)));
        if sk != 0.0 {
            m = m
                .then_after(Affine::rotate(-sa))
                .then_after(Affine::shear((-sk).to_radians().tan()))
                .then_after(Affine::rotate(sa));
        }
        m.then_after(Affine::scale(s[0] / 100.0, s[1] / 100.0)).then_after(Affine::translate(-a[0], -a[1]))
    }

    /// Opacity from 0 to 1.
    pub fn opacity(&self, frame: f32) -> f32 {
        (self.opacity.one(frame) / 100.0).clamp(0.0, 1.0)
    }
}

/// How a mask joins the ones before it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum MaskMode {
    Add,
    Subtract,
    Intersect,
}

/// A mask on a layer.
#[derive(Clone, Debug)]
pub(crate) struct Mask {
    pub mode: MaskMode,
    pub inverted: bool,
    pub path: Path,
    pub opacity: Numbers,
}

/// A colour gradient.
#[derive(Clone, Debug)]
pub(crate) struct Gradient {
    pub radial: bool,
    pub start: Numbers,
    pub end: Numbers,
    /// How many colour stops `stops` begins with; opacity stops follow.
    pub count: usize,
    pub stops: Numbers,
}

impl Gradient {
    fn read(r: &mut Reading, v: &Value) -> Gradient {
        let g = v.get("g");
        if v.get("h").is_some_and(|h| h.get("k").and_then(number).is_some_and(|k| k != 0.0) || flag(h.get("a"))) {
            r.note("the highlight of a radial gradient");
        }
        Gradient {
            radial: v.get("t").and_then(number) == Some(2.0),
            start: r.numbers(v.get("s"), &[0.0, 0.0]),
            end: r.numbers(v.get("e"), &[100.0, 0.0]),
            count: g.and_then(|g| g.get("p")).and_then(number).unwrap_or(0.0) as usize,
            stops: r.numbers(g.and_then(|g| g.get("k")), &[]),
        }
    }

    /// The stops at `frame`: offsets and straight colours, opacity stops worked in.
    pub fn stops(&self, frame: f32) -> Vec<(f32, [f32; 4])> {
        let v = self.stops.at(frame);
        let n = self.count.min(v.len() / 4);
        let colours: Vec<(f32, [f32; 3])> =
            (0..n).map(|k| (v[k * 4], [v[k * 4 + 1], v[k * 4 + 2], v[k * 4 + 3]])).collect();
        let alphas: Vec<(f32, f32)> = v[n * 4..].as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect();
        if colours.is_empty() {
            return Vec::new();
        }
        let colour_at = |t: f32| -> [f32; 3] {
            let mut last = colours[0];
            for c in &colours {
                if t <= c.0 {
                    let k = if c.0 > last.0 { (t - last.0) / (c.0 - last.0) } else { 1.0 };
                    return std::array::from_fn(|d| last.1[d] + (c.1[d] - last.1[d]) * k);
                }
                last = *c;
            }
            last.1
        };
        let alpha_at = |t: f32| -> f32 {
            let Some(mut last) = alphas.first().copied() else { return 1.0 };
            for a in &alphas {
                if t <= a.0 {
                    let k = if a.0 > last.0 { (t - last.0) / (a.0 - last.0) } else { 1.0 };
                    return last.1 + (a.1 - last.1) * k;
                }
                last = *a;
            }
            last.1
        };
        let mut at: Vec<f32> = colours.iter().map(|c| c.0).chain(alphas.iter().map(|a| a.0)).collect();
        at.sort_by(f32::total_cmp);
        at.dedup();
        at.into_iter()
            .map(|t| {
                let c = colour_at(t);
                (t, [c[0], c[1], c[2], alpha_at(t)])
            })
            .collect()
    }
}

/// What a fill or a stroke is painted with.
#[derive(Clone, Debug)]
pub(crate) enum Colour {
    Solid(Numbers),
    Gradient(Gradient),
}

/// How an outline is drawn.
#[derive(Clone, Debug)]
pub(crate) struct Pen {
    pub width: Numbers,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f32,
    /// Dash and gap lengths in turn, and how far in the pattern starts.
    pub dashes: Vec<Numbers>,
    pub dash_offset: Option<Numbers>,
}

impl Pen {
    fn read(r: &mut Reading, v: &Value) -> Pen {
        let mut dashes = Vec::new();
        let mut dash_offset = None;
        for d in v.get("d").and_then(Value::as_array).into_iter().flatten() {
            let value = r.numbers(d.get("v"), &[0.0]);
            match d.get("n").and_then(Value::as_str) {
                Some("o") => dash_offset = Some(value),
                _ => dashes.push(value),
            }
        }
        Pen {
            width: r.numbers(v.get("w"), &[1.0]),
            cap: match v.get("lc").and_then(number).unwrap_or(2.0) as u8 {
                1 => Cap::Butt,
                3 => Cap::Square,
                _ => Cap::Round,
            },
            join: match v.get("lj").and_then(number).unwrap_or(2.0) as u8 {
                1 => Join::Miter,
                3 => Join::Bevel,
                _ => Join::Round,
            },
            miter_limit: v.get("ml").and_then(number).unwrap_or(4.0),
            dashes,
            dash_offset,
        }
    }
}

/// A star or a regular polygon.
#[derive(Clone, Debug)]
pub(crate) struct Star {
    pub star: bool,
    pub points: Numbers,
    pub position: Numbers,
    pub rotation: Numbers,
    pub outer: Numbers,
    pub outer_round: Numbers,
    pub inner: Numbers,
    pub inner_round: Numbers,
    pub reversed: bool,
}

/// Copies of what is above a repeater.
#[derive(Clone, Debug)]
pub(crate) struct Repeater {
    pub copies: Numbers,
    pub offset: Numbers,
    /// Later copies go under earlier ones.
    pub below: bool,
    pub anchor: Numbers,
    pub position: Numbers,
    pub scale: Numbers,
    pub rotation: Numbers,
    pub start_opacity: Numbers,
    pub end_opacity: Numbers,
}

/// One thing in a shape layer's contents.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    Group(Vec<Item>),
    Path(Path),
    Rect {
        position: Numbers,
        size: Numbers,
        round: Numbers,
        reversed: bool,
    },
    Ellipse {
        position: Numbers,
        size: Numbers,
        reversed: bool,
    },
    Star(Box<Star>),
    Fill {
        colour: Colour,
        opacity: Numbers,
        rule: Rule,
    },
    Stroke {
        colour: Colour,
        opacity: Numbers,
        pen: Pen,
    },
    Transform(Box<Transform>),
    Trim {
        start: Numbers,
        end: Numbers,
        offset: Numbers,
        together: bool,
    },
    Round(Numbers),
    Repeater(Box<Repeater>),
    /// Something not drawn.
    Other,
}

fn items(r: &mut Reading, v: Option<&Value>) -> Vec<Item> {
    v.and_then(Value::as_array).into_iter().flatten().map(|it| item(r, it)).collect()
}

fn item(r: &mut Reading, v: &Value) -> Item {
    if flag(v.get("hd")) {
        return Item::Other;
    }
    let reversed = v.get("d").and_then(number) == Some(3.0);
    let rule = if v.get("r").and_then(number) == Some(2.0) { Rule::EvenOdd } else { Rule::NonZero };
    match v.get("ty").and_then(Value::as_str).unwrap_or("") {
        "gr" => Item::Group(items(r, v.get("it"))),
        "sh" => {
            // the oldest files say whether a path is closed beside it
            let closed = r.before([4, 4, 18]).then(|| v.get("closed").map(|c| flag(Some(c)))).flatten();
            Path::read(v.get("ks"), closed).map_or(Item::Other, Item::Path)
        }
        "rc" => Item::Rect {
            position: r.numbers(v.get("p"), &[0.0, 0.0]),
            size: r.numbers(v.get("s"), &[0.0, 0.0]),
            round: r.numbers(v.get("r"), &[0.0]),
            reversed,
        },
        "el" => Item::Ellipse {
            position: r.numbers(v.get("p"), &[0.0, 0.0]),
            size: r.numbers(v.get("s"), &[0.0, 0.0]),
            reversed,
        },
        "sr" => Item::Star(Box::new(Star {
            star: v.get("sy").and_then(number) != Some(2.0),
            points: r.numbers(v.get("pt"), &[5.0]),
            position: r.numbers(v.get("p"), &[0.0, 0.0]),
            rotation: r.numbers(v.get("r"), &[0.0]),
            outer: r.numbers(v.get("or"), &[0.0]),
            outer_round: r.numbers(v.get("os"), &[0.0]),
            inner: r.numbers(v.get("ir"), &[0.0]),
            inner_round: r.numbers(v.get("is"), &[0.0]),
            reversed,
        })),
        "fl" => {
            Item::Fill { colour: Colour::Solid(r.colour(v.get("c"))), opacity: r.numbers(v.get("o"), &[100.0]), rule }
        }
        "gf" => Item::Fill {
            colour: Colour::Gradient(Gradient::read(r, v)),
            opacity: r.numbers(v.get("o"), &[100.0]),
            rule,
        },
        "st" => Item::Stroke {
            colour: Colour::Solid(r.colour(v.get("c"))),
            opacity: r.numbers(v.get("o"), &[100.0]),
            pen: Pen::read(r, v),
        },
        "gs" => Item::Stroke {
            colour: Colour::Gradient(Gradient::read(r, v)),
            opacity: r.numbers(v.get("o"), &[100.0]),
            pen: Pen::read(r, v),
        },
        "tr" => Item::Transform(Box::new(Transform::read(r, Some(v)))),
        "tm" => Item::Trim {
            start: r.numbers(v.get("s"), &[0.0]),
            end: r.numbers(v.get("e"), &[100.0]),
            offset: r.numbers(v.get("o"), &[0.0]),
            together: v.get("m").and_then(number) == Some(2.0),
        },
        "rd" => Item::Round(r.numbers(v.get("r"), &[0.0])),
        "rp" => {
            let tr = v.get("tr");
            let get = |k: &str| tr.and_then(|t| t.get(k));
            Item::Repeater(Box::new(Repeater {
                copies: r.numbers(v.get("c"), &[1.0]),
                offset: r.numbers(v.get("o"), &[0.0]),
                below: v.get("m").and_then(number) == Some(2.0),
                anchor: r.numbers(get("a"), &[0.0, 0.0]),
                position: r.numbers(get("p"), &[0.0, 0.0]),
                scale: r.numbers(get("s"), &[100.0, 100.0]),
                rotation: r.numbers(get("r"), &[0.0]),
                start_opacity: r.numbers(get("so"), &[100.0]),
                end_opacity: r.numbers(get("eo"), &[100.0]),
            }))
        }
        "mm" => {
            // paths under one fill are one path anyway; the other ways of merging are not done
            if v.get("mm").and_then(number).is_some_and(|m| m > 1.0) {
                r.note("merge paths other than plain merging");
            }
            Item::Other
        }
        "op" => {
            r.note("offset paths");
            Item::Other
        }
        "zz" | "pb" | "tw" => {
            r.note("zig zag, pucker and twist");
            Item::Other
        }
        _ => Item::Other,
    }
}

/// A line of set text, drawn from the glyph outlines the file carries.
#[derive(Clone, Debug)]
pub(crate) struct Text {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub line_height: f32,
    pub tracking: f32,
    /// 0 left, 1 right, 2 centre.
    pub justify: u8,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<([f32; 4], f32)>,
    /// The outline is drawn over the fill (as players do when the file does not say), not under.
    pub stroke_over: bool,
}

/// What a layer shows.
#[derive(Clone, Debug)]
pub(crate) enum Content {
    /// Another composition, by its id, in a window of this size.
    Comp {
        id: String,
        size: P,
    },
    Solid {
        size: P,
        colour: [f32; 4],
    },
    Image(String),
    Shapes(Vec<Item>),
    Text(Box<Text>),
    /// Nothing: a null, or a kind of layer that is not drawn.
    Nothing,
}

/// How a layer is shown through the layer above it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Matte {
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

/// A layer.
#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub index: Option<i64>,
    pub parent: Option<i64>,
    /// The frames it is there from and until.
    pub from: f32,
    pub until: f32,
    /// A composition layer's own frame 0, and how much slower than its composition it plays.
    pub start: f32,
    pub stretch: f32,
    /// A composition layer's time, in seconds of its composition, when it is remapped.
    pub remap: Option<Numbers>,
    pub transform: Transform,
    pub auto_orient: bool,
    pub masks: Vec<Mask>,
    /// It is shown through another layer.
    pub matte: Option<Matte>,
    /// The layer it is shown through, when the file names one.
    pub matte_layer: Option<i64>,
    /// It is only there for another layer to be shown through.
    pub is_matte: bool,
    pub content: Content,
}

fn hex(s: &str) -> Option<[f32; 4]> {
    let s = s.trim_start_matches('#');
    let part = |k: usize| u8::from_str_radix(s.get(k * 2..k * 2 + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    Some([part(0)?, part(1)?, part(2)?, 1.0])
}

fn text(r: &mut Reading, v: Option<&Value>) -> Option<Text> {
    let t = v?;
    if t.get("a").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        r.note("text animators");
    }
    let d = t.get("d")?;
    // the document is keyed (held), or bare in the oldest files
    let keys = d.get("k").and_then(Value::as_array);
    if keys.is_some_and(|k| k.len() > 1) {
        r.note("text that changes over time");
    }
    let doc = keys.and_then(|k| k.first()).map_or(d, |k| k.get("s").unwrap_or(k));
    let colour = |k: &str| {
        let c: Vec<f32> = doc.get(k)?.as_array()?.iter().filter_map(number).collect();
        (c.len() >= 3).then(|| [c[0], c[1], c[2], 1.0])
    };
    if doc.get("sz").is_some() {
        r.note("text wrapped in a box");
    }
    let size = doc.get("s").and_then(number).unwrap_or(0.0);
    Some(Text {
        text: doc.get("t").and_then(Value::as_str)?.to_string(),
        font: doc.get("f").and_then(Value::as_str).unwrap_or("").to_string(),
        size,
        line_height: doc.get("lh").and_then(number).unwrap_or(size * 1.2),
        tracking: doc.get("tr").and_then(number).unwrap_or(0.0),
        justify: doc.get("j").and_then(number).unwrap_or(0.0) as u8,
        fill: colour("fc"),
        stroke: colour("sc").map(|c| (c, doc.get("sw").and_then(number).unwrap_or(0.0))),
        stroke_over: doc.get("of").is_none_or(|v| flag(Some(v))),
    })
}

fn layer(r: &mut Reading, v: &Value) -> Option<Layer> {
    let num = |k: &str| v.get(k).and_then(number);
    let ty = num("ty")? as i64;
    if flag(v.get("ddd")) {
        r.note("layers turned in space (3D)");
    }
    if v.get("ef").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) {
        r.note("effects");
    }
    if v.get("sy").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) {
        r.note("layer styles");
    }
    if num("bm").is_some_and(|b| b != 0.0) {
        r.note("blend modes inside the animation");
    }
    let mut masks = Vec::new();
    for m in v.get("masksProperties").and_then(Value::as_array).into_iter().flatten() {
        let mode = match m.get("mode").and_then(Value::as_str).unwrap_or("a") {
            "a" => MaskMode::Add,
            "s" => MaskMode::Subtract,
            "i" => MaskMode::Intersect,
            "n" => continue,
            _ => {
                r.note("lighten, darken and difference masks");
                MaskMode::Add
            }
        };
        if m.get("x").is_some_and(|x| x.get("k").and_then(number).is_some_and(|k| k != 0.0) || flag(x.get("a"))) {
            r.note("mask expansion");
        }
        let closed = r.before([4, 4, 18]).then(|| m.get("cl").map(|c| flag(Some(c)))).flatten();
        if let Some(path) = Path::read(m.get("pt"), closed) {
            masks.push(Mask { mode, inverted: flag(m.get("inv")), path, opacity: r.numbers(m.get("o"), &[100.0]) });
        }
    }
    // a hidden layer may still be a parent, so it stays, showing nothing
    let hidden = flag(v.get("hd"));
    let content = match ty {
        _ if hidden => Content::Nothing,
        0 => Content::Comp {
            id: v.get("refId").and_then(Value::as_str)?.to_string(),
            size: [num("w").unwrap_or(0.0), num("h").unwrap_or(0.0)],
        },
        1 => Content::Solid {
            size: [num("sw").unwrap_or(0.0), num("sh").unwrap_or(0.0)],
            colour: v.get("sc").and_then(Value::as_str).and_then(hex).unwrap_or([0.0, 0.0, 0.0, 1.0]),
        },
        2 => Content::Image(v.get("refId").and_then(Value::as_str)?.to_string()),
        4 => Content::Shapes(items(r, v.get("shapes"))),
        5 => text(r, v.get("t")).map_or(Content::Nothing, |t| Content::Text(Box::new(t))),
        3 => Content::Nothing,
        _ => {
            r.note("audio, video and camera layers");
            Content::Nothing
        }
    };
    Some(Layer {
        index: num("ind").map(|n| n as i64),
        parent: num("parent").map(|n| n as i64),
        from: num("ip").unwrap_or(f32::MIN),
        until: num("op").unwrap_or(f32::MAX),
        start: num("st").unwrap_or(0.0),
        stretch: num("sr").filter(|s| *s > 0.0).unwrap_or(1.0),
        remap: v.get("tm").map(|tm| r.numbers(Some(tm), &[0.0])),
        transform: Transform::read(r, v.get("ks")),
        auto_orient: flag(v.get("ao")),
        masks,
        matte: match num("tt").unwrap_or(0.0) as i64 {
            1 => Some(Matte::Alpha),
            2 => Some(Matte::AlphaInverted),
            3 => Some(Matte::Luma),
            4 => Some(Matte::LumaInverted),
            _ => None,
        },
        matte_layer: num("tp").map(|n| n as i64),
        is_matte: flag(v.get("td")),
        content,
    })
}

pub(crate) fn layers(r: &mut Reading, v: Option<&Value>) -> Vec<Layer> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(|l| layer(r, l)).collect()
}

/// A glyph the file carries: its outline at size 100 and how far the next one is.
#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub advance: f32,
    pub outline: Vec<Item>,
}

/// The glyphs by character and font family.
pub(crate) fn glyphs(r: &mut Reading, root: &Value) -> HashMap<(String, String), Glyph> {
    let mut out = HashMap::new();
    for c in root.get("chars").and_then(Value::as_array).into_iter().flatten() {
        let (Some(ch), Some(family)) = (c.get("ch").and_then(Value::as_str), c.get("fFamily").and_then(Value::as_str))
        else {
            continue;
        };
        let shapes = c.get("data").and_then(|d| d.get("shapes"));
        out.insert(
            (ch.to_string(), family.to_string()),
            Glyph { advance: c.get("w").and_then(number).unwrap_or(0.0), outline: items(r, shapes) },
        );
    }
    out
}

/// The family of each font the file names.
pub(crate) fn families(root: &Value) -> HashMap<String, String> {
    let list = root.get("fonts").and_then(|f| f.get("list")).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|f| Some((f.get("fName")?.as_str()?.to_string(), f.get("fFamily")?.as_str()?.to_string())))
        .collect()
}

/// A picture the file uses: its size as the animation lays it out, and the picture when it was
/// found.
pub(crate) struct Picture {
    pub size: P,
    pub bitmap: Option<Bitmap>,
}
