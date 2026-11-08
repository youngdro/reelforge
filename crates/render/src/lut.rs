//! 3-D colour lookup tables (`color.lut@1`), read from the `.cube` format.
//!
//! A table maps sRGB-encoded colour to sRGB-encoded colour, as grading tools write them. The
//! compositors work in linear light, so a pixel is encoded, looked up (trilinear, between the
//! table's points) and decoded again.

use crate::image::{linear_to_srgb, srgb_to_linear};

/// A 3-D table: `size³` points, red changing fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    /// Points along each axis.
    pub size: usize,
    /// Output colours, `r + g·size + b·size²`.
    pub data: Vec<[f32; 3]>,
    /// The input colour at the first point.
    pub min: [f32; 3],
    /// The input colour at the last point.
    pub max: [f32; 3],
}

/// The largest table read: 65³ points is what grading tools write at most.
const MAX_SIZE: usize = 65;

impl Lut {
    /// Reads a `.cube` file (Adobe / Resolve): `LUT_3D_SIZE`, optional `DOMAIN_MIN` and
    /// `DOMAIN_MAX`, then the points. 1-D tables are refused.
    pub fn parse_cube(text: &str) -> Result<Lut, String> {
        let mut size = None;
        let (mut min, mut max) = ([0.0f32; 3], [1.0f32; 3]);
        let mut data = Vec::new();
        let three = |rest: &[&str], line: usize| -> Result<[f32; 3], String> {
            let v: Vec<f32> = rest
                .iter()
                .map(|w| w.parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("line {line}: not a number"))?;
            match v[..] {
                [a, b, c] if v.iter().all(|x| x.is_finite()) => Ok([a, b, c]),
                _ => Err(format!("line {line}: expected three numbers")),
            }
        };
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[0] {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err("1-D lookup tables are not supported; export a 3-D one".into()),
                "LUT_3D_SIZE" => {
                    let n: usize =
                        words.get(1).and_then(|w| w.parse().ok()).ok_or(format!("line {}: bad size", i + 1))?;
                    if !(2..=MAX_SIZE).contains(&n) {
                        return Err(format!("line {}: the size must be 2 to {MAX_SIZE}", i + 1));
                    }
                    size = Some(n);
                }
                "DOMAIN_MIN" => min = three(&words[1..], i + 1)?,
                "DOMAIN_MAX" => max = three(&words[1..], i + 1)?,
                w if w.starts_with(|c: char| c.is_ascii_alphabetic()) => {} // other keywords
                _ => data.push(three(&words, i + 1)?),
            }
        }
        let size = size.ok_or("no LUT_3D_SIZE line")?;
        if data.len() != size * size * size {
            return Err(format!("expected {} points, found {}", size * size * size, data.len()));
        }
        if (0..3).any(|c| max[c] <= min[c]) {
            return Err("DOMAIN_MAX must be above DOMAIN_MIN".into());
        }
        Ok(Lut { size, data, min, max })
    }

    /// The identity table with `size` points per axis.
    pub fn identity(size: usize) -> Lut {
        let k = (size - 1) as f32;
        let data = (0..size * size * size)
            .map(|i| [(i % size) as f32 / k, (i / size % size) as f32 / k, (i / size / size) as f32 / k])
            .collect();
        Lut { size, data, min: [0.0; 3], max: [1.0; 3] }
    }

    /// Looks up an encoded colour, between the points.
    pub fn lookup(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let k = (n - 1) as f32;
        let mut base = [0usize; 3];
        let mut frac = [0f32; 3];
        for i in 0..3 {
            let u = ((c[i] - self.min[i]) / (self.max[i] - self.min[i])).clamp(0.0, 1.0) * k;
            let b = (u.floor() as usize).min(n - 2);
            base[i] = b;
            frac[i] = u - b as f32;
        }
        let at = |r: usize, g: usize, b: usize| self.data[r + g * n + b * n * n];
        let mut out = [0f32; 3];
        for (dr, dg, db) in [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)] {
            let w = (if dr == 1 { frac[0] } else { 1.0 - frac[0] })
                * (if dg == 1 { frac[1] } else { 1.0 - frac[1] })
                * (if db == 1 { frac[2] } else { 1.0 - frac[2] });
            let v = at(base[0] + dr, base[1] + dg, base[2] + db);
            for c in 0..3 {
                out[c] += w * v[c];
            }
        }
        out
    }

    /// Applies the table to a premultiplied linear pixel, mixed in by `amount`.
    pub fn apply(&self, p: &mut [f32; 4], amount: f32) {
        let a = p[3];
        if a <= 0.0 {
            return;
        }
        let enc = [0, 1, 2].map(|c| linear_to_srgb((p[c] / a).clamp(0.0, 1.0)));
        let looked = self.lookup(enc);
        for c in 0..3 {
            let lin = srgb_to_linear(looked[c].clamp(0.0, 1.0)) * a;
            p[c] += (lin - p[c]) * amount;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_files_read_and_identity_changes_nothing() {
        let cube = "TITLE \"swap\"\n# red and blue swapped\nLUT_3D_SIZE 2\n\
            0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let lut = Lut::parse_cube(cube).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.lookup([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        let mid = lut.lookup([0.25, 0.5, 0.75]);
        assert!((mid[0] - 0.75).abs() < 1e-6 && (mid[2] - 0.25).abs() < 1e-6);
        let id = Lut::identity(17);
        let mut p = [0.2, 0.05, 0.6, 0.8];
        let before = p;
        id.apply(&mut p, 1.0);
        assert!(p.iter().zip(before).all(|(a, b)| (a - b).abs() < 1e-4), "{p:?}");
        assert!(Lut::parse_cube("LUT_1D_SIZE 4\n").is_err());
        assert!(Lut::parse_cube("LUT_3D_SIZE 2\n0 0 0\n").unwrap_err().contains("expected 8"));
    }
}
