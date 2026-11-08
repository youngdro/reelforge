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
            "template",
            json!({"type": "string", "description": "A title template id from title.templates (classic, variety, sticker, candy, fire, neon, retro, soft, boxed, label); other parameters given override it."}),
        ),
    ]
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
        "commands.list" | "project.inspect" | "edit.undo" | "edit.redo" | "title.templates" => object(vec![], &[]),
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
        "media.import" => object(
            vec![
                (
                    "path",
                    json!({"type": "string", "description": "File path or URI of the media, as the renderer will reach it."}),
                ),
                (
                    "kind",
                    json!({"enum": ["video", "image", "audio"], "description": "Measured from the file when it is reachable."}),
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
            ];
            props.extend(title_props());
            object(props, &["track"])
        }
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
            ],
            &["srt"],
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
        "clip.remove" => object(
            vec![(
                "clips",
                json!({"description": "A clip id or a list of them.", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}),
            )],
            &["clips"],
        ),
        "clip.setSpeed" => object(
            vec![
                clip(),
                (
                    "speed",
                    json!({"type": "number", "exclusiveMinimum": 0, "description": "Playback speed; 2 is twice as fast."}),
                ),
            ],
            &["clip", "speed"],
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
            ];
            props.extend(title_props());
            object(props, &["clip"])
        }
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
