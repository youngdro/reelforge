//! `shadow.drop@1`: the shadow a layer's picture casts, drawn under it.
//!
//! A shadow is the picture's outline as it shows (what its keys, crop and mask leave of it),
//! filled with one colour, softened and moved aside. The outline is taken from the picture on a
//! coarse grid, as a shadow is soft; pressed towards its lowest line and pushed sideways for one
//! that lies on the ground; blurred; and handed to the compositor as a sprite placed where the
//! layer is, so it moves, turns and is smeared with the layer. CPU and GPU draw the same sprite.

use std::sync::{Arc, Mutex, Weak};

use reelforge_renderspec::effects;

use crate::plan::{Map, PlannedLayer, is_affine, then};
use crate::text::Sprite;
use crate::{Image, image::srgb_to_linear};

/// The longer side of the grid an outline is taken on, at most.
const GRID: f32 = 512.0;
/// And at least, however soft the shadow.
const COARSE: f32 = 96.0;
/// Shadows kept at most: a paused preview draws the same one again and again.
const KEPT: usize = 8;

/// A shadow kept: the picture it was cast by, everything else that shaped it, the sprite's
/// picture and where the outline's grid lies in it.
type Kept = (Weak<Image>, String, Arc<Image>, Lie);

/// Where an outline's grid lies in a shadow's picture: the grid's first cell is `pad` cells in,
/// and each cell is `step` pixels of the layer's visible picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lie {
    pad: [f32; 2],
    step: f32,
}

static CAST: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// What a layer's `shadow.drop@1` asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shadow {
    pub color: [f32; 3],
    pub opacity: f32,
    pub offset: [f32; 2],
    pub blur: f32,
    pub flat: f32,
    pub lean: f32,
}

impl Shadow {
    /// The shadow `layer` casts, if it casts one.
    pub(crate) fn of(layer: &PlannedLayer) -> Option<Shadow> {
        let (_, params) = layer.fx.iter().find(|(id, _)| id == effects::SHADOW)?;
        let get = |name: &str, default: f32| params.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v);
        let shadow = Shadow {
            color: [get("r", 0.0), get("g", 0.0), get("b", 0.0)].map(|v| v.clamp(0.0, 1.0)),
            opacity: get("opacity", 0.5).clamp(0.0, 1.0),
            offset: [get("x", 0.02), get("y", 0.03)].map(|v| v.clamp(-1.0, 1.0)),
            blur: get("blur", 0.03).clamp(0.0, 0.5),
            flat: get("flat", 0.0).clamp(0.0, 0.98),
            lean: get("lean", 0.0).clamp(-1.0, 1.0),
        };
        (shadow.opacity > 0.0).then_some(shadow)
    }
}

/// How much of `layer`'s picture shows at a place of its visible part (`cw` by `ch` pixels):
/// the picture's own alpha there, less what a keyed colour and a mask take.
fn shows(layer: &PlannedLayer, pic: &Image, at: [f32; 2], [cw, ch]: [f32; 2]) -> f32 {
    let (px, py) = layer.in_picture(at[0], at[1], cw, ch, pic.width as f32, pic.height as f32);
    let mut p = pic.sample(px, py);
    for (id, params) in &layer.fx {
        if id == effects::CHROMA_KEY {
            let get = |name: &str, default: f32| params.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v);
            crate::cpu::chroma_key(
                &mut p,
                [get("r", 0.0), get("g", 1.0), get("b", 0.0)],
                [get("tolerance", 0.3), get("softness", 0.1), get("spill", 0.5)],
            );
        }
    }
    p[3] * layer.shape.map_or(1.0, |m| m.coverage([at[0] / cw, at[1] / ch], cw, ch))
}

/// The outline of `layer`'s picture as a shadow: a grid of how dark each cell is, `w` across,
/// and where the picture's own grid lies in it.
fn outline(layer: &PlannedLayer, pic: &Image, shadow: &Shadow) -> Option<(Vec<f32>, usize, Lie)> {
    let (cw, ch) = layer.visible(pic.width as f32, pic.height as f32);
    let long = cw.max(ch);
    // a soft shadow needs few cells: about three to its blur
    let sigma = shadow.blur * cw.min(ch);
    let step = (sigma / 3.0).max(long / GRID).min(long / COARSE).max(1.0);
    let (gw, gh) = ((cw / step).ceil() as usize, (ch / step).ceil() as usize);
    if gw == 0 || gh == 0 || gw * gh > 1 << 22 {
        return None;
    }
    let mut grid = vec![0.0f32; gw * gh];
    for (i, cell) in grid.iter_mut().enumerate() {
        let (x, y) = ((i % gw) as f32, (i / gw) as f32);
        // four places in the cell
        let mut sum = 0.0;
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let at = [((x + dx) * step).min(cw - 0.01), ((y + dy) * step).min(ch - 0.01)];
            sum += shows(layer, pic, at, [cw, ch]);
        }
        *cell = (sum / 4.0).clamp(0.0, 1.0);
    }
    // the ground: under the lowest row something stands in
    let stands = |row: &[f32]| row.iter().any(|a| *a > 0.5);
    let ground = grid.chunks(gw).rposition(stands).map(|y| y + 1)?;
    let top = grid.chunks(gw).position(stands).unwrap_or(0);
    let lying = shadow.flat > 0.0 || shadow.lean != 0.0;
    let sigma = sigma / step;
    let reach = (sigma * 3.0).ceil() as usize + 1;
    let aside = if lying { (shadow.lean.abs() * (ground - top) as f32).ceil() as usize } else { 0 };
    let pad = [reach + aside, reach];
    let (w, h) = (gw + 2 * pad[0], gh + 2 * pad[1]);
    let read = |x: f32, y: f32| -> f32 {
        // between the cells' middles, nothing beyond the grid
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let at = |x: f32, y: f32| {
            if x < 0.0 || y < 0.0 || x >= gw as f32 || y >= gh as f32 {
                0.0
            } else {
                grid[y as usize * gw + x as usize]
            }
        };
        let (fx, fy) = (x - x0, y - y0);
        let (a, b) = (
            at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * fx,
            at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * fx,
        );
        a + (b - a) * fy
    };
    let keep = 1.0 - shadow.flat;
    let mut out = vec![0.0f32; w * h];
    for (i, cell) in out.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32 + 0.5 - pad[0] as f32, (i / w) as f32 + 0.5 - pad[1] as f32);
        *cell = if lying {
            // how far above the ground this place of the shadow is, and the place of the
            // outline that falls here: that much higher, and not pushed aside
            let up = ground as f32 - y;
            if up < 0.0 {
                0.0
            } else {
                let high = up / keep;
                read(x - shadow.lean * high, ground as f32 - high)
            }
        } else {
            read(x, y)
        };
    }
    if sigma > 0.3 {
        soften(&mut out, w, h, sigma);
    }
    Some((out, w, Lie { pad: [pad[0] as f32, pad[1] as f32], step }))
}

/// A Gaussian blur of `v` (`w` across), deviation `sigma` cells, nothing beyond its edges.
fn soften(v: &mut [f32], w: usize, h: usize, sigma: f32) {
    let r = (sigma * 3.0).ceil() as isize;
    let weights: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = weights.iter().sum();
    let pass = |from: &[f32], to: &mut [f32], len: usize, lines: usize, along: usize, across: usize| {
        for line in 0..lines {
            for i in 0..len {
                let mut sum = 0.0;
                for (k, weight) in weights.iter().enumerate() {
                    let j = i as isize + k as isize - r;
                    if j >= 0 && (j as usize) < len {
                        sum += from[line * across + j as usize * along] * weight;
                    }
                }
                to[line * across + i * along] = sum / total;
            }
        }
    };
    let mut rows = vec![0.0f32; v.len()];
    pass(v, &mut rows, w, h, 1, w);
    pass(&rows, v, h, w, w, 1);
}

/// The shadow `of` casts with picture `pic`, as a sprite for a frame of `target` pixels: placed
/// where the layer's picture is and moved aside. `None` when nothing of the picture shows.
pub(crate) fn cast(of: &PlannedLayer, pic: &Arc<Image>, target: [f32; 2]) -> Option<Sprite> {
    let shadow = Shadow::of(of)?;
    // everything but the picture that shapes the outline
    let chroma: Vec<_> = of.fx.iter().filter(|(id, _)| id == effects::CHROMA_KEY).collect();
    let shaped = format!(
        "{:?} {:?} {:?} {:?} {:?} {} {} {}",
        of.crop, of.flip, of.shape, chroma, shadow.color, shadow.blur, shadow.flat, shadow.lean
    );
    let kept = |cast: &Vec<Kept>| {
        cast.iter()
            .find(|(from, as_, _, _)| *as_ == shaped && std::ptr::eq(from.as_ptr(), Arc::as_ptr(pic)))
            .map(|(_, _, image, lie)| (image.clone(), *lie))
    };
    let found = kept(&CAST.lock().unwrap_or_else(|e| e.into_inner()));
    let (image, lie) = match found {
        Some(found) => found,
        None => {
            let (grid, w, lie) = outline(of, pic, &shadow)?;
            let [r, g, b] = shadow.color.map(srgb_to_linear);
            let pixels = grid.iter().map(|a| [r * a, g * a, b * a, *a]).collect();
            let image = Arc::new(Image { width: w as u32, height: (grid.len() / w) as u32, pixels });
            let mut cast = CAST.lock().unwrap_or_else(|e| e.into_inner());
            cast.retain(|(from, _, _, _)| from.strong_count() > 0);
            cast.push((Arc::downgrade(pic), shaped, image.clone(), lie));
            while cast.len() > KEPT {
                cast.remove(0);
            }
            (image, lie)
        }
    };
    let (cw, ch) = of.visible(pic.width as f32, pic.height as f32);
    // the sprite's cells to the visible picture's pixels
    let cells: Map = [lie.step, 0.0, -lie.pad[0] * lie.step, 0.0, lie.step, -lie.pad[1] * lie.step, 0.0, 0.0, 1.0];
    let placed = |m: &Map| {
        // aside by shares of the picture's shorter side as it is placed
        let per = if is_affine(m) {
            m[0].hypot(m[3]).max(m[1].hypot(m[4]))
        } else {
            of.spread(cw, ch, target).unwrap_or(0.0)
        };
        let by = shadow.offset.map(|v| v * cw.min(ch) * per);
        then(&[1.0, 0.0, by[0], 0.0, 1.0, by[1], 0.0, 0.0, 1.0], &then(m, &cells))
    };
    Some(Sprite {
        x: 0,
        y: 0,
        image,
        map: Some(placed(&of.matrix(cw, ch, target))),
        smear: of.smears(cw, ch, target).iter().map(placed).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PlannedSource, apply};

    fn layer(fx: Vec<(String, Vec<(String, f32)>)>) -> PlannedLayer {
        PlannedLayer {
            id: "l".into(),
            source: PlannedSource::Solid([0.0; 4]),
            fit: reelforge_renderspec::Fit::Contain,
            center: [200.0, 100.0],
            scale: 0.5,
            rotation: 0.0,
            opacity: 1.0,
            blend: reelforge_renderspec::Blend::Normal,
            fx,
            lut: None,
            crop: [0.0; 4],
            flip: [false; 2],
            shape: None,
            stretch: [1.0, 1.0],
            anchor: [0.5, 0.5],
            turn: None,
            carried: Vec::new(),
            frame: [400.0, 200.0],
            smear: Vec::new(),
        }
    }

    fn shadow(params: &[(&str, f32)]) -> (String, Vec<(String, f32)>) {
        (effects::SHADOW.to_string(), params.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }

    /// A see-through picture with a box standing in its middle.
    fn boxed() -> Arc<Image> {
        let mut img = Image::filled(200, 100, [0.0; 4]);
        for y in 30..80 {
            for x in 80..120 {
                img.pixels[y * 200 + x] = [0.2, 0.4, 0.6, 1.0];
            }
        }
        Arc::new(img)
    }

    /// How dark the shadow is at a place of the frame.
    fn dark(sp: &Sprite, x: f32, y: f32) -> f32 {
        let inv = crate::plan::invert(sp.map.expect("placed")).expect("seen");
        let [sx, sy] = apply(&inv, x, y).expect("in front");
        if sx < 0.0 || sy < 0.0 || sx >= sp.image.width as f32 || sy >= sp.image.height as f32 {
            return 0.0;
        }
        sp.image.sample(sx, sy)[3]
    }

    #[test]
    fn a_shadow_is_the_outline_softened_and_moved_aside() {
        let pic = boxed();
        // the picture fills the frame's height at scale 1; at half that it is 200 × 100 at
        // (100, 50), the box at 180..220 × 80..130
        let sharp = layer(vec![shadow(&[("blur", 0.0), ("x", 0.1), ("y", 0.2), ("r", 1.0)])]);
        let sp = cast(&sharp, &pic, [400.0, 200.0]).expect("a shadow");
        // aside by shares of the placed picture's shorter side (100): 10 right, 20 down
        assert!(dark(&sp, 200.0, 90.0) < 0.01 && dark(&sp, 200.0, 105.0) > 0.99);
        assert!(dark(&sp, 185.0, 120.0) < 0.01 && dark(&sp, 195.0, 120.0) > 0.99);
        assert!(dark(&sp, 225.0, 145.0) > 0.99 && dark(&sp, 235.0, 145.0) < 0.01 && dark(&sp, 225.0, 155.0) < 0.01);
        // its colour, multiplied by how dark it is
        let p = sp.image.pixels.iter().find(|p| p[3] > 0.99).expect("dark somewhere");
        assert_eq!(*p, [1.0, 0.0, 0.0, 1.0]);
        // the same picture asked again: the one made before
        assert!(Arc::ptr_eq(&sp.image, &cast(&sharp, &pic, [400.0, 200.0]).expect("kept").image));

        // soft: half dark at the outline, fading both ways, and as much darkness in all
        let soft = layer(vec![shadow(&[("blur", 0.05), ("x", 0.0), ("y", 0.0)])]);
        let sp = cast(&soft, &pic, [400.0, 200.0]).expect("a shadow");
        let edge = dark(&sp, 180.0, 105.0);
        assert!((edge - 0.5).abs() < 0.06, "{edge}");
        assert!(dark(&sp, 172.0, 105.0) > 0.02 && dark(&sp, 172.0, 105.0) < 0.2 && dark(&sp, 200.0, 105.0) > 0.95);
        assert!(dark(&sp, 150.0, 105.0) < 0.01);

        // lying on the ground: pressed towards the box's foot, and leaning
        let lying = layer(vec![shadow(&[("blur", 0.0), ("x", 0.0), ("y", 0.0), ("flat", 0.8), ("lean", 0.5)])]);
        let sp = cast(&lying, &pic, [400.0, 200.0]).expect("a shadow");
        // the foot stays where it is; a fifth of the height (10) is left above it
        assert!(dark(&sp, 200.0, 128.0) > 0.99 && dark(&sp, 200.0, 135.0) < 0.01);
        assert!(dark(&sp, 215.0, 115.0) < 0.01);
        // its top is pushed half the box's height (25) to the right
        assert!(dark(&sp, 240.0, 121.5) > 0.9 && dark(&sp, 190.0, 121.5) < 0.1, "{}", dark(&sp, 240.0, 121.5));

        // nothing shows, no shadow; no opacity, none either
        assert!(cast(&sharp, &Arc::new(Image::filled(8, 8, [0.0; 4])), [400.0, 200.0]).is_none());
        assert!(cast(&layer(vec![shadow(&[("opacity", 0.0)])]), &pic, [400.0, 200.0]).is_none());
        assert!(cast(&layer(vec![]), &pic, [400.0, 200.0]).is_none());
    }

    #[test]
    fn a_shadow_is_of_what_the_crop_the_mask_and_a_keyed_colour_leave() {
        // green all over with the box on it: keyed, only the box casts
        let mut img = Image::filled(200, 100, [0.0, 1.0, 0.0, 1.0]);
        for y in 30..80 {
            for x in 80..120 {
                img.pixels[y * 200 + x] = [0.6, 0.1, 0.1, 1.0];
            }
        }
        let pic = Arc::new(img);
        let sharp = shadow(&[("blur", 0.0), ("x", 0.0), ("y", 0.0)]);
        let whole = cast(&layer(vec![sharp.clone()]), &pic, [400.0, 200.0]).expect("a shadow");
        assert!(dark(&whole, 110.0, 60.0) > 0.99);
        let keyed = layer(vec![(effects::CHROMA_KEY.to_string(), vec![]), sharp.clone()]);
        let sp = cast(&keyed, &pic, [400.0, 200.0]).expect("a shadow");
        assert!(dark(&sp, 110.0, 60.0) < 0.01 && dark(&sp, 200.0, 105.0) > 0.99);
        // cropped to its right half, the picture is placed anew and the shadow with it: all
        // of what is left casts (it is not keyed), 100 × 100 placed at 150..250
        let mut cropped = layer(vec![sharp.clone()]);
        cropped.crop = [0.5, 0.0, 0.0, 0.0];
        let sp = cast(&cropped, &pic, [400.0, 200.0]).expect("a shadow");
        assert!(dark(&sp, 160.0, 105.0) > 0.99 && dark(&sp, 240.0, 105.0) > 0.99 && dark(&sp, 140.0, 105.0) < 0.01);
        // behind a round mask it is round
        let mut masked = layer(vec![sharp]);
        masked.shape = Some(reelforge_renderspec::ShapeMask {
            shape: reelforge_renderspec::MaskShape::Circle,
            center: [0.5, 0.5],
            size: [0.6, 0.6],
            rotation: 0.0,
            feather: 0.0,
            invert: false,
        });
        let sp = cast(&masked, &pic, [400.0, 200.0]).expect("a shadow");
        assert!(dark(&sp, 200.0, 100.0) > 0.99 && dark(&sp, 200.0, 75.0) > 0.99);
        assert!(dark(&sp, 174.0, 74.0) < 0.01 && dark(&sp, 200.0, 135.0) < 0.01);
    }
}
