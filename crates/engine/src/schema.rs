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
            "template",
            json!({"type": "string", "description": "A title template id from title.templates (classic, variety, sticker, candy, fire, neon, retro, soft, boxed, label); other parameters given override it."}),
        ),
    ]
}

fn prop_schema() -> (&'static str, Value) {
    (
        "prop",
        json!({"enum": reelforge_model::Prop::ALL.map(reelforge_model::Prop::name), "description": "The property that moves: placement, opacity and volume; the crop's edges; the mask's centre, size, turn and soft edge; the colour adjustments."}),
    )
}

fn ease_schema() -> (&'static str, Value) {
    (
        "ease",
        json!({"enum": ["linear", "hold", "inOut"], "description": "How the value moves on to the next keyframe: a straight line, a jump at the next key, or slow out and in."}),
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

fn object(props: Vec<(&'static str, Value)>, required: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in props {
        p.insert(k.to_string(), v);
    }
    json!({"type": "object", "properties": p, "required": required, "additionalProperties": false})
}

/// The JSON Schema of command `id`'s parameters, or `None` for an unknown id.
pub fn input_schema(id: &str) -> Option<Value> {
    let clip = || ("clip", ident("Clip id, such as c4."));
    Some(match id {
        "commands.list" | "project.inspect" | "project.new" | "edit.undo" | "edit.redo" | "title.templates"
        | "sound.library" | "timeline.list" | "fx.list" | "sticker.library" | "voice.list" => object(vec![], &[]),
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
                ("width", json!({"type": "integer", "minimum": 16, "description": "Output width in pixels."})),
                ("height", json!({"type": "integer", "minimum": 16, "description": "Output height in pixels."})),
                (
                    "fps",
                    json!({"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 2, "maxItems": 2, "description": "Frame rate as [numerator, denominator], such as [30, 1] or [30000, 1001]."}),
                ),
                ("background", color("Canvas background.")),
            ],
            &[],
        ),
        "project.setExport" => {
            use reelforge_model::{Codec, Quality, Resolution, export};
            object(
                vec![
                    (
                        "preset",
                        json!({"enum": export::PRESETS.map(|(id, _)| id), "description": "Ready-made settings, applied before the others: draft (720p 30 fps, low), standard (1080p 30 fps), smooth (1080p 60 fps, high), 4k (2160p HEVC, high)."}),
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
                    json!({"enum": ["video", "image", "audio", "lut", "font"], "description": "Measured from the file when it is reachable."}),
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
        "media.remove" => object(vec![("media", ident("Media id, such as m1."))], &["media"]),
        "timeline.remove" => object(vec![("timeline", ident("Timeline id; the current one by default."))], &[]),
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
            let mut props = vec![
                ("track", ident("Track id, such as t1 (project.inspect lists them).")),
                ("start", time("Timeline start; the end of the track by default.")),
                ("duration", time("Timeline length; the media's remaining length, or 3 s, by default.")),
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
                ("fx", fx("Picture effects the clip starts with.")),
            ];
            props.extend(title_props());
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
        "clip.setAnimation" => object(
            vec![
                clip(),
                (
                    "slot",
                    json!({"enum": ["in", "out"], "description": "Plays from the clip's start (in) or up to its end (out)."}),
                ),
                (
                    "kind",
                    json!({"description": "Preset; null removes the animation. typewriter (titles only) types the words in, or erases them as the title leaves.", "oneOf": [{"enum": ["fade", "slideUp", "slideDown", "slideLeft", "slideRight", "zoomIn", "zoomOut", "pop", "spin", "typewriter"]}, {"type": "null"}]}),
                ),
                ("duration", time("How long it plays; 0.5 s by default.")),
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
                    json!({"description": "Transition from the clip before; null removes it.", "oneOf": [{"enum": ["dissolve", "fadeBlack", "fadeWhite", "pushLeft", "pushRight", "pushUp", "pushDown", "slideLeft", "slideRight", "slideUp", "slideDown", "zoom"]}, {"type": "null"}]}),
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
        "edit.batch" => object(
            vec![
                (
                    "steps",
                    json!({
                        "type": "array",
                        "description": "Commands to run in order; if any fails, none applies.",
                        "items": {"type": "object", "properties": {"id": {"type": "string"}, "params": {"type": "object"}}, "required": ["id"]}
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
