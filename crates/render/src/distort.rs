//! The `distort.*` effects: where a layer's picture is read, instead of straight through.
//!
//! Both compositors read a layer's picture at a point of its visible part. With a distortion they
//! read it somewhere else ([`Warp::taps`]): rows slid along a sine, bands slid sideways, the
//! middle of a block, and red and blue apart. `composite.wgsl` does the same, step for step; the
//! bands come from whole-number arithmetic ([`hash`]), which is the same on every machine.

use reelforge_renderspec::effects;

/// The distortions of one layer at one instant.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Warp {
    /// `distort.wave@1`: amount, waves, phase.
    pub wave: [f32; 3],
    /// `distort.glitch@1`: amount, seed.
    pub glitch: [f32; 2],
    /// `distort.mosaic@1`: size.
    pub mosaic: f32,
    /// `distort.split@1`: x, y.
    pub split: [f32; 2],
}

/// A whole number from two, well mixed: the same in Rust and in WGSL.
pub fn hash(a: u32, b: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77).wrapping_add(0xC2B2_AE3D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}

impl Warp {
    /// The distortions among a layer's sampled effects; `None` when none changes anything.
    pub fn of(fx: &[(String, Vec<(String, f32)>)]) -> Option<Warp> {
        let get = |id: &str, name: &str, default: f32| {
            fx.iter()
                .find(|(i, _)| i == id)
                .and_then(|(_, params)| params.iter().find(|(k, _)| k == name))
                .map_or(default, |(_, v)| *v)
        };
        let w = Warp {
            wave: [
                get(effects::WAVE, "amount", 0.0),
                get(effects::WAVE, "waves", 4.0),
                get(effects::WAVE, "phase", 0.0),
            ],
            glitch: [get(effects::GLITCH, "amount", 0.0), get(effects::GLITCH, "seed", 0.0)],
            mosaic: get(effects::MOSAIC, "size", 0.0),
            split: [get(effects::SPLIT, "x", 0.0), get(effects::SPLIT, "y", 0.0)],
        };
        let rest = w.wave[0] == 0.0 && w.glitch[0] == 0.0 && w.mosaic <= 0.0 && w.split == [0.0, 0.0];
        (!rest).then_some(w)
    }

    /// Whether red, green and blue are read in three places.
    pub fn splits(&self) -> bool {
        self.split != [0.0, 0.0]
    }

    /// Where the picture is read for the point `uv` of a visible part `size` pixels big: for red,
    /// green and blue, each as fractions of the visible part.
    pub fn taps(&self, uv: [f32; 2], size: [f32; 2]) -> [[f32; 2]; 3] {
        let [mut u, mut v] = uv;
        let [amount, waves, phase] = self.wave;
        if amount != 0.0 {
            u += amount * (std::f32::consts::TAU * (v * waves + phase)).sin();
        }
        let [amount, seed] = self.glitch;
        if amount != 0.0 {
            let step = seed.max(0.0).floor() as u32;
            let bands = 6 + hash(step, 0) % 18;
            let band = ((v * bands as f32).floor().max(0.0) as u32).min(bands - 1);
            let h = hash(step, band + 1);
            // three bands in eight slide
            if h & 0xff < 96 {
                let slide = (((h >> 8) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0) * amount;
                u = (u + slide).rem_euclid(1.0);
            }
        }
        if self.mosaic > 0.0 {
            let side = self.mosaic * size[0].min(size[1]);
            let (bx, by) = (side / size[0], side / size[1]);
            u = ((u / bx).floor() + 0.5) * bx;
            v = ((v / by).floor() + 0.5) * by;
        }
        let [x, y] = self.split;
        [[u + x, v + y], [u, v], [u - x, v - y]]
    }

    /// The colour at `uv`: `read` gives the picture's at a point of the visible part, in its
    /// pixels. Points are held half a pixel inside its edges.
    pub fn sample(&self, uv: [f32; 2], size: [f32; 2], read: impl Fn(f32, f32) -> [f32; 4]) -> [f32; 4] {
        let at = |[u, v]: [f32; 2]| {
            read((u * size[0]).max(0.5).min(size[0] - 0.5), (v * size[1]).max(0.5).min(size[1] - 0.5))
        };
        let [r, g, b] = self.taps(uv, size);
        let green = at(g);
        if !self.splits() {
            return green;
        }
        let (red, blue) = (at(r), at(b));
        [red[0], green[1], blue[2], red[3].max(green[3]).max(blue[3])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(id: &str, params: &[(&str, f32)]) -> (String, Vec<(String, f32)>) {
        (id.to_string(), params.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }

    #[test]
    fn nothing_to_do_is_no_warp() {
        assert_eq!(Warp::of(&[]), None);
        assert_eq!(Warp::of(&[fx(effects::GRADE, &[("exposure", 1.0)])]), None);
        assert_eq!(Warp::of(&[fx(effects::SPLIT, &[]), fx(effects::MOSAIC, &[("size", 0.0)])]), None);
        let w = Warp::of(&[fx(effects::WAVE, &[("amount", 0.1)])]).expect("a wave");
        assert_eq!(w.wave, [0.1, 4.0, 0.0]);
        assert!(!w.splits());
    }

    #[test]
    fn each_distortion_reads_where_it_says() {
        let size = [200.0, 100.0];
        // red and blue apart, green where it was
        let w = Warp::of(&[fx(effects::SPLIT, &[("x", 0.05), ("y", -0.01)])]).unwrap();
        assert_eq!(w.taps([0.5, 0.5], size), [[0.55, 0.49], [0.5, 0.5], [0.45, 0.51]]);
        // a mosaic reads the middle of a square block: 10 pixels a side on either axis
        let w = Warp::of(&[fx(effects::MOSAIC, &[("size", 0.1)])]).unwrap();
        let [_, g, _] = w.taps([0.512, 0.37], size);
        assert!((g[0] * 200.0 - 105.0).abs() < 1e-3 && (g[1] * 100.0 - 35.0).abs() < 1e-3, "{g:?}");
        // a wave slides rows by up to its amount, none where the sine crosses zero
        let w = Warp::of(&[fx(effects::WAVE, &[("amount", 0.1), ("waves", 1.0)])]).unwrap();
        assert!((w.taps([0.5, 0.25], size)[1][0] - 0.6).abs() < 1e-6);
        assert!((w.taps([0.5, 0.5], size)[1][0] - 0.5).abs() < 1e-6);
        assert_eq!(w.taps([0.5, 0.25], size)[1][1], 0.25);
    }

    #[test]
    fn a_glitch_slides_some_bands_and_changes_with_its_seed() {
        let size = [100.0, 100.0];
        let slid = |seed: f32| -> Vec<f32> {
            let w = Warp::of(&[fx(effects::GLITCH, &[("amount", 0.2), ("seed", seed)])]).unwrap();
            (0..100).map(|y| w.taps([0.5, (y as f32 + 0.5) / 100.0], size)[1][0] - 0.5).collect()
        };
        let a = slid(3.0);
        let moved = a.iter().filter(|d| **d != 0.0).count();
        assert!(moved > 5 && moved < 80, "{moved} rows of 100 slid");
        assert!(a.iter().all(|d| d.abs() <= 0.2 + 1e-6));
        // rows of a band slide together
        assert!(a.windows(2).filter(|w| w[0] != w[1]).count() < 30);
        // the whole part of the seed picks the pattern
        assert_eq!(slid(3.9), a);
        assert_ne!(slid(4.0), a);
        // over many steps about three bands in eight slide
        let all: Vec<f32> = (0..200).flat_map(|s| slid(s as f32)).collect();
        let share = all.iter().filter(|d| **d != 0.0).count() as f32 / all.len() as f32;
        assert!((share - 0.375).abs() < 0.06, "{share}");
    }

    #[test]
    fn reads_are_held_inside_the_picture_and_split_by_channel() {
        let w = Warp::of(&[fx(effects::SPLIT, &[("x", 0.5)])]).unwrap();
        // the colour is where it was read: x in red, y in green
        let read = |x: f32, y: f32| [x, y, x, 1.0];
        let s = w.sample([0.75, 0.5], [100.0, 40.0], read);
        assert_eq!(s, [99.5, 20.0, 25.0, 1.0]);
    }
}
