//! Storyboards: whole videos planned ahead for a kind of job (a launch, a flash sale, a shop to
//! visit), to be filled with the person's own pictures and words.
//!
//! A storyboard is scenes in order. Each has a length, a picture (a template slot, with a line on
//! what to film for it, or a backdrop in the brand's colour), ready-made pieces over it, and the
//! line to say. Its words are *fields* with names (`hook`, `product`, `price`): the pieces are
//! written from them, and the project keeps them ([`reelforge_model::Project::copy`]), so giving
//! a field other words (`storyboard.fill`) makes every piece written from it again.
//!
//! The storyboards are data (`crates/engine/storyboards/*.json`), built here by the commands
//! anyone could run, in one undo step.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use reelforge_model::{ClipContent, CopyField, CopyLine, Forms, Project, TrackKind, fields_in, is_field_name, written};
use reelforge_time::Tick;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::commands::{fps, one_step};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// A language the storyboards are written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lang {
    /// English.
    En,
    /// Chinese.
    Zh,
    /// Japanese.
    Ja,
}

impl Lang {
    /// The names commands use.
    pub const NAMES: [&'static str; 3] = ["en", "zh", "ja"];

    /// The one a command names.
    pub fn parse(name: &str) -> Option<Lang> {
        Some(match name {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            "ja" => Lang::Ja,
            _ => return None,
        })
    }
}

/// Something said in each language.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Said {
    en: String,
    zh: String,
    ja: String,
}

impl Said {
    /// It, in `lang`.
    pub fn get(&self, lang: Lang) -> &str {
        match lang {
            Lang::En => &self.en,
            Lang::Zh => &self.zh,
            Lang::Ja => &self.ja,
        }
    }
}

/// One of a storyboard's fields: words, or a number.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// Its name.
    pub id: String,
    text: Option<Said>,
    number: Option<f64>,
}

impl Field {
    /// What it says until it is told: its words in `lang`, or its number as it is written.
    pub fn default(&self, lang: Lang) -> String {
        match (&self.text, self.number) {
            (Some(text), _) => text.get(lang).to_string(),
            (None, n) => plain(n.unwrap_or(0.0)),
        }
    }

    /// Whether it is a number.
    pub fn is_number(&self) -> bool {
        self.number.is_some()
    }
}

/// A number as copy has it: no more digits than it needs.
fn plain(n: f64) -> String {
    let rounded = (n * 100.0).round() / 100.0;
    format!("{rounded}")
}

/// A ready-made piece of a scene: `promo.add`'s parameters, where a string may be said in each
/// language and may name fields between braces.
#[derive(Clone, Debug, Deserialize)]
struct Piece {
    kind: String,
    /// Seconds into the scene.
    #[serde(default)]
    at: f64,
    #[serde(flatten)]
    params: Map<String, Value>,
}

/// One scene.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// What part of the video it is (its name is in the labels).
    pub id: String,
    /// How long it lasts.
    pub seconds: f64,
    /// What to film for it; a scene without is a backdrop.
    pub shot: Option<Said>,
    backdrop: Option<String>,
    /// A picture effect on its picture.
    fx: Option<String>,
    /// The transition into it.
    cut: Option<String>,
    /// A colour sweep over the cut into it, and the way it goes.
    sweep: Option<String>,
    /// The line to say.
    pub voice: Option<Said>,
    pieces: Vec<Piece>,
}

/// A storyboard.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storyboard {
    /// Stable id, as commands take it.
    pub id: String,
    /// Its name.
    pub name: Said,
    /// What it is for.
    pub about: Said,
    /// The library music under it.
    pub music: String,
    /// Its words, in the order a form asks for them.
    pub fields: Vec<Field>,
    /// Its scenes, in order.
    pub scenes: Vec<Scene>,
}

impl Storyboard {
    /// How long it runs, in seconds.
    pub fn seconds(&self) -> f64 {
        self.scenes.iter().map(|s| s.seconds).sum()
    }

    /// How many pictures it takes.
    pub fn slots(&self) -> usize {
        self.scenes.iter().filter(|s| s.shot.is_some()).count()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Labels {
    fields: BTreeMap<String, Said>,
    scenes: BTreeMap<String, Said>,
}

fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| serde_json::from_str(include_str!("../storyboards/labels.json")).expect("labels.json"))
}

/// What a field of the copy is called in `lang`; its own name where no storyboard knows it.
pub fn field_label(name: &str, lang: Lang) -> &str {
    labels().fields.get(name).map_or(name, |l| l.get(lang))
}

/// What a scene is called in `lang`.
pub fn scene_label(id: &str, lang: Lang) -> &str {
    labels().scenes.get(id).map_or(id, |l| l.get(lang))
}

/// The built-in storyboards, in the order a picker shows them.
pub fn storyboards() -> &'static [Storyboard] {
    static ALL: OnceLock<Vec<Storyboard>> = OnceLock::new();
    ALL.get_or_init(|| {
        [
            include_str!("../storyboards/launch.json"),
            include_str!("../storyboards/flashSale.json"),
            include_str!("../storyboards/bogo.json"),
            include_str!("../storyboards/beforeAfter.json"),
            include_str!("../storyboards/roundup.json"),
            include_str!("../storyboards/storeVisit.json"),
        ]
        .into_iter()
        .map(|text| serde_json::from_str(text).expect("a storyboard"))
        .collect()
    })
}

fn wrong(name: &str, what: &str) -> EngineError {
    EngineError::Params(format!("`{name}` must be {what}"))
}

fn lang_param(p: &P) -> Result<Lang, EngineError> {
    match p.opt_str("lang")? {
        None => Ok(Lang::Zh),
        Some(name) => Lang::parse(name).ok_or_else(|| wrong("lang", "en, zh or ja")),
    }
}

/// `storyboard.list`.
pub(crate) fn storyboard_list(s: &mut Session, v: &Value) -> CmdResult {
    let lang = lang_param(&P::new(v, fps(s)))?;
    let all: Vec<Value> = storyboards()
        .iter()
        .map(|b| {
            let fields: Vec<Value> = b
                .fields
                .iter()
                .map(|f| {
                    let value = if f.is_number() { json!(f.number) } else { json!(f.default(lang)) };
                    json!({"name": f.id, "label": field_label(&f.id, lang), "value": value})
                })
                .collect();
            let scenes: Vec<Value> = b
                .scenes
                .iter()
                .map(|sc| {
                    json!({
                        "id": sc.id,
                        "name": scene_label(&sc.id, lang),
                        "seconds": sc.seconds,
                        "shot": sc.shot.as_ref().map(|x| x.get(lang)),
                        "voice": sc.voice.as_ref().map(|x| x.get(lang)),
                        "pieces": sc.pieces.iter().map(|p| p.kind.as_str()).collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "id": b.id,
                "name": b.name.get(lang),
                "about": b.about.get(lang),
                "seconds": b.seconds(),
                "slots": b.slots(),
                "music": b.music,
                "fields": fields,
                "scenes": scenes,
            })
        })
        .collect();
    Ok(json!({ "storyboards": all }))
}

/// The copy a command gives: an object of words or numbers by field name.
fn copy_param(v: &Value) -> Result<Vec<CopyField>, EngineError> {
    let what = "an object of words or numbers by field name, such as {\"product\": \"保温杯\", \"price\": 129}";
    let Some(given) = v.get("copy") else { return Ok(vec![]) };
    let given = given.as_object().ok_or_else(|| wrong("copy", what))?;
    let mut out = vec![];
    for (name, value) in given {
        let value = match value {
            Value::String(text) if text.chars().count() <= 400 => text.clone(),
            Value::Number(n) => plain(n.as_f64().unwrap_or(0.0)),
            _ => return Err(wrong("copy", what)),
        };
        if !is_field_name(name) {
            return Err(EngineError::Params(format!(
                "`copy` names its fields with letters and digits, a letter first (not {name:?})"
            )));
        }
        out.push(CopyField { name: name.clone(), value });
    }
    Ok(out)
}

/// Puts `given` into the project's copy: over the fields it has, after them the new ones.
fn keep(proj: &mut Project, given: &[CopyField]) {
    for field in given {
        match proj.copy.iter_mut().find(|f| f.name == field.name) {
            Some(known) => known.value = field.value.clone(),
            None => proj.copy.push(field.clone()),
        }
    }
}

/// The number `form` comes to, written from `copy`.
fn number(form: &str, copy: &[CopyField]) -> Result<f32, EngineError> {
    let said = written(form, copy);
    said.trim().parse::<f32>().ok().filter(|n| n.is_finite() && n.abs() < 1e12).ok_or_else(|| {
        let fields: Vec<&str> = fields_in(form).collect();
        EngineError::Params(format!(
            "the field {} must be a number: a piece counts to it (it says {said:?})",
            fields.first().map_or_else(|| form.to_string(), |f| format!("`{f}`"))
        ))
    })
}

/// Writes what `made` is told from `copy`, by its forms.
fn rewrite(made: &mut reelforge_model::Made, copy: &[CopyField]) -> Result<(), EngineError> {
    let forms = made.forms.clone();
    if let Some(form) = &forms.text {
        made.text = written(form, copy);
    }
    if let Some(lines) = &forms.lines {
        made.lines = lines.iter().map(|form| written(form, copy)).collect();
    }
    if let Some(form) = &forms.value {
        made.value = Some(number(form, copy)?);
    }
    if let Some(values) = &forms.values {
        made.values = values.iter().map(|form| number(form, copy)).collect::<Result<_, _>>()?;
    }
    Ok(())
}

/// The project's copy as a command gives it back: each field with what it is called and how
/// many pieces are written from it.
fn copy_json(proj: &Project, lang: Lang) -> Value {
    let forms: Vec<&Forms> = proj.timelines.iter().filter_map(|t| t.made.as_ref()).map(|m| &m.forms).collect();
    let uses = |name: &str| forms.iter().filter(|f| f.all().any(|form| fields_in(form).any(|n| n == name))).count();
    json!(
        proj.copy
            .iter()
            .map(|f| json!({"name": f.name, "label": field_label(&f.name, lang), "value": f.value, "pieces": uses(&f.name)}))
            .collect::<Vec<_>>()
    )
}

/// `storyboard.copy`.
pub(crate) fn storyboard_copy(s: &mut Session, v: &Value) -> CmdResult {
    let lang = lang_param(&P::new(v, fps(s)))?;
    Ok(json!({ "copy": copy_json(&s.project, lang) }))
}

/// `storyboard.fill`.
pub(crate) fn storyboard_fill(s: &mut Session, v: &Value) -> CmdResult {
    let lang = lang_param(&P::new(v, fps(s)))?;
    let given = copy_param(v)?;
    if given.is_empty() {
        return Err(wrong("copy", "an object naming at least one field"));
    }
    s.edit("Fill Storyboard", |proj| {
        keep(proj, &given);
        // every piece written from a field that was given is made again
        let told = |form: &str| fields_in(form).any(|name| given.iter().any(|f| f.name == name));
        let stale: Vec<String> = proj
            .timelines
            .iter()
            .filter(|t| t.made.as_ref().is_some_and(|m| m.forms.all().any(told)))
            .map(|t| t.id.clone())
            .collect();
        let copy = proj.copy.clone();
        for timeline in &stale {
            let Some(mut made) = proj.timelines.iter().find(|t| t.id == *timeline).and_then(|t| t.made.clone()) else {
                continue;
            };
            rewrite(&mut made, &copy)?;
            crate::promo::remake(proj, timeline, made);
        }
        // and every caption written from one, unless it was changed by hand since or is gone
        let mut lines = std::mem::take(&mut proj.copy_lines);
        let mut captions = 0;
        lines.retain_mut(|line| {
            let Some(text) = caption_mut(proj, &line.clip).filter(|text| **text == line.text) else { return false };
            if told(&line.form) {
                line.text = written(&line.form, &copy);
                text.clone_from(&line.text);
                captions += 1;
            }
            true
        });
        proj.copy_lines = lines;
        Ok(json!({ "copy": copy_json(proj, lang), "pieces": stale.len(), "captions": captions }))
    })
}

/// The words of title clip `id`, on whichever timeline it is.
fn caption_mut<'a>(proj: &'a mut Project, id: &str) -> Option<&'a mut String> {
    let own = proj.tracks.iter_mut();
    let clips = own.chain(proj.timelines.iter_mut().flat_map(|t| &mut t.tracks)).flat_map(|t| &mut t.clips);
    match &mut clips.into_iter().find(|c| c.id == id)?.content {
        ClipContent::Text { text, .. } => Some(text),
        _ => None,
    }
}

/// A string of a storyboard's data: said in each language, or the same in all.
fn said(v: &Value, lang: Lang) -> Option<String> {
    match v {
        Value::String(text) => Some(text.clone()),
        Value::Object(_) => serde_json::from_value::<Said>(v.clone()).ok().map(|x| x.get(lang).to_string()),
        _ => None,
    }
}

/// What `promo.add` is told for `piece`, written from `copy`, and the forms it was written by.
fn told(piece: &Piece, lang: Lang, copy: &[CopyField]) -> Result<(Map<String, Value>, Forms), EngineError> {
    let bad = |key: &str| EngineError::Params(format!("a storyboard's {} has a bad `{key}`", piece.kind));
    let named = |form: &str| fields_in(form).next().is_some();
    let mut params = Map::new();
    let mut forms = Forms::default();
    for (key, value) in &piece.params {
        match key.as_str() {
            "text" => {
                let form = said(value, lang).ok_or_else(|| bad(key))?;
                params.insert(key.clone(), json!(written(&form, copy)));
                forms.text = named(&form).then_some(form);
            }
            "lines" => {
                let list = value.as_array().ok_or_else(|| bad(key))?;
                let list: Vec<String> =
                    list.iter().map(|x| said(x, lang).ok_or_else(|| bad(key))).collect::<Result<_, _>>()?;
                params.insert(key.clone(), json!(list.iter().map(|form| written(form, copy)).collect::<Vec<_>>()));
                forms.lines = list.iter().any(|form| named(form)).then_some(list);
            }
            "value" => match value {
                Value::String(form) => {
                    params.insert(key.clone(), json!(number(form, copy)?));
                    forms.value = named(form).then(|| form.clone());
                }
                other => {
                    params.insert(key.clone(), other.clone());
                }
            },
            "values" => {
                let list = value.as_array().ok_or_else(|| bad(key))?;
                let list: Vec<String> = list
                    .iter()
                    .map(|x| match x {
                        Value::String(form) => Ok(form.clone()),
                        Value::Number(n) => Ok(plain(n.as_f64().unwrap_or(0.0))),
                        _ => Err(bad(key)),
                    })
                    .collect::<Result<_, _>>()?;
                let numbers: Vec<f32> = list.iter().map(|form| number(form, copy)).collect::<Result<_, _>>()?;
                params.insert(key.clone(), json!(numbers));
                forms.values = list.iter().any(|form| named(form)).then_some(list);
            }
            _ => {
                params.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((params, forms))
}

/// The id a command gave back under `key`.
fn made(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

/// What `storyboard.apply` is asked for.
struct Ask {
    lang: Lang,
    copy: Vec<CopyField>,
    /// How much longer or shorter than written each scene is.
    stretch: f64,
    captions: bool,
}

/// Builds `board` on the timeline being edited, which is empty. Returns its scenes as they were
/// placed and its pieces.
fn build(s: &mut Session, board: &Storyboard, ask: &Ask) -> Result<(Vec<Value>, Vec<Value>), EngineError> {
    let lang = ask.lang;
    // the copy: what the storyboard comes with, under what was given
    let mut sheet: Vec<CopyField> =
        board.fields.iter().map(|f| CopyField { name: f.id.clone(), value: f.default(lang) }).collect();
    for given in &ask.copy {
        match sheet.iter_mut().find(|f| f.name == given.name) {
            Some(known) => known.value = given.value.clone(),
            None => sheet.push(given.clone()),
        }
    }
    s.edit("Copy", |proj| {
        keep(proj, &sheet);
        Ok(Value::Null)
    })?;
    let copy = s.project.copy.clone();

    let video = match s.project.tracks.iter().find(|t| t.kind == TrackKind::Video) {
        Some(track) => track.id.clone(),
        None => made(&s.execute("track.add", &json!({"kind": "video", "name": "Video 1"}))?, "track"),
    };
    let (cw, ch) = (s.project.canvas.width, s.project.canvas.height);
    // the scenes are laid out for an upright frame of 9:16; on a wider one their pieces keep to a
    // column down the middle, smaller, so that none meets the next (the layouts leave room for
    // them to be a third larger than the column's width alone would have them)
    let short = cw.min(ch) as f64;
    let column = (ch as f64 * 9.0 / 16.0).min(cw as f64);
    let shrink = (column / short * 1.3).min(1.0);
    let narrow = short * shrink / cw as f64;
    // a backdrop runs from a shade of the brand's colour into a darker one
    let primary = s.project.brand.primary.unwrap_or(crate::pieces::RED);
    let shade = |k: f32| [primary[0] * k, primary[1] * k, primary[2] * k, 1.0];

    let mut scenes = vec![];
    let mut pieces = vec![];
    let mut cues = vec![];
    // the lines to say: what each is written from, and what it says
    let mut forms: Vec<(String, String)> = vec![];
    let mut at = Tick::ZERO;
    let mut slot = 0u32;
    for (i, scene) in board.scenes.iter().enumerate() {
        let long = Tick::from_seconds(scene.seconds * ask.stretch);
        // its picture: a slot for the person's own, or a backdrop
        let mut params = json!({"track": video, "start": at, "duration": long});
        let filmed = scene.shot.is_some();
        if filmed {
            slot += 1;
            params["color"] = json!(crate::remix::slot_colour(slot));
        } else {
            let (from, to) = match scene.backdrop.as_deref() {
                Some("dark") => (shade(0.28), shade(0.1)),
                _ => (shade(0.62), shade(0.22)),
            };
            params["shape"] = json!({
                "kind": "rectangle", "size": [cw, ch],
                "fill": {"type": "linear", "from": from, "to": to, "angle": 90},
            });
        }
        if let (Some(fx), true) = (&scene.fx, filmed) {
            params["fx"] = json!([fx]);
        }
        let picture = made(&s.execute("clip.add", &params)?, "clip");
        if filmed {
            s.execute("template.mark", &json!({"clips": [picture], "slot": slot}))?;
        }
        if let (Some(kind), true) = (&scene.cut, i > 0) {
            s.execute("clip.setTransition", &json!({"clip": picture, "kind": kind, "duration": {"seconds": 0.4}}))?;
        }
        // a sweep of colour over the cut into it
        if let (Some(side), true) = (&scene.sweep, i > 0) {
            let lead = Tick::from_seconds(0.45).min(at);
            s.execute("promo.add", &json!({"kind": "colorSweep", "start": at - lead, "side": side, "compound": true}))?;
        }
        // its pieces, each folded into a clip and written from the copy
        for piece in &scene.pieces {
            let wait = Tick::from_seconds(piece.at).min(Tick(long.0 / 2));
            let (mut params, forms) = told(piece, lang, &copy)?;
            if shrink < 0.999 || narrow < 0.999 {
                let own = crate::pieces::PROMOS.iter().find(|p| p.id == piece.kind).map_or(0.0, |p| p.place[0]);
                let x = params.get("x").and_then(Value::as_f64).unwrap_or(own as f64);
                let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
                params.insert("x".into(), json!(x * narrow));
                params.insert("scale".into(), json!(scale * shrink));
            }
            params.insert("kind".into(), json!(piece.kind));
            params.insert("start".into(), json!(at + wait));
            params.insert("duration".into(), json!(long - wait));
            params.insert("compound".into(), json!(true));
            let placed = s.execute("promo.add", &Value::Object(params))?;
            let timeline = made(&placed, "timeline");
            if !forms.is_empty() {
                s.edit("Copy", |proj| {
                    if let Some(m) = proj.timelines.iter_mut().find(|t| t.id == timeline).and_then(|t| t.made.as_mut())
                    {
                        m.forms = forms.clone();
                    }
                    Ok(Value::Null)
                })?;
            }
            pieces.push(json!({"clip": placed["clip"], "kind": piece.kind, "scene": scene.id}));
        }
        let name = scene_label(&scene.id, lang);
        let shot = scene.shot.as_ref().map(|x| x.get(lang));
        let note = match shot {
            Some(shot) => format!("{name} · {shot}"),
            None => name.to_string(),
        };
        s.execute("marker.add", &json!({"at": at, "note": note}))?;
        let voice = scene.voice.as_ref().map(|x| written(x.get(lang), &copy));
        if let (Some(line), Some(form)) = (&voice, &scene.voice) {
            cues.push(json!({"start": at, "end": at + long, "text": line}));
            forms.push((form.get(lang).to_string(), line.clone()));
        }
        scenes.push(json!({
            "id": scene.id,
            "name": name,
            "start": at,
            "duration": long,
            "slot": filmed.then_some(slot),
            "shot": shot,
            "voice": voice,
        }));
        at += long;
    }

    // the music, cut to the pictures' length and kept under a voice
    let sound = made(&s.execute("sound.add", &json!({"sound": board.music, "at": 0}))?, "clip");
    if s.project.clip(&sound).is_some_and(|(_, c)| c.duration > at) {
        s.execute("clip.trim", &json!({"clip": sound, "end": at}))?;
    }
    s.execute("clip.set", &json!({"clip": sound, "fadeOut": {"seconds": 1.0}, "volumeDb": -8.0}))?;
    if ask.captions && !cues.is_empty() {
        let added = s.execute("captions.add", &json!({"cues": cues}))?;
        // they are written again when the copy is filled
        let clips: Vec<String> =
            added["clips"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
        s.edit("Copy", |proj| {
            for (clip, (form, text)) in clips.into_iter().zip(forms) {
                proj.copy_lines.push(CopyLine { clip, form, text });
            }
            Ok(Value::Null)
        })?;
    }
    Ok((scenes, pieces))
}

/// `storyboard.apply`.
pub(crate) fn storyboard_apply(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("storyboard")?;
    let board = storyboards().iter().find(|b| b.id == id).ok_or_else(|| {
        wrong(
            "storyboard",
            &format!("one of {}", storyboards().iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(", ")),
        )
    })?;
    let lang = lang_param(&p)?;
    let stretch = match p.opt_f64("seconds")? {
        None => 1.0,
        Some(wanted) => {
            let stretch = wanted / board.seconds();
            if !(0.5..=2.0).contains(&stretch) {
                let (least, most) = (board.seconds() * 0.5, board.seconds() * 2.0);
                return Err(wrong("seconds", &format!("{least} to {most} for this storyboard")));
            }
            stretch
        }
    };
    let ask = Ask { lang, copy: copy_param(v)?, stretch, captions: p.opt_bool("captions")?.unwrap_or(false) };
    let media = v.get("media").cloned();
    let name = p.opt_str("name")?.unwrap_or(board.name.get(lang)).to_string();
    one_step(s, "Use Storyboard", |s| {
        // on the timeline being edited when nothing is on it yet, else on one of its own
        if s.project.tracks.iter().any(|t| !t.clips.is_empty()) {
            s.execute("timeline.add", &json!({"name": name}))?;
        }
        let (scenes, pieces) = build(s, board, &ask)?;
        if let Some(media) = &media {
            s.execute("template.fill", &json!({"media": media, "from": 1}))?;
        }
        let mut out = crate::remix::slots_json(&s.project);
        out.as_object_mut().expect("an object").remove("texts");
        out["storyboard"] = json!(board.id);
        out["timeline"] = json!(s.project.timeline);
        out["duration"] = json!(s.project.duration());
        out["scenes"] = json!(scenes);
        out["pieces"] = json!(pieces);
        out["copy"] = copy_json(&s.project, lang);
        if lang == Lang::Ja && s.project.brand.font.is_none() {
            // the bundled face is cut down to Chinese and Latin: say so before a render shows gaps
            out["note"] = json!(
                "The built-in font lacks many Japanese characters. Import a Japanese font (media.import), make it the brand's (brand.set) and build the storyboard again: pieces take the brand's font when they are made."
            );
        }
        Ok(out)
    })
}
