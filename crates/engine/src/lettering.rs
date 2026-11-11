//! The title style parameters for what letters wear beyond a colour, an outline and a box: `fill`,
//! `palette`, `shadows`, `depth`, `hollow`, `bevel`, `scatter`, `arc`, `backing`, `border`, `skew`.

use reelforge_model::{Backing, Bevel, Depth, Outline, Palette, PaletteUnit, Scatter, Shadow, TitleFill, TitleStyle};
use serde_json::Value;

use crate::EngineError;
use crate::params::P;

fn wrong(what: &str) -> EngineError {
    EngineError::Params(what.into())
}

/// The colours in a list under `name` of `o`, or `o` itself when it is the list.
fn colors(p: &P, v: &Value, name: &str) -> Result<Option<Vec<[f32; 4]>>, EngineError> {
    let list = if v.is_array() { Some(v) } else { v.get("colors") };
    let Some(list) = list else { return Ok(None) };
    let bad = || wrong(&format!("`{name}` takes a list of colours, each [r, g, b] or [r, g, b, a] from 0 to 1"));
    let items = list.as_array().ok_or_else(bad)?;
    let mut out = vec![];
    for item in items {
        let holder = serde_json::json!({ "c": item });
        out.push(P::new(&holder, p.fps).opt_color("c").map_err(|_| bad())?.ok_or_else(bad)?);
    }
    Ok(Some(out))
}

fn pair(v: Option<&Value>, name: &str) -> Result<Option<[f32; 2]>, EngineError> {
    v.map(|v| {
        serde_json::from_value::<[f32; 2]>(v.clone())
            .map_err(|_| wrong(&format!("`{name}` must be [right, down] in ems")))
    })
    .transpose()
}

fn number(o: &P, name: &str) -> Result<Option<f32>, EngineError> {
    Ok(o.opt_f64(name)?.map(|v| v as f32))
}

/// `st` with these parameters in `p` applied: each replaces what was there, keeping
/// what it does not mention, and `null` takes it away.
pub(crate) fn wear(p: &P, st: &mut TitleStyle) -> Result<(), EngineError> {
    match p.v.get("fill") {
        Some(Value::Null) => st.fill = None,
        Some(v) => {
            let o = P::new(v, p.fps);
            let was = st.fill.clone();
            let colors = match colors(p, v, "fill")? {
                Some(c) => c,
                None => was.as_ref().map(|f| f.colors.clone()).ok_or_else(|| wrong("`fill` needs `colors`"))?,
            };
            let was = was.unwrap_or_else(|| TitleFill::down(&[]));
            st.fill = Some(TitleFill {
                colors,
                angle: number(&o, "angle")?.unwrap_or(was.angle),
                lines: o.opt_bool("lines")?.unwrap_or(was.lines),
                bands: o.opt_bool("bands")?.unwrap_or(was.bands),
                repeat: number(&o, "repeat")?.unwrap_or(was.repeat),
            });
            // one fill or the other
            st.gradient = None;
        }
        None => {}
    }
    match p.v.get("palette") {
        Some(Value::Null) => st.palette = None,
        Some(v) => {
            let o = P::new(v, p.fps);
            let was = st.palette.clone();
            let colors = match colors(p, v, "palette")? {
                Some(c) => c,
                None => was.as_ref().map(|f| f.colors.clone()).ok_or_else(|| wrong("`palette` needs `colors`"))?,
            };
            let unit = match o.opt_str("unit")? {
                Some(u) => PaletteUnit::parse(u).ok_or_else(|| wrong("`palette.unit` must be char, word or line"))?,
                None => was.map(|p| p.unit).unwrap_or_default(),
            };
            st.palette = Some(Palette { colors, unit });
        }
        None => {}
    }
    match p.v.get("shadows") {
        Some(Value::Null) => st.shadows.clear(),
        Some(Value::Array(list)) => {
            let mut out = vec![];
            for v in list {
                let o = P::new(v, p.fps);
                out.push(Shadow {
                    color: o.opt_color("color")?.unwrap_or([0.0, 0.0, 0.0, 0.6]),
                    offset: pair(o.opt("offset"), "shadows.offset")?.unwrap_or([0.0, 0.0]),
                    blur: number(&o, "blur")?.unwrap_or(0.1),
                });
            }
            st.shadows = out;
        }
        Some(_) => return Err(wrong("`shadows` must be a list of {color, offset, blur} or null")),
        None => {}
    }
    match p.v.get("depth") {
        Some(Value::Null) => st.depth = None,
        Some(v) if v.is_object() => {
            let o = P::new(v, p.fps);
            let was =
                st.depth.clone().unwrap_or(Depth { color: [0.0, 0.0, 0.0, 1.0], offset: [0.06, 0.06], fade: None });
            st.depth = Some(Depth {
                color: o.opt_color("color")?.unwrap_or(was.color),
                offset: pair(o.opt("offset"), "depth.offset")?.unwrap_or(was.offset),
                fade: o.opt_color_or_null("fade")?.unwrap_or(was.fade),
            });
        }
        Some(_) => return Err(wrong("`depth` must be {color, offset, fade} or null")),
        None => {}
    }
    match p.v.get("hollow") {
        Some(Value::Null) => st.hollow = None,
        Some(_) => st.hollow = number(p, "hollow")?.filter(|w| *w != 0.0),
        None => {}
    }
    match p.v.get("bevel") {
        Some(Value::Null) => st.bevel = None,
        Some(v) if v.is_object() => {
            let o = P::new(v, p.fps);
            let was = st.bevel.clone().unwrap_or(Bevel {
                light: [1.0, 1.0, 1.0, 0.8],
                dark: [0.0, 0.0, 0.0, 0.5],
                size: 0.05,
                angle: 315.0,
            });
            st.bevel = Some(Bevel {
                light: o.opt_color("light")?.unwrap_or(was.light),
                dark: o.opt_color("dark")?.unwrap_or(was.dark),
                size: number(&o, "size")?.unwrap_or(was.size),
                angle: number(&o, "angle")?.unwrap_or(was.angle),
            });
        }
        Some(_) => return Err(wrong("`bevel` must be {light, dark, size, angle} or null")),
        None => {}
    }
    match p.v.get("scatter") {
        Some(Value::Null) => st.scatter = None,
        Some(v) if v.is_object() => {
            let o = P::new(v, p.fps);
            let was = st.scatter.clone().unwrap_or(Scatter { rotate: 0.0, shift: 0.0, scale: 0.0, seed: 0 });
            st.scatter = Some(Scatter {
                rotate: number(&o, "rotate")?.unwrap_or(was.rotate),
                shift: number(&o, "shift")?.unwrap_or(was.shift),
                scale: number(&o, "scale")?.unwrap_or(was.scale),
                seed: o.opt_u32("seed")?.unwrap_or(was.seed),
            });
        }
        Some(_) => return Err(wrong("`scatter` must be {rotate, shift, scale, seed} or null")),
        None => {}
    }
    match p.v.get("arc") {
        Some(Value::Null) => st.arc = 0.0,
        Some(_) => st.arc = number(p, "arc")?.unwrap_or(0.0),
        None => {}
    }
    if let Some(b) = p.opt_str("backing")? {
        st.backing = Backing::parse(b).ok_or_else(|| wrong("`backing` must be box, marker or tiles"))?;
    }
    match p.v.get("border") {
        Some(Value::Null) => st.border = None,
        Some(v) if v.is_object() => {
            let o = P::new(v, p.fps);
            let was = st.border.clone().unwrap_or(Outline { color: [1.0; 4], width: 0.04 });
            st.border = Some(Outline {
                color: o.opt_color("color")?.unwrap_or(was.color),
                width: number(&o, "width")?.unwrap_or(was.width),
            });
        }
        Some(_) => return Err(wrong("`border` must be {color, width} or null")),
        None => {}
    }
    match p.v.get("skew") {
        Some(Value::Null) => st.skew = 0.0,
        Some(_) => st.skew = number(p, "skew")?.unwrap_or(0.0),
        None => {}
    }
    match st.problem() {
        Some(why) => Err(EngineError::Params(why.into())),
        None => Ok(()),
    }
}
