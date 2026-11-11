//! JSON Schemas of command parameters, for clients that need them typed: MCP tool definitions,
//! generated CLI help, form builders. The parser in `params.rs` stays the authority; these
//! describe what it accepts (a test sends each schema's example through the real command).

use serde_json::{Value, json};

fn time(doc: &str) -> Value {
    json!({
        "description": format!("{doc} Ticks (254 016 000 000 per second), or {{\"seconds\": s}}, {{\"frame\": n}} or {{\"ticks\": n}}."),
        "oneOf": [
            {"type": "integer"},
            {"type": "object", "properties": {"seconds": {"type": "number"}}, "required": ["seconds"], "additionalProperties": false},
            {"type": "object", "properties": {"frame": {"type": "integer"}}, "required": ["frame"], "additionalProperties": false},
            {"type": "object", "properties": {"ticks": {"type": "integer"}}, "required": ["ticks"], "additionalProperties": false}
        ]
    })
}

fn color(doc: &str) -> Value {
    json!({
        "description": format!("{doc} [r, g, b] or [r, g, b, a], each 0 to 1, sRGB."),
        "type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 3, "maxItems": 4
    })
}

fn storyboard_lang() -> Value {
    json!({"enum": crate::storyboard::Lang::NAMES, "description": "The language of names and of the copy a storyboard comes with; zh by default."})
}

fn storyboard_copy(doc: &str) -> Value {
    json!({
        "type": "object",
        "description": format!("{doc} Such as {{\"product\": \"保温杯\", \"price\": 129}}."),
        "additionalProperties": {"type": ["string", "number"]},
    })
}

fn ident(doc: &str) -> Value {
    json!({"type": "string", "description": doc})
}

fn num(doc: &str) -> Value {
    json!({"type": "number", "description": doc})
}

fn nullable(mut v: Value) -> Value {
    // a value or null (null clears it)
    let doc = v.get("description").cloned();
    if let Some(o) = v.as_object_mut() {
        o.remove("description");
    }
    let mut out = json!({"oneOf": [v, {"type": "null"}]});
    if let Some(d) = doc {
        out["description"] = json!(format!("{} null clears it.", d.as_str().unwrap_or_default()));
    }
    out
}

/// Title look, shared by `clip.add` and `clip.set`.
fn title_props() -> Vec<(&'static str, Value)> {
    vec![
        ("text", json!({"type": "string", "description": "The words of a title clip."})),
        ("size", num("Title size in output pixels.")),
        ("color", color("Clip colour: a solid's fill or a title's text.")),
        (
            "weight",
            json!({"type": "integer", "minimum": 100, "maximum": 900, "description": "Title weight: 400 regular, 700 bold."}),
        ),
        ("align", json!({"enum": ["left", "center", "right"], "description": "Title alignment."})),
        (
            "wrap",
            json!({"type": "number", "minimum": 0, "maximum": 1, "description": "Wrap width as a share of the canvas width."}),
        ),
        (
            "outline",
            nullable(json!({
                "description": "Title outline.",
                "type": "object",
                "properties": {"color": color("Outline colour."), "width": num("Width in em (0.08 is a subtitle outline).")},
                "required": ["color", "width"]
            })),
        ),
        ("background", nullable(color("Box behind the title."))),
        ("radius", num("Corner radius of the box, in em.")),
        ("gradient", nullable(color("The fill fades from `color` at the top to this at the bottom."))),
        (
            "outer",
            nullable(json!({
                "description": "A second outline outside the first.",
                "type": "object",
                "properties": {"color": color("Outline colour."), "width": num("Width in em beyond the first outline.")},
            })),
        ),
        (
            "shadow",
            nullable(json!({
                "description": "A shadow beneath the title.",
                "type": "object",
                "properties": {
                    "color": color("Shadow colour."),
                    "offset": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "[right, down] in em."},
                    "blur": num("Softness in em; 0 is a hard edge.")
                },
            })),
        ),
        (
            "lineBoxes",
            json!({"type": "boolean", "description": "The box goes behind each line, hugging it, as subtitles have it."}),
        ),
        (
            "font",
            json!({"type": ["string", "null"], "description": "The face the title is set in: font media (media.import a .ttf, .otf or .ttc); null is the built-in face."}),
        ),
        (
            "tracking",
            json!({"type": "number", "minimum": -0.5, "maximum": 4, "description": "Space added after every character, in em."}),
        ),
        (
            "leading",
            json!({"type": "number", "minimum": -0.5, "maximum": 4, "description": "Space added between lines, in em."}),
        ),
        ("italic", json!({"type": "boolean", "description": "Slanted."})),
        ("underline", json!({"type": "boolean", "description": "Underlined."})),
        (
            "karaoke",
            nullable(json!({
                "description": "What is being said takes another colour: a voice goes through the title's characters at an even pace. null removes it. Give one caption a karaoke and captions.restyle gives it to the rest, each over its own length.",
                "type": "object",
                "properties": {
                    "color": color("The colour of what is lit; yellow unless given."),
                    "mode": {"enum": ["sweep", "word", "line"], "description": "sweep: everything said so far, the edge passing smoothly through the character being said. word: the word being said, alone (a Chinese character is a word of its own). line: the line being said, alone."},
                    "start": time("When the voice starts, from the clip's start; 0 by default."),
                    "duration": nullable(time("How long the voice takes; null, until the clip ends."))
                },
            })),
        ),
        (
            "template",
            json!({"type": "string", "description": "A title template id from title.templates (classic, variety, sticker, candy, fire, neon, retro, soft, boxed, label); other parameters given override it."}),
        ),
    ]
}

fn prop_schema() -> (&'static str, Value) {
    (
        "prop",
        json!({"enum": reelforge_model::Prop::ALL.map(reelforge_model::Prop::name).to_vec(), "description": "The property that moves: placement, opacity and volume; the crop's edges; the mask's centre, size, turn and soft edge; the colour adjustments; `colorR`, `colorG`, `colorB`, `colorA` (each 0 to 1), the colour of a title's words or of a shape's fill (of a gradient, the colour it starts from); `width`, `height`, `radius` and `strokeWidth` (canvas pixels), a shape's box, corner and outline; `strokeR`, `strokeG`, `strokeB`, `strokeA`, its outline's colour; `skewX`, `skewY` (degrees within ±89), how it leans."}),
    )
}

fn ease_schema() -> (&'static str, Value) {
    (
        "ease",
        json!({
            "description": "How the value moves on to the next keyframe: a curve by name, or one drawn by hand.",
            "oneOf": [
                {
                    "enum": reelforge_model::Ease::NAMED.map(|(n, _)| n),
                    "description": "A straight line; a jump at the next key; slow out and in; slow away (in); slow to arrive (out); past the next key and back; back a little first.",
                },
                {
                    "type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4,
                    "description": "A cubic Bézier [x1, y1, x2, y2] from (0, 0) to (1, 1), as CSS's cubic-bezier: across is time (each x from 0 to 1), up is how far the value has got (each y from -1 to 2).",
                },
            ],
        }),
    )
}

/// A list of picture effects.
fn fx(doc: &str) -> Value {
    let kinds: Vec<&str> = reelforge_model::FxKind::ALL.iter().map(|k| k.name()).collect();
    json!({
        "type": "array",
        "description": format!("{doc} Each is a preset's name, or the name with how strong and how fast it plays."),
        "items": {"oneOf": [
            {"enum": kinds},
            {
                "type": "object",
                "properties": {
                    "kind": {"enum": kinds},
                    "strength": {"type": "number", "minimum": 0, "maximum": 1, "description": "0.5 is the preset as designed."},
                    "speed": {"type": "number", "minimum": 0.25, "maximum": 4, "description": "Times the preset's own pace; 1 by default."}
                },
                "required": ["kind"],
                "additionalProperties": false
            }
        ]}
    })
}

/// A shape's parameters, as `clip.add`'s `shape` (an object) or `clip.setShape` (the clip's own).
fn shape_props() -> Vec<(&'static str, Value)> {
    let colour = || json!({"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 3, "maxItems": 4});
    vec![
        (
            "kind",
            json!({"enum": reelforge_model::ShapeKind::ALL.map(reelforge_model::ShapeKind::name), "description": "A box (corners rounded by radius), an ellipse, a line across the box, a regular polygon, a star (a burst badge with many points), a price tag pointed at its left end, an arrow pointing right."}),
        ),
        (
            "size",
            json!({"type": "array", "items": {"type": "number", "minimum": 0}, "minItems": 2, "maxItems": 2, "description": "Width and height in canvas pixels, before the clip's scale."}),
        ),
        ("radius", num("Corner radius in canvas pixels (rectangle, tag, arrow).")),
        (
            "points",
            json!({"type": "integer", "minimum": 3, "maximum": 64, "description": "Sides of a polygon, points of a star."}),
        ),
        (
            "inner",
            json!({"type": "number", "minimum": 0.05, "maximum": 1, "description": "How far out a star's inner corners are."}),
        ),
        (
            "fill",
            json!({"description": "A colour, a gradient, or null for none.", "oneOf": [
                colour(),
                {"type": "object", "properties": {"type": {"const": "linear"}, "from": colour(), "to": colour(), "angle": {"type": "number", "description": "Degrees clockwise from left-to-right; 90 is top to bottom."}}, "required": ["type", "from", "to"], "additionalProperties": false},
                {"type": "object", "properties": {"type": {"const": "radial"}, "from": colour(), "to": colour()}, "required": ["type", "from", "to"], "additionalProperties": false},
                {"type": "null"}
            ]}),
        ),
        (
            "stroke",
            json!({"description": "The outline, centred on the edge; null for none.", "oneOf": [{"type": "object", "properties": {"color": colour(), "width": {"type": "number", "minimum": 0, "description": "Canvas pixels."}}, "required": ["color", "width"], "additionalProperties": false}, {"type": "null"}]}),
        ),
        (
            "shadow",
            json!({"description": "A soft shadow beneath it; null for none.", "oneOf": [{"type": "object", "properties": {"color": colour(), "offset": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "[right, down] in canvas pixels."}, "blur": {"type": "number", "minimum": 0}}, "required": ["color", "offset"], "additionalProperties": false}, {"type": "null"}]}),
        ),
        (
            "trim",
            json!({"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 2, "maxItems": 2, "description": "The part of the outline drawn, [start, end] as fractions of the way round from the top-left."}),
        ),
    ]
}

fn shape_schema(doc: &str, _: bool) -> Value {
    let mut v = object(shape_props(), &[]);
    v["description"] = json!(doc);
    v
}

fn track_ids() -> Value {
    json!({
        "type": "array", "items": {"type": "string"},
        "description": "The tracks to cut; every track that is not locked by default. Leave a music track out and it plays on unbroken."
    })
}

fn filler_words() -> Value {
    json!({
        "type": "array", "items": {"type": "string"},
        "description": "More words to count as fillers, besides the built-in ones."
    })
}

fn object(props: Vec<(&'static str, Value)>, required: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in props {
        p.insert(k.to_string(), v);
    }
    json!({"type": "object", "properties": p, "required": required, "additionalProperties": false})
}

/// The parameter names command `id` takes, from its schema; `None` for an unknown id or one
/// whose schema lists no properties.
fn names(id: &str) -> Option<&'static [String]> {
    static NAMES: std::sync::OnceLock<std::collections::HashMap<&'static str, Vec<String>>> =
        std::sync::OnceLock::new();
    let all = NAMES.get_or_init(|| {
        crate::registry()
            .iter()
            .filter_map(|c| {
                let props = input_schema(c.id)?.get("properties")?.as_object()?.keys().cloned().collect::<Vec<_>>();
                Some((c.id, props))
            })
            .collect()
    });
    all.get(id).map(Vec::as_slice)
}

/// Refuses a parameter that command `id` does not take, so a misspelt or misplaced one (`y` on
/// clip.add, which places with clip.set) is not silently dropped; names a close one if there is.
pub(crate) fn check_names(id: &str, params: &Value) -> Result<(), String> {
    let (Some(names), Some(given)) = (names(id), params.as_object()) else { return Ok(()) };
    let Some(odd) = given.keys().find(|k| !names.iter().any(|n| n == *k)) else { return Ok(()) };
    let close = names.iter().find(|n| n.eq_ignore_ascii_case(odd) || n.replace('_', "") == odd.replace('_', ""));
    let hint = match close {
        Some(n) => format!("; did you mean `{n}`?"),
        None if names.is_empty() => "; it takes none".to_string(),
        None => format!("; it takes {}", names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")),
    };
    Err(format!("{id} has no parameter `{odd}`{hint}"))
}

/// The JSON Schema of command `id`'s parameters, or `None` for an unknown id.
pub fn input_schema(id: &str) -> Option<Value> {
    let clip = || ("clip", ident("Clip id, such as c4."));
    Some(match id {
        "commands.list" | "project.inspect" | "project.outline" | "project.new" | "edit.undo" | "edit.redo"
        | "title.templates" | "sound.library" | "timeline.list" | "fx.list" | "sticker.library" | "voice.list"
        | "template.list" | "template.slots" | "promo.list" | "brand.get" | "decor.list" | "layout.list"
        | "script.list" | "beat.list" | "preset.list" => object(vec![], &[]),
        "beat.apply" | "beat.cut" => {
            let moves: Vec<&str> = crate::beat::MOVES.iter().map(|m| m.id).collect();
            let cut = id == "beat.cut";
            let mut props = vec![
                (
                    "clips",
                    json!({"type": "array", "items": {"type": "string"}, "minItems": 1, "description": if cut {
                        "The clips, all on one track, in the order they are to follow one another."
                    } else {
                        "The clips to move: anything that is seen."
                    }}),
                ),
                (
                    "move",
                    json!({"enum": moves, "description": if cut {
                        "A beat move every clip arrives with, once, at its cut; none unless given."
                    } else {
                        "What the clips do on each beat; beat.list says what each looks like."
                    }}),
                ),
                ("strength", num("How far the move goes, 0 to 1; 0.5 is the move as designed.")),
                (
                    "every",
                    json!({"type": "integer", "minimum": 1, "maximum": 16, "description": if cut {
                        "How many beats each clip takes; 1 by default."
                    } else {
                        "Every how many beats the clips move; 1 by default, 4 for the first of each bar in four-four."
                    }}),
                ),
                (
                    "offset",
                    json!({"type": "integer", "minimum": 0, "maximum": 15, "description": "How many beats to leave out first; 0 by default."}),
                ),
                ("bpm", num("A tempo to use in place of the timeline's beat markers, in beats a minute, 20 to 400.")),
                ("first", time("With `bpm`: when the first beat is; 0 by default.")),
                (
                    "at",
                    json!({"type": "array", "items": time("A beat."), "minItems": 1, "description": "The beats themselves, in place of the timeline's beat markers."}),
                ),
            ];
            if cut {
                props.push((
                    "from",
                    time("Where the first clip starts; where the earliest of the clips starts by default."),
                ));
            }
            object(props, if cut { &["clips"] } else { &["clips", "move"] })
        }
        "layout.apply" => object(
            vec![
                (
                    "layout",
                    json!({"enum": crate::layout::LAYOUTS.iter().map(|l| l.id).collect::<Vec<_>>(), "description": "Which layout; layout.list says what each does."}),
                ),
                (
                    "clips",
                    json!({"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 6, "description": "The picture clips, in the order of their cells: for pip the large one first, for compare the one underneath."}),
                ),
                ("gap", num("The space between cells, as a share of the frame's shorter side; 0.012 by default.")),
                (
                    "margin",
                    num(
                        "The space round the cells, as a share of the frame's shorter side; 0 by default, 0.04 for pip.",
                    ),
                ),
                (
                    "corner",
                    json!({"enum": crate::layout::CORNERS, "description": "Where pip's small pictures sit; topRight by default."}),
                ),
                (
                    "size",
                    num(
                        "A small picture's width as a share of the frame's (a circle's, of the frame's shorter side), 0.1 to 0.6; 0.32 by default.",
                    ),
                ),
                ("round", json!({"type": "boolean", "description": "Small pictures as circles."})),
                (
                    "border",
                    json!({"anyOf": [
                        {"type": "object", "properties": {"color": color("White by default."), "width": num("Pixels; 6 on a frame 1080 across by default.")}, "additionalProperties": false},
                        {"type": "null"},
                    ], "description": "An outline round each cell; pip's small pictures have a white one unless this is null."}),
                ),
                ("from", num("Where compare's divider starts, 0 to 1 across the frame; 0.85 by default.")),
                ("to", num("Where it ends; 0.15 by default.")),
                (
                    "axis",
                    json!({"enum": ["x", "y"], "description": "The way compare's divider travels: across (x, the default) or down."}),
                ),
                ("divider", json!({"type": "boolean", "description": "false leaves compare's line and knob out."})),
            ],
            &["layout", "clips"],
        ),
        "decor.add" => object(
            vec![
                (
                    "decor",
                    json!({"enum": reelforge_media::decor::DECOR.iter().map(|d| d.id).collect::<Vec<_>>(), "description": "A library decoration (see decor.list)."}),
                ),
                ("start", time("Where it starts on the timeline; 0 by default.")),
                ("duration", time("How long it shows; as decor.list says by default.")),
                (
                    "track",
                    json!({"type": "string", "description": "A video track with room there; by default a free one above whatever shows then, or a new one."}),
                ),
                ("color", color("Its main colour, where it has one; the brand's primary, or its own red.")),
                ("color2", color("The colour beside it, where it has one; the brand's secondary, or its own yellow.")),
                ("x", num("Horizontal offset as a share of the canvas width; 0 (centred) by default.")),
                ("y", num("Vertical offset as a share of the canvas height; 0 is centred, positive is down.")),
                (
                    "scale",
                    json!({"type": "number", "exclusiveMinimum": 0, "description": "Size; 1 fits the canvas. As decor.list says by default."}),
                ),
                ("rotation", num("Clockwise, degrees.")),
                (
                    "opacity",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How solid it is; 1 by default."}),
                ),
                (
                    "name",
                    json!({"type": "string", "description": "What the project's media calls it; its English name by default."}),
                ),
            ],
            &["decor"],
        ),
        "sound.add" => object(
            vec![
                (
                    "sound",
                    json!({"enum": reelforge_media::library::SOUNDS.iter().map(|s| s.id).collect::<Vec<_>>(), "description": "A library sound (see sound.library)."}),
                ),
                ("at", time("Where it starts on the timeline; after the track's last clip by default.")),
                (
                    "track",
                    json!({"type": "string", "description": "An audio track; the first one (or a new one) by default."}),
                ),
                (
                    "name",
                    json!({"type": "string", "description": "The media's display name; the English name by default."}),
                ),
            ],
            &["sound"],
        ),
        "project.setCanvas" => object(
            vec![
                (
                    "width",
                    json!({"type": "integer", "minimum": 16, "maximum": 8192, "description": "Output width in pixels; even."}),
                ),
                (
                    "height",
                    json!({"type": "integer", "minimum": 16, "maximum": 8192, "description": "Output height in pixels; even."}),
                ),
                (
                    "ratio",
                    json!({"type": "string", "enum": crate::canvas::RATIOS.map(|r| r.0), "description": "A ready-made shape at a 1080-pixel shorter side; width and height, when given too, win."}),
                ),
                (
                    "refit",
                    json!({"type": "boolean", "description": "Whether clips follow a new size (true by default): pictures that covered the canvas go on covering it, titles keep their size against it."}),
                ),
                (
                    "fps",
                    json!({"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 2, "maxItems": 2, "description": "Frame rate as [numerator, denominator], such as [30, 1] or [30000, 1001]."}),
                ),
                ("background", color("Canvas background.")),
            ],
            &[],
        ),
        "project.setCover" => {
            let mut at = time("The instant whose frame is the cover; null for no cover.");
            at["oneOf"].as_array_mut().expect("a time is one of several shapes").push(json!({"type": "null"}));
            object(vec![("at", at)], &["at"])
        }
        "project.setExport" => {
            use reelforge_model::{Codec, Quality, Resolution, export};
            object(
                vec![
                    (
                        "preset",
                        json!({"enum": export::PRESETS.map(|(id, _)| id), "description": "Ready-made settings, applied before the others: draft (720p 30 fps, low), standard (1080p 30 fps), smooth (1080p 60 fps, high), 4k (2160p HEVC, high)."}),
                    ),
                    (
                        "format",
                        json!({"enum": export::Format::ALL.map(export::Format::name), "description": "The kind of file: mp4 (video and sound), gif (a silent animated GIF, its size and frame rate by `quality` alone), m4a (the sound alone, AAC) or wav (the sound alone, uncompressed)."}),
                    ),
                    (
                        "resolution",
                        json!({"enum": Resolution::ALL.map(Resolution::name), "description": "Picture size by its shorter side; canvas keeps the canvas's size."}),
                    ),
                    (
                        "fps",
                        json!({"enum": std::iter::once(0).chain(export::FRAME_RATES).collect::<Vec<_>>(), "description": "Frames per second; 0 keeps the canvas's rate."}),
                    ),
                    (
                        "quality",
                        json!({"enum": Quality::ALL.map(Quality::name), "description": "Picture quality, and so file size."}),
                    ),
                    (
                        "codec",
                        json!({"enum": Codec::ALL.map(Codec::name), "description": "Video codec. HEVC and AV1 need the browser's encoder."}),
                    ),
                ],
                &[],
            )
        }
        "media.import" => object(
            vec![
                (
                    "path",
                    json!({"type": "string", "description": "File path or URI of the media, as the renderer will reach it."}),
                ),
                (
                    "kind",
                    json!({"enum": ["video", "image", "audio", "lut", "font", "lottie"], "description": "Measured from the file when it is reachable."}),
                ),
                ("name", json!({"type": "string", "description": "Display name; the file name by default."})),
                ("duration", time("Length, when the file cannot be measured.")),
                (
                    "size",
                    json!({"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2, "description": "Picture size [w, h], when the file cannot be measured."}),
                ),
                (
                    "sha256",
                    json!({"type": "string", "pattern": "^[0-9a-f]{64}$", "description": "Content hash, when the file cannot be read."}),
                ),
            ],
            &["path"],
        ),
        "capcut.import" => object(
            vec![
                ("draft", json!({"type": "string", "description": "The text of the draft's draft_content.json."})),
                ("folder", json!({"type": "string", "description": "The draft's folder, for paths relative to it."})),
                ("name", json!({"type": "string", "description": "Timeline name; the draft's by default."})),
            ],
            &["draft"],
        ),
        "timeline.add" => object(
            vec![
                ("name", json!({"type": "string", "description": "Name; \"Timeline N\" by default."})),
                ("copy", json!({"type": "boolean", "description": "Start as a copy of the current timeline."})),
            ],
            &[],
        ),
        "timeline.switch" => object(vec![("timeline", ident("Timeline id, such as s4."))], &["timeline"]),
        "timeline.rename" => object(
            vec![
                ("timeline", ident("Timeline id; the current one by default.")),
                ("name", json!({"type": "string", "minLength": 1, "description": "New name."})),
            ],
            &["name"],
        ),
        "media.set" => object(
            vec![
                (
                    "media",
                    json!({
                        "description": "Media id, such as m1, or several to file under a folder at once.",
                        "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}],
                    }),
                ),
                ("name", ident("The file's new name in the media panel (one file at a time).")),
                (
                    "folder",
                    json!({
                        "type": ["string", "null"],
                        "description": "The folder to file the media under (it appears when first used); null takes it out of its folder.",
                    }),
                ),
            ],
            &["media"],
        ),
        "media.remove" => object(vec![("media", ident("Media id, such as m1."))], &["media"]),
        "timeline.remove" => object(vec![("timeline", ident("Timeline id; the current one by default."))], &[]),
        "timeline.cut" => object(
            vec![
                (
                    "ranges",
                    json!({
                        "type": "array", "minItems": 1,
                        "description": "The stretches of timeline time to take out; ones that overlap are joined.",
                        "items": {
                            "type": "object",
                            "properties": {"from": time("Where the stretch starts."), "to": time("Where it ends.")},
                            "required": ["from", "to"], "additionalProperties": false
                        }
                    }),
                ),
                ("tracks", track_ids()),
            ],
            &["ranges"],
        ),
        "captions.fillers" => object(
            vec![("track", ident("A text track; the Captions track by default.")), ("words", filler_words())],
            &[],
        ),
        "captions.cutFillers" => object(
            vec![
                ("track", ident("A text track; the Captions track by default.")),
                ("words", filler_words()),
                ("tracks", track_ids()),
            ],
            &[],
        ),
        "track.add" => object(
            vec![
                ("kind", json!({"enum": ["video", "text", "audio"], "description": "What the track holds."})),
                ("name", json!({"type": "string", "description": "Track name."})),
            ],
            &["kind"],
        ),
        "track.set" => object(
            vec![
                ("track", ident("Track id, such as t1.")),
                ("name", json!({"type": "string", "minLength": 1, "description": "New name."})),
                (
                    "hidden",
                    json!({"type": "boolean", "description": "Hide the track's picture (video and text tracks)."}),
                ),
                ("muted", json!({"type": "boolean", "description": "Mute the track's sound."})),
                (
                    "magnetic",
                    json!({"type": "boolean", "description": "Keep the clips end to end from the start: removing or trimming closes gaps, moving reorders."}),
                ),
                ("locked", json!({"type": "boolean", "description": "Lock the track's clips against changes."})),
            ],
            &["track"],
        ),
        "clip.add" => {
            // a stretch, a turn in space, an anchor and motion blur, as clip.set says them
            let set = input_schema("clip.set").expect("clip.set has a schema");
            let placed = [
                "scaleX",
                "scaleY",
                "rotationX",
                "rotationY",
                "skewX",
                "skewY",
                "perspective",
                "motionBlur",
                "anchorX",
                "anchorY",
            ]
            .map(|k| (k, set["properties"][k].clone()));
            let mut props = vec![
                ("track", ident("Track id, such as t1 (project.inspect lists them).")),
                ("start", time("Timeline start; the end of the track by default.")),
                ("duration", time("Timeline length; the media's remaining length, or 5 s, by default.")),
                ("media", ident("Media id (from media.import) for a video, image or audio clip.")),
                ("sourceIn", time("Where in the media the clip starts.")),
                (
                    "sticker",
                    json!({"enum": reelforge_media::stickers::STICKERS.iter().map(|s| s.id).collect::<Vec<_>>(), "description": "A library sticker (see sticker.library), placed as an image a third of the frame wide."}),
                ),
                (
                    "name",
                    json!({"type": "string", "description": "With `sticker`: what the project's media calls it."}),
                ),
                (
                    "adjustment",
                    json!({"type": "boolean", "description": "An adjustment clip (video tracks): it shows nothing; its `fx` and its colour (clip.setColor) work on the tracks below while it lasts."}),
                ),
                (
                    "timeline",
                    ident(
                        "Another timeline of the project (timeline.list), placed as a compound clip on a video track: its picture and sound as one clip.",
                    ),
                ),
                ("fx", fx("Picture effects the clip starts with.")),
                ("x", num("Horizontal offset as a share of the canvas width; 0 (centred) by default.")),
                ("y", num("Vertical offset as a share of the canvas height; 0 is centred, positive is down.")),
                ("scale", num("Size; 1 fits the canvas (a sticker starts at 0.35).")),
                ("rotation", num("Clockwise, degrees.")),
                (
                    "opacity",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "Opacity; 1 by default."}),
                ),
                ("volumeDb", num("Gain in dB; 0 is unchanged.")),
                (
                    "shape",
                    shape_schema(
                        "A drawn shape on a video track (see clip.setShape): a 400 × 400 white box unless it says otherwise.",
                        false,
                    ),
                ),
            ];
            props.extend(title_props());
            props.extend(placed);
            object(props, &["track"])
        }
        "clip.setFx" => object(
            vec![clip(), ("fx", fx("The clip's effects from now on; an empty list removes them."))],
            &["clip", "fx"],
        ),
        "clip.setVoice" => object(
            vec![
                clip(),
                (
                    "voice",
                    json!({"enum": ["high", "low", "chipmunk", "monster", "robot", "phone", "megaphone", "echo", null], "description": "A ready-made voice (voice.list); null takes it off."}),
                ),
                (
                    "pitch",
                    json!({"type": "number", "minimum": -12, "maximum": 12, "description": "Semitones up (negative: down), on top of the voice's own; the sound keeps its length."}),
                ),
            ],
            &["clip"],
        ),
        "clip.setDenoise" => object(
            vec![
                clip(),
                (
                    "amount",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How much noise is taken out; 0 turns it off."}),
                ),
                (
                    "floor",
                    json!({"type": "array", "items": {"type": "number"}, "minItems": 24, "maxItems": 24, "description": "The noise: its power density in dB in 24 bands evenly spaced on a log scale from 60 Hz to 16 kHz. audio.denoise measures it; without it the clip's own is kept."}),
                ),
            ],
            &["clip", "amount"],
        ),
        "clip.setStabilize" => object(
            vec![
                clip(),
                (
                    "amount",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How much of the shake goes; 0 turns it off."}),
                ),
                (
                    "path",
                    json!({
                        "type": "array",
                        "items": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                        "description": "The camera's path as measured: where the picture is at each sample against the first, as [across, down, turn] in fractions of its width and height and in degrees. clip.stabilize measures it; without it the clip's own is kept.",
                    }),
                ),
                ("from", time("The source instant of the path's first sample; the file's start by default.")),
                ("step", time("The source time from one sample to the next; required with `path`.")),
            ],
            &["clip", "amount"],
        ),
        "captions.import" => object(
            vec![
                (
                    "srt",
                    json!({"type": "string", "description": "The SubRip text: numbered cues with `hh:mm:ss,mmm --> hh:mm:ss,mmm` times."}),
                ),
                ("track", ident("A text track to put them on; a new Captions track by default.")),
                (
                    "template",
                    json!({"type": "string", "description": "Title template id (title.templates); classic by default."}),
                ),
                (
                    "line",
                    json!({"enum": [1, 2], "description": "2 places a second language: smaller, under the first, on its own track."}),
                ),
            ],
            &["srt"],
        ),
        "captions.list" => object(vec![("track", ident("A text track; the Captions track by default."))], &[]),
        "captions.split" => object(
            vec![
                ("clip", ident("The caption (a title clip).")),
                (
                    "at",
                    json!({"type": "integer", "minimum": 1, "description": "How many characters stay in the first half."}),
                ),
                ("time", time("Where the time is cut; in proportion to the words by default.")),
            ],
            &["clip"],
        ),
        "captions.merge" => object(
            vec![(
                "clips",
                json!({"type": "array", "items": {"type": "string"}, "minItems": 2, "description": "Captions on one track."}),
            )],
            &["clips"],
        ),
        "captions.replace" => object(
            vec![
                ("find", json!({"type": "string", "minLength": 1, "description": "The words to look for."})),
                ("replace", json!({"type": "string", "description": "What goes in their place."})),
                ("track", ident("A text track; the Captions track by default.")),
            ],
            &["find", "replace"],
        ),
        "captions.restyle" => object(
            vec![
                ("from", ident("The title whose look the others take.")),
                ("track", ident("A text track; the one `from` is on by default.")),
            ],
            &["from"],
        ),
        "captions.shift" => object(
            vec![
                ("by", time("How much later (earlier when negative).")),
                ("track", ident("A text track; the Captions track by default.")),
            ],
            &["by"],
        ),
        "captions.add" => object(
            vec![
                (
                    "cues",
                    json!({
                        "type": "array",
                        "description": "The lines, each shown from `start` to `end`; `\\n` in `text` breaks a line.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "start": time("When the line appears."),
                                "end": time("When it goes."),
                                "text": {"type": "string"}
                            },
                            "required": ["start", "end", "text"],
                            "additionalProperties": false
                        }
                    }),
                ),
                ("track", ident("A text track to put them on; a new Captions track by default.")),
                (
                    "template",
                    json!({"type": "string", "description": "Title template id (title.templates); classic by default."}),
                ),
            ],
            &["cues"],
        ),
        "voiceover.add" => object(
            vec![
                (
                    "lines",
                    json!({
                        "type": "array",
                        "minItems": 1,
                        "description": "The spoken lines, each a sound file and where it starts.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "The sound file, as for media.import."},
                                "start": time("Where on the timeline it starts."),
                                "name": {"type": "string", "description": "Display name in the media list; the file name by default."},
                                "sha256": {"type": "string", "pattern": "^[0-9a-f]{64}$", "description": "Content hash, when the file cannot be read here."},
                                "duration": time("Length, when the file cannot be measured here.")
                            },
                            "required": ["path", "start"],
                            "additionalProperties": false
                        }
                    }),
                ),
                ("track", ident("An audio track to put them all on; Voice-over tracks by default.")),
            ],
            &["lines"],
        ),
        "captions.export" => object(vec![("track", ident("A text track; the Captions track by default."))], &[]),
        "clip.split" => object(vec![clip(), ("at", time("Timeline time inside the clip."))], &["clip", "at"]),
        "clip.trim" => {
            object(vec![clip(), ("start", time("New start edge.")), ("end", time("New end edge."))], &["clip"])
        }
        "clip.move" => object(
            vec![
                clip(),
                ("start", time("New timeline start.")),
                ("track", ident("Track to move to, of the same kind.")),
            ],
            &["clip", "start"],
        ),
        "clip.roll" => {
            object(vec![clip(), ("by", time("How far the cut moves; negative is earlier."))], &["clip", "by"])
        }
        "clip.slip" => object(
            vec![clip(), ("by", time("How much later in the media the clip shows; negative is earlier."))],
            &["clip", "by"],
        ),
        "clip.detachAudio" => object(vec![clip()], &["clip"]),
        "track.move" => object(
            vec![
                ("track", ident("Track id, such as t2.")),
                (
                    "index",
                    json!({"type": "integer", "minimum": 0, "description": "Place among the tracks of its kind, 0 at the bottom."}),
                ),
            ],
            &["track", "index"],
        ),
        "track.remove" => object(vec![("track", ident("Track id, such as t2."))], &["track"]),
        "marker.add" => object(
            vec![("at", time("Where.")), ("note", json!({"type": "string", "description": "What it is for."}))],
            &["at"],
        ),
        "marker.set" => object(
            vec![
                ("marker", ident("Marker id, such as mk7.")),
                ("at", time("Where it goes.")),
                ("note", json!({"type": "string", "description": "What it is for."})),
            ],
            &["marker"],
        ),
        "marker.remove" => object(vec![("marker", ident("Marker id, such as mk7."))], &["marker"]),
        "clip.shift" => object(
            vec![
                (
                    "clips",
                    json!({"description": "A clip id or a list of them.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
                ),
                ("by", time("How far to move them; negative is earlier.")),
            ],
            &["clips", "by"],
        ),
        "clip.group" => object(
            vec![(
                "clips",
                json!({"type": "array", "items": {"type": "string"}, "minItems": 2, "description": "The clips to group."}),
            )],
            &["clips"],
        ),
        "clip.ungroup" => object(
            vec![(
                "clips",
                json!({"description": "A clip id or a list of them; the whole group of each is dissolved.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
            )],
            &["clips"],
        ),
        "clip.compound" => object(
            vec![
                (
                    "clips",
                    json!({"description": "A clip id or a list of them; at least one shows a picture or text.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
                ),
                (
                    "name",
                    json!({"type": "string", "description": "What the compound clip's timeline is called; Compound 1, 2, … by default."}),
                ),
            ],
            &["clips"],
        ),
        "clip.uncompound" => object(vec![("clip", ident("The compound clip's id."))], &["clip"]),
        "clip.copy" | "clip.duplicate" => object(
            vec![(
                "clips",
                json!({"description": "A clip id or a list of them.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
            )],
            &["clips"],
        ),
        "clip.paste" => object(
            vec![
                (
                    "clipboard",
                    json!({"type": "object", "description": "What `clip.copy` returned.", "properties": {"clips": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {"track": {"type": "string"}, "kind": {"enum": ["video", "text", "audio"]}, "clip": {"type": "object"}}, "required": ["kind", "clip"]}}}, "required": ["clips"]}),
                ),
                ("at", time("Where the earliest clip starts.")),
            ],
            &["clipboard", "at"],
        ),
        "clip.remove" => object(
            vec![
                (
                    "clips",
                    json!({"description": "A clip id or a list of them.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
                ),
                (
                    "ripple",
                    json!({"type": "boolean", "description": "Move later clips on the same track up to close the gaps."}),
                ),
            ],
            &["clips"],
        ),
        "clip.setSpeed" => object(
            vec![
                clip(),
                (
                    "speed",
                    json!({"type": "number", "exclusiveMinimum": 0, "description": "Playback speed; 2 is twice as fast."}),
                ),
                (
                    "keepPitch",
                    json!({"type": "boolean", "description": "Sound keeps its pitch at another speed (default); false lets it rise and fall like tape."}),
                ),
                (
                    "frames",
                    json!({"enum": ["repeat", "blend", "flow"], "description": "What a video shows between two frames of its file (a slowed clip has to): the frame before held, the two mixed, or a frame made in between by following what moved."}),
                ),
            ],
            &["clip"],
        ),
        "clip.set" => {
            let mut props = vec![
                clip(),
                ("x", num("Horizontal offset as a share of the canvas width; 0 is centred.")),
                ("y", num("Vertical offset as a share of the canvas height; 0 is centred, positive is down.")),
                ("scale", num("Size; 1 fits the canvas.")),
                ("rotation", num("Clockwise, degrees.")),
                ("opacity", json!({"type": "number", "minimum": 0, "maximum": 1, "description": "Opacity."})),
                ("volumeDb", num("Gain in dB; 0 is unchanged.")),
                ("scaleX", num("Width alone, on top of the scale: 1 as it is, negative mirrored.")),
                ("scaleY", num("Height alone, on top of the scale.")),
                (
                    "skewX",
                    json!({"type": "number", "minimum": -89, "maximum": 89, "description": "Leaning across about its anchor, degrees, as CSS skewX: positive slants its top edge left."}),
                ),
                (
                    "skewY",
                    json!({"type": "number", "minimum": -89, "maximum": 89, "description": "Leaning down about its anchor, degrees, as CSS skewY: positive drops its right edge."}),
                ),
                (
                    "rotationX",
                    num(
                        "Turned in space about the horizontal axis through its anchor, degrees: positive tips the top edge away.",
                    ),
                ),
                (
                    "rotationY",
                    num(
                        "Turned in space about the vertical axis through its anchor, degrees: positive swings the right edge away; past 90 it shows its back.",
                    ),
                ),
                (
                    "perspective",
                    json!({"type": "number", "minimum": 0.2, "maximum": 50, "description": "How far the eye is from a clip turned in space, in canvas heights; 2 by default, nearer is stronger."}),
                ),
                (
                    "motionBlur",
                    json!({"type": ["boolean", "number"], "minimum": 0, "maximum": 2, "description": "Motion blur: the clip smeared along its own movement while it moves, grows or turns, sharp at rest. true turns it on, false or 0 off; a number is the share of a frame's time the shutter is open (0.5 by default, a film camera's; up to 2)."}),
                ),
                (
                    "anchorX",
                    json!({"type": "number", "minimum": -4, "maximum": 5, "description": "The point of the picture that `x` and `y` place and that it is scaled and turned about, across: 0 its left edge, 0.5 its middle, 1 its right edge."}),
                ),
                (
                    "anchorY",
                    json!({"type": "number", "minimum": -4, "maximum": 5, "description": "The same, down: 0 its top edge, 0.5 its middle, 1 its bottom edge."}),
                ),
                (
                    "keep",
                    json!({"type": "boolean", "description": "With another anchor, whether the picture stays where it lies (`x` and `y` change to match); true by default."}),
                ),
                ("at", time("Timeline time at which keyframed properties are set.")),
                ("fadeIn", time("Sound fade-in from the clip's start; 0 removes it.")),
                ("fadeOut", time("Sound fade-out up to the clip's end; 0 removes it.")),
            ];
            props.extend(title_props());
            object(props, &["clip"])
        }
        "clip.setKey" => object(
            vec![
                clip(),
                prop_schema(),
                ("at", time("Timeline time of the keyframe.")),
                ("value", num("Value there; the property's current value at that time by default.")),
                ease_schema(),
            ],
            &["clip", "prop", "at"],
        ),
        "clip.setParent" => object(
            vec![
                clip(),
                (
                    "parent",
                    json!({"description": "The clip that carries it: a picture, title or shape clip of the same timeline; null sets it free.", "oneOf": [{"type": "string"}, {"type": "null"}]}),
                ),
                (
                    "keep",
                    json!({"type": "boolean", "description": "Whether the clip stays where it shows (its x, y, scale and rotation are rewritten for the new parent); true by default."}),
                ),
                ("at", time("Timeline time at which it stays where it shows; the clip's start by default.")),
            ],
            &["clip", "parent"],
        ),
        "clip.setPath" => object(
            vec![
                clip(),
                (
                    "points",
                    json!({
                        "description": "The points the path passes through, each [x, y] counted as the clip's x and y are (shares of the canvas from its centre, or from the parent's anchor); null removes the path. Left out, the path the clip has keeps its points.",
                        "oneOf": [
                            {
                                "type": "array", "minItems": 2, "maxItems": 64,
                                "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                            },
                            {"type": "null"},
                        ],
                    }),
                ),
                (
                    "closed",
                    json!({"type": "boolean", "description": "Goes on from the last point round to the first again."}),
                ),
                (
                    "orient",
                    json!({"type": "boolean", "description": "Turns the clip to face the way it goes: the path's heading (0 going right, 90 going down) is added to its rotation."}),
                ),
                ("start", time("When it sets off, from the clip's start; 0 by default.")),
                ("duration", time("How long the journey takes; the rest of the clip by default.")),
                ease_schema(),
            ],
            &["clip"],
        ),
        "clip.setEase" => object(
            vec![
                clip(),
                ("at", time("Timeline time of the keyframes the curve leaves from, to within half a frame.")),
                ease_schema(),
                (
                    "props",
                    json!({
                        "type": "array", "minItems": 1,
                        "items": {"enum": reelforge_model::Prop::ALL.map(reelforge_model::Prop::name).to_vec()},
                        "description": "Only these properties' keyframes; every property keyed at that time by default.",
                    }),
                ),
            ],
            &["clip", "at", "ease"],
        ),
        "clip.removeKey" => object(
            vec![clip(), prop_schema(), ("at", time("Timeline time of the keyframe, to within half a frame."))],
            &["clip", "prop", "at"],
        ),
        "clip.clearKeys" => object(
            vec![
                clip(),
                prop_schema(),
                ("at", time("Timeline time whose values the clip keeps; its start by default.")),
            ],
            &["clip"],
        ),
        "clip.setShape" => {
            let mut props = vec![clip()];
            props.extend(shape_props());
            object(props, &["clip"])
        }
        "clip.setCounter" => object(
            vec![
                clip(),
                (
                    "counter",
                    json!({"description": "The number the title shows, written where its words say {n} (or as all of them); null removes it.", "oneOf": [{"type": "null"}, {
                        "type": "object",
                        "properties": {
                            "from": {"type": "number", "description": "The value at the clip's start (0 by default)."},
                            "to": {"type": "number", "description": "The value it arrives at (100 by default)."},
                            "over": {"description": "How long it takes; the whole clip by default.", "oneOf": [time("A length."), {"type": "null"}]},
                            "ease": ease_schema().1,
                            "decimals": {"type": "integer", "minimum": 0, "maximum": 4},
                            "group": {"type": "boolean", "description": "Commas between thousands."},
                            "clock": {"type": "boolean", "description": "Whole seconds written as a clock, 01:30."}
                        },
                        "additionalProperties": false
                    }]}),
                ),
            ],
            &["clip", "counter"],
        ),
        "script.add" | "script.check" => {
            let mut props = vec![
                (
                    "source",
                    json!({"type": "string", "description": "The script: JavaScript with `export function build({ params, canvas, scene, timeline })` and, if it takes parameters, `export const params = {name: {type, default, label?}}`. See docs/scripts.md."}),
                ),
                (
                    "params",
                    json!({"type": "object", "description": "Values for the parameters the script declares, by name; the rest take their defaults."}),
                ),
                (
                    "duration",
                    time(
                        "How long the piece is; else as long as the script says (scene.setDuration), else until its last tween ends.",
                    ),
                ),
                (
                    "seed",
                    json!({"type": "integer", "minimum": 0, "description": "What the script's Math.random starts from; 1 by default."}),
                ),
            ];
            if id == "script.add" {
                props.push(("start", time("Where the piece starts on the timeline; 0 by default.")));
                props.push(("name", json!({"type": "string", "description": "What to call it."})));
            }
            object(props, &["source"])
        }
        "script.get" | "script.detach" => {
            object(vec![("group", ident("The script's group id, such as g7 (script.list)."))], &["group"])
        }
        "script.setParams" => object(
            vec![
                ("group", ident("The script's group id, such as g7 (script.list).")),
                (
                    "params",
                    json!({"type": "object", "description": "The values to change, by name; null puts one back to the script's default."}),
                ),
                ("reset", json!({"type": "boolean", "description": "Puts every parameter back to its default first."})),
                ("duration", nullable(time("How long the piece is."))),
                (
                    "seed",
                    json!({"type": "integer", "minimum": 0, "description": "What the script's Math.random starts from."}),
                ),
            ],
            &["group"],
        ),
        "script.setSource" => object(
            vec![
                ("group", ident("The script's group id, such as g7 (script.list).")),
                ("source", json!({"type": "string", "description": "The new source."})),
                (
                    "params",
                    json!({"type": "object", "description": "All its parameter values anew; without this, those the new source still declares are kept."}),
                ),
                ("name", json!({"type": "string", "description": "What to call it."})),
            ],
            &["group", "source"],
        ),
        "preset.save" => object(
            vec![
                ("group", ident("Keep the script that built this group, such as g7 (script.list), with its values.")),
                ("source", json!({"type": "string", "description": "Or keep this script source (see script.add)."})),
                (
                    "params",
                    json!({"type": "object", "description": "With `source`: the values to keep, by name; the rest stay the script's defaults."}),
                ),
                ("name", json!({"type": "string", "description": "What to call the preset."})),
                (
                    "seed",
                    json!({"type": "integer", "minimum": 0, "description": "With `source`: what its Math.random starts from."}),
                ),
                ("duration", time("With `source`: how long it is.")),
            ],
            &[],
        ),
        "preset.get" | "preset.remove" => {
            object(vec![("preset", ident("The preset's id, such as p12, or its name (preset.list)."))], &["preset"])
        }
        "preset.set" => object(
            vec![
                ("preset", ident("The preset's id, such as p12, or its name (preset.list).")),
                ("name", json!({"type": "string", "description": "Its new name."})),
            ],
            &["preset", "name"],
        ),
        "preset.apply" => object(
            vec![
                ("preset", ident("The preset's id, such as p12, or its name (preset.list).")),
                ("start", time("Where it starts on the timeline; 0 by default.")),
                (
                    "params",
                    json!({"type": "object", "description": "Values over those it was saved with, by name; null takes the script's default."}),
                ),
                ("duration", time("How long the piece is.")),
                (
                    "seed",
                    json!({"type": "integer", "minimum": 0, "description": "What the script's Math.random starts from."}),
                ),
            ],
            &["preset"],
        ),
        "promo.add" | "promo.set" => {
            let mut props = vec![
                ("text", json!({"type": "string", "description": "Its words; {n} is where its number goes."})),
                (
                    "lines",
                    json!({"type": "array", "items": {"type": "string", "maxLength": 200}, "maxItems": 6, "description": "Its further lines: the rows of a list or table, or the smaller words beside the main ones (promo.list says which)."}),
                ),
                (
                    "value",
                    num(
                        "The number it counts to (a price, a count, a rating); a countdown's seconds to count down from.",
                    ),
                ),
                (
                    "values",
                    json!({"type": "array", "items": {"type": "number"}, "maxItems": 4, "description": "The numbers of a piece that compares, such as the two prices of priceCompare."}),
                ),
                ("from", num("Where counting starts; 0 for most.")),
                ("color", color("Its main colour; the brand's primary, or red.")),
                ("color2", color("The colour beside it; the brand's secondary, or yellow.")),
                (
                    "side",
                    json!({"enum": crate::pieces::Side::NAMES, "description": "The way it leans, points or moves, for the pieces that do."}),
                ),
                (
                    "media",
                    if id == "promo.set" {
                        nullable(ident("An image or video of the project the piece shows; null takes it off."))
                    } else {
                        ident(
                            "An image or video of the project the piece shows, such as m2: the product on productCard or productStage.",
                        )
                    },
                ),
                (
                    "x",
                    num(
                        "Horizontal offset of its centre from the frame's, as a fraction of the width; each piece has its own by default.",
                    ),
                ),
                ("y", num("Vertical offset, as a fraction of the height.")),
                (
                    "scale",
                    num("Its size; 1 is as designed for a frame whose shorter side is 1080, in proportion for others."),
                ),
            ];
            if id == "promo.set" {
                props.insert(0, ("clip", ident("The compound clip the piece is, such as c9.")));
                return Some(object(props, &["clip"]));
            }
            props.insert(
                0,
                (
                    "kind",
                    json!({"enum": crate::pieces::PROMOS.iter().map(|p| p.id).collect::<Vec<_>>(), "description": "Which piece; promo.list says what each is and what its parameters mean."}),
                ),
            );
            props.insert(1, ("start", time("Where it starts; 0 by default.")));
            props.insert(2, ("duration", time("How long it shows; the piece's own length by default, 3 s for most.")));
            props.push(("compound", json!({"type": "boolean", "description": "Fold it into one compound clip, which promo.set can give other words later."})));
            object(props, &["kind"])
        }
        "storyboard.list" | "storyboard.copy" => object(vec![("lang", storyboard_lang())], &[]),
        "storyboard.apply" => object(
            vec![
                (
                    "storyboard",
                    json!({"enum": crate::storyboard::storyboards().iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), "description": "Which storyboard (see storyboard.list)."}),
                ),
                (
                    "copy",
                    storyboard_copy(
                        "Words and numbers for its fields, by name; those not given keep what the storyboard comes with.",
                    ),
                ),
                (
                    "media",
                    json!({
                        "type": "array",
                        "items": {"type": ["string", "null"]},
                        "minItems": 1,
                        "description": "Media ids for the scenes' pictures, in order; null skips one.",
                    }),
                ),
                ("lang", storyboard_lang()),
                (
                    "seconds",
                    json!({"type": "number", "exclusiveMinimum": 0, "description": "How long the whole runs: half to twice the storyboard's own length, each scene in proportion."}),
                ),
                (
                    "captions",
                    json!({"type": "boolean", "description": "Also write each scene's line to say as captions."}),
                ),
                ("name", ident("The new timeline's name, when the storyboard gets a timeline of its own.")),
            ],
            &["storyboard"],
        ),
        "storyboard.fill" => object(
            vec![
                (
                    "copy",
                    storyboard_copy(
                        "The fields to change, by name (storyboard.copy lists them); a name the copy does not have yet is added.",
                    ),
                ),
                ("lang", storyboard_lang()),
            ],
            &["copy"],
        ),
        "promo.get" => object(vec![("clip", ident("Clip id, such as c9."))], &["clip"]),
        "brand.set" => object(
            vec![
                ("primary", nullable(color("The main colour of ready-made pieces."))),
                ("secondary", nullable(color("The colour beside it."))),
                ("font", nullable(ident("An imported font, such as m3."))),
                ("logo", nullable(ident("An imported picture, such as m4."))),
            ],
            &[],
        ),
        "clip.setAnimation" => object(
            vec![
                clip(),
                (
                    "slot",
                    json!({"enum": ["in", "out", "loop"], "description": "Plays from the clip's start (in), up to its end (out), or round and round for as long as the clip is on screen (loop)."}),
                ),
                (
                    "kind",
                    json!({"description": "Preset; null removes the animation. In and out: fade to spin, flipY (swings in like a door) and flipX (tips up like a lid), both in space about the clip's anchor, and for titles only typewriter (types the words in, or erases them) and charFade, charRise, charDrop, charPop (the characters one after another), wordFade, wordRise, wordPop (word by word), lineRise (line by line), charElastic (each springs up past its size and back) and charBounce (each falls and bounces), for shapes and titles draw (a pen draws the outline on, then the fill comes in), and for any clip elastic (grows in on a spring) and drop (falls in and bounces). Loop: breathe, heartbeat, swing, float, bounce, shake, spin, blink, flip (round and round in space, showing its back), tilt (rocks from side to side in space), and for titles only wave and jump (the characters one after another).", "oneOf": [{"enum": ["fade", "slideUp", "slideDown", "slideLeft", "slideRight", "zoomIn", "zoomOut", "pop", "spin", "typewriter", "charFade", "charRise", "charDrop", "charPop", "draw", "flipY", "flipX", "elastic", "drop", "wordFade", "wordRise", "wordPop", "lineRise", "charElastic", "charBounce", "breathe", "heartbeat", "swing", "float", "bounce", "shake", "blink", "wave", "jump", "flip", "tilt"]}, {"type": "null"}]}),
                ),
                (
                    "duration",
                    time(
                        "How long it plays, 0.5 s by default; for a loop, how long one round takes (1 s or so by default, by kind).",
                    ),
                ),
            ],
            &["clip", "slot", "kind"],
        ),
        "clip.setColor" => {
            let mut props = vec![
                clip(),
                (
                    "filter",
                    json!({"description": "A ready-made look; null removes it.", "oneOf": [{"enum": ["fresh", "warm", "cool", "mono", "vintage", "film", "vivid", "dusk"]}, {"type": "null"}]}),
                ),
                (
                    "filterStrength",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How strongly the filter applies; 1 by default."}),
                ),
                (
                    "lut",
                    json!({"description": "A lookup table: the id of `lut` media (imported from a .cube file); null removes it.", "oneOf": [{"type": "string"}, {"type": "null"}]}),
                ),
                (
                    "lutStrength",
                    json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How strongly the table applies; 1 by default."}),
                ),
                ("reset", json!({"type": "boolean", "description": "Clear the filter, table and adjustments first."})),
                ("at", time("Where a value with keyframes is set: the key there changes or one is added.")),
            ];
            for (name, lo, hi) in reelforge_model::Adjust::RANGES {
                props.push((
                    name,
                    json!({"type": "number", "minimum": lo, "maximum": hi, "description": format!("Adjustment, {lo} to {hi}; 0 changes nothing.")}),
                ));
            }
            object(props, &["clip"])
        }
        "clip.setCrop" => {
            let mut props = vec![clip()];
            for edge in ["left", "top", "right", "bottom"] {
                props.push((
                    edge,
                    json!({"type": "number", "minimum": 0, "exclusiveMaximum": 1, "description": "Fraction cut off this edge; at least 1 % of the picture must stay."}),
                ));
            }
            props.push(("flipX", json!({"type": "boolean", "description": "Mirrored left to right."})));
            props.push(("flipY", json!({"type": "boolean", "description": "Mirrored top to bottom."})));
            props.push(("at", time("Where a value with keyframes is set: the key there changes or one is added.")));
            object(props, &["clip"])
        }
        "clip.setBackdrop" => object(
            vec![
                clip(),
                (
                    "blur",
                    json!({"description": "From 0 (a little blurred) to 1 (very); null removes the backdrop.", "oneOf": [{"type": "number", "minimum": 0, "maximum": 1}, {"type": "null"}]}),
                ),
                ("all", json!({"type": "boolean", "description": "Every video and picture clip on the clip's track."})),
            ],
            &["clip", "blur"],
        ),
        "clip.setPersonKey" => object(
            vec![
                clip(),
                ("on", json!({"type": "boolean", "description": "false removes the cut-out; true when left out."})),
            ],
            &["clip"],
        ),
        "clip.setCutout" => {
            let unit = |doc: &str| json!({"type": "number", "minimum": 0, "maximum": 1, "description": doc});
            object(
                vec![
                    clip(),
                    ("on", json!({"type": "boolean", "description": "false removes the cut-out; true when left out."})),
                    ("tolerance", unit("How far from the backdrop's colour still goes; 0.5 at first.")),
                    (
                        "holes",
                        unit("How readily backdrop-coloured places the edges do not reach go; 0 never, 0.5 at first."),
                    ),
                ],
                &["clip"],
            )
        }
        "clip.setShadow" => {
            let within = |low: f64, high: f64, doc: &str| json!({"type": "number", "minimum": low, "maximum": high, "description": doc});
            object(
                vec![
                    clip(),
                    ("on", json!({"type": "boolean", "description": "false removes the shadow; true when left out."})),
                    (
                        "kind",
                        json!({"enum": reelforge_model::CastShadow::KINDS, "description": "A ready-made shadow to start from: drop (floating), ground (standing on a floor), long (a low sun), glow (a white light all round)."}),
                    ),
                    (
                        "color",
                        json!({"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 3, "maxItems": 4, "description": "Its colour as on screen: [r, g, b], each 0 to 1."}),
                    ),
                    ("opacity", within(0.0, 1.0, "How dark it is.")),
                    (
                        "x",
                        within(
                            -1.0,
                            1.0,
                            "How far right of the picture it lies, in shares of the picture's shorter side.",
                        ),
                    ),
                    ("y", within(-1.0, 1.0, "How far below.")),
                    ("blur", within(0.0, 0.5, "How soft it is, in shares of the picture's shorter side.")),
                    (
                        "flat",
                        within(
                            0.0,
                            1.0,
                            "How much of its height is pressed away towards the outline's lowest line: a shadow lying on the ground.",
                        ),
                    ),
                    (
                        "lean",
                        within(-1.0, 1.0, "How far its top is pushed sideways, in shares of the outline's height."),
                    ),
                ],
                &["clip"],
            )
        }
        "clip.setChromaKey" => {
            let unit = |doc: &str| json!({"type": "number", "minimum": 0, "maximum": 1, "description": doc});
            object(
                vec![
                    clip(),
                    (
                        "color",
                        json!({"description": "The colour that goes, as on screen: [r, g, b], each 0 to 1. null removes the key.", "oneOf": [{"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 3, "maxItems": 3}, {"type": "null"}]}),
                    ),
                    ("tolerance", unit("How far from the colour still goes; 0.2 at first.")),
                    ("softness", unit("How gradually what is further comes back; 0 is a hard edge.")),
                    ("spill", unit("How much of the colour's tint comes off what stays near it.")),
                ],
                &["clip"],
            )
        }
        "clip.setBlend" => object(
            vec![
                clip(),
                (
                    "mode",
                    json!({"enum": reelforge_model::BlendMode::ALL.map(reelforge_model::BlendMode::name), "description": "How the clip combines with the tracks below."}),
                ),
            ],
            &["clip", "mode"],
        ),
        "clip.setMask" => object(
            vec![
                clip(),
                (
                    "shape",
                    json!({"description": "The shape; a new one starts in the middle. null removes the mask.", "oneOf": [{"enum": reelforge_model::MaskKind::ALL.map(reelforge_model::MaskKind::name)}, {"type": "null"}]}),
                ),
                ("x", json!({"type": "number", "description": "Centre, as a fraction of the picture's width."})),
                ("y", json!({"type": "number", "description": "Centre, as a fraction of the picture's height."})),
                (
                    "width",
                    json!({"type": "number", "minimum": 0, "description": "In units of the picture's shorter side."}),
                ),
                (
                    "height",
                    json!({"type": "number", "minimum": 0, "description": "In units of the shorter side; a mirror's band thickness."}),
                ),
                ("rotation", json!({"type": "number", "description": "Clockwise turn, degrees."})),
                (
                    "feather",
                    json!({"type": "number", "minimum": 0, "description": "Soft edge, in units of the shorter side; 0 is hard."}),
                ),
                ("invert", json!({"type": "boolean", "description": "Show the outside instead."})),
                ("at", time("Where a value with keyframes is set: the key there changes or one is added.")),
            ],
            &["clip"],
        ),
        "clip.setReverse" => object(
            vec![
                clip(),
                ("reversed", json!({"type": "boolean", "description": "Play backwards; false plays forwards again."})),
            ],
            &["clip", "reversed"],
        ),
        "clip.setSpeedCurve" => object(
            vec![
                clip(),
                (
                    "curve",
                    json!({"description": "A preset, points of [fraction of the clip 0-1, multiple of the average speed 0.1-10] in order, or null for a steady speed.", "oneOf": [
                        {"enum": ["montage", "hero", "bullet", "jumpCut", "flashIn", "flashOut"]},
                        {"type": "array", "minItems": 2, "items": {"type": "array", "prefixItems": [{"type": "number", "minimum": 0, "maximum": 1}, {"type": "number", "minimum": 0.1, "maximum": 10}], "minItems": 2, "maxItems": 2}},
                        {"type": "null"}
                    ]}),
                ),
            ],
            &["clip", "curve"],
        ),
        "clip.freeze" => object(
            vec![
                clip(),
                ("at", time("Timeline time of the frame to hold, within the clip.")),
                ("duration", time("How long the still lasts; 3 s by default.")),
            ],
            &["clip", "at"],
        ),
        "clip.setTransition" => object(
            vec![
                clip(),
                (
                    "kind",
                    json!({"description": "Transition from the clip before; null removes it. wipe*: a soft edge travels that way and uncovers this clip. circle, box, diamond: it opens from the middle through that shape. flip, flipUp: the last clip turns away in space like a card with this one on its back. pageTurn: this clip is turned in from the right edge like a page. lightLeak: warm light drifts across as it fades in. glitch: the picture tears into bands, its colours apart, and cuts. zoomBlur: zooms into the last clip out of focus and back out of this one.", "oneOf": [{"enum": ["dissolve", "fadeBlack", "fadeWhite", "pushLeft", "pushRight", "pushUp", "pushDown", "slideLeft", "slideRight", "slideUp", "slideDown", "zoom", "wipeLeft", "wipeRight", "wipeUp", "wipeDown", "circle", "box", "diamond", "flip", "flipUp", "pageTurn", "lightLeak", "glitch", "zoomBlur"]}, {"type": "null"}]}),
                ),
                (
                    "duration",
                    time("How long it takes, centred on the cut; 0.5 s by default, at most the shorter clip."),
                ),
            ],
            &["clip", "kind"],
        ),
        "clip.addEffect" => object(
            vec![
                clip(),
                (
                    "effect",
                    json!({"type": "string", "description": "Effect id: color.brightness@1 (amount -1..1) or color.saturation@1 (amount, 1 unchanged)."}),
                ),
                (
                    "params",
                    json!({"type": "object", "additionalProperties": {"type": "number"}, "description": "Effect parameters by name."}),
                ),
            ],
            &["clip", "effect"],
        ),
        "clip.replace" => object(
            vec![
                clip(),
                ("media", ident("The file that takes the clip's place: a media id, such as m2.")),
                ("sourceIn", time("Where a video or a sound starts playing from; its start by default.")),
                (
                    "fit",
                    json!({
                        "enum": ["refuse", "slow"],
                        "description": "What to do with a video shorter than the clip: refuse (the default), or slow it down to fill.",
                    }),
                ),
            ],
            &["clip", "media"],
        ),
        "template.apply" => object(
            vec![
                (
                    "template",
                    json!({"enum": crate::timeline_templates().iter().map(|t| t.id).collect::<Vec<_>>(), "description": "A built-in template (see template.list)."}),
                ),
                (
                    "media",
                    json!({
                        "type": "array",
                        "items": {"type": ["string", "null"]},
                        "description": "Media ids for the slots, in order; null leaves a slot empty.",
                    }),
                ),
                (
                    "texts",
                    json!({
                        "type": "array",
                        "items": {"type": ["string", "null"]},
                        "description": "The titles' words, in order; null keeps a title's own.",
                    }),
                ),
                ("name", ident("The new timeline's name, when the template gets a timeline of its own.")),
            ],
            &["template"],
        ),
        "template.fill" => object(
            vec![
                (
                    "media",
                    json!({
                        "type": "array",
                        "items": {"type": ["string", "null"]},
                        "minItems": 1,
                        "description": "Media ids for the slots, in order; null skips a slot.",
                    }),
                ),
                (
                    "from",
                    json!({
                        "type": "integer",
                        "minimum": 1,
                        "description": "The first slot to fill, replacing what is in it and those after; without it only empty slots are filled.",
                    }),
                ),
                (
                    "fit",
                    json!({
                        "enum": ["slow", "refuse"],
                        "description": "What to do with a video shorter than its slot: slow it down to fill (the default), or refuse.",
                    }),
                ),
            ],
            &["media"],
        ),
        "template.mark" => object(
            vec![
                (
                    "clips",
                    json!({"description": "A clip id or a list of them: pictures on video tracks.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
                ),
                (
                    "slot",
                    json!({
                        "type": ["integer", "null"],
                        "minimum": 1,
                        "description": "The slot's number for all of them; null makes them ordinary clips again. Without it each gets the next free number.",
                    }),
                ),
            ],
            &["clips"],
        ),
        "edit.batch" => object(
            vec![
                (
                    "steps",
                    json!({
                        "type": "array",
                        "description": "Commands to run in order; if any fails, none applies. A parameter written \"${n.field}\" is what step n (from 0) replied there: \"${0.clip}\" is the clip step 0 added.",
                        "items": {"type": "object", "properties": {"id": {"type": "string", "description": "A command id, such as clip.add."}, "params": {"type": "object"}}, "required": ["id"]}
                    }),
                ),
                ("label", json!({"type": "string", "description": "Undo label."})),
            ],
            &["steps"],
        ),
        "render.compile" => object(
            vec![("encode", json!({"type": "boolean", "description": "Include the encoder settings (default true)."}))],
            &[],
        ),
        _ => return None,
    })
}
