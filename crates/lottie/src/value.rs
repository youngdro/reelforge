//! Animated values: numbers, points, colours and paths that are either fixed or keyed, each key
//! easing into the next along its own cubic Bézier, and points along a path in space.

use serde_json::Value;

use crate::geom::{Bez, P};

/// A number out of JSON, where Lottie writes one as a number or as a list of one.
pub(crate) fn number(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::Array(a) => a.first().and_then(number),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn numbers(v: &Value) -> Vec<f32> {
    match v {
        Value::Array(a) => a.iter().filter_map(number).collect(),
        other => number(other).into_iter().collect(),
    }
}

/// One key of an animated value.
#[derive(Clone, Debug)]
pub(crate) struct Key<T> {
    /// The frame it is at.
    t: f32,
    /// The value there, and where it has got to by the next key.
    from: T,
    to: T,
    /// It stays at `from` until the next key.
    hold: bool,
    /// The easing curve's two handles, one pair for each dimension or one for all.
    out: Vec<P>,
    into: Vec<P>,
    /// The handles of the path a point travels along, when it has one.
    path: Option<(Vec<f32>, Vec<f32>)>,
}

/// A value that may change over the frames.
#[derive(Clone, Debug)]
pub(crate) enum Animated<T> {
    Fixed(T),
    Keyed(Vec<Key<T>>, T),
}

fn handles(v: Option<&Value>) -> Vec<P> {
    let Some(v) = v else { return Vec::new() };
    let (xs, ys) = (v.get("x").map(numbers).unwrap_or_default(), v.get("y").map(numbers).unwrap_or_default());
    (0..xs.len().min(ys.len()).max(1))
        .filter_map(|k| Some([*xs.get(k).or(xs.last())?, *ys.get(k).or(ys.last())?]))
        .collect()
}

/// The eased progress for a progress `x` along a cubic Bézier from (0, 0) to (1, 1) with
/// handles `p1` and `p2`.
fn ease(p1: P, p2: P, x: f32) -> f32 {
    if (p1[0] == p1[1] && p2[0] == p2[1]) || x <= 0.0 || x >= 1.0 {
        return x.clamp(0.0, 1.0);
    }
    let curve = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let (x1, x2) = (p1[0].clamp(0.0, 1.0), p2[0].clamp(0.0, 1.0));
    let (mut lo, mut hi, mut t) = (0.0f32, 1.0f32, x);
    for _ in 0..24 {
        let at = curve(x1, x2, t);
        if (at - x).abs() < 1e-5 {
            break;
        }
        if at < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    curve(p1[1], p2[1], t)
}

/// Places a value on its path in space (the handles out of the one and into the other), a part of
/// the way along; none when it has no such path.
type Along<T> = dyn Fn(&T, &T, &(Vec<f32>, Vec<f32>), f32) -> Option<T>;

impl<T: Clone> Animated<T> {
    /// Reads `{ "a": 0|1, "k": … }`, or a bare value; `read` makes one value of the JSON of one.
    pub fn parse(v: Option<&Value>, read: &dyn Fn(&Value) -> Option<T>) -> Option<Animated<T>> {
        let v = v?;
        let k = v.get("k").unwrap_or(v);
        let keyed = k.as_array().and_then(|a| a.first()).is_some_and(|f| f.is_object() && f.get("t").is_some());
        if !keyed {
            return read(k).map(Animated::Fixed);
        }
        let list = k.as_array()?;
        let mut keys: Vec<Key<T>> = Vec::with_capacity(list.len());
        let mut last: Option<T> = None;
        for (n, key) in list.iter().enumerate() {
            let t = key.get("t").and_then(number)?;
            let from = key.get("s").and_then(read);
            // the older form says where each key ends; the newer one has the next key's start
            let to =
                key.get("e").and_then(read).or_else(|| list.get(n + 1).and_then(|next| next.get("s")).and_then(read));
            let Some(from) = from.or_else(|| last.clone()) else { continue };
            let to = to.unwrap_or_else(|| from.clone());
            last = Some(to.clone());
            if n + 1 == list.len() {
                // the last key only says when the value before it stops changing
                if key.get("s").is_some() {
                    last = Some(from.clone());
                }
                keys.push(Key {
                    t,
                    from: last.clone()?,
                    to: last.clone()?,
                    hold: true,
                    out: vec![],
                    into: vec![],
                    path: None,
                });
                break;
            }
            let path = match (key.get("to").map(numbers), key.get("ti").map(numbers)) {
                (Some(a), Some(b)) if a.iter().chain(&b).any(|v| *v != 0.0) => Some((a, b)),
                _ => None,
            };
            keys.push(Key {
                t,
                from,
                to,
                hold: key.get("h").and_then(number).is_some_and(|h| h != 0.0),
                out: handles(key.get("o")),
                into: handles(key.get("i")),
                path,
            });
        }
        let end = last?;
        if keys.is_empty() { Some(Animated::Fixed(end)) } else { Some(Animated::Keyed(keys, end)) }
    }

    /// The value at `frame`; `mix` goes from one value towards another by a part for each
    /// dimension (one part when the easing is the same for all), and `along` places a point on
    /// its path in space.
    fn at_with(&self, frame: f32, mix: &dyn Fn(&T, &T, &[f32]) -> T, along: &Along<T>) -> T {
        let (keys, end) = match self {
            Animated::Fixed(v) => return v.clone(),
            Animated::Keyed(keys, end) => (keys, end),
        };
        if frame <= keys[0].t {
            return keys[0].from.clone();
        }
        if frame >= keys[keys.len() - 1].t {
            return end.clone();
        }
        let n = keys.partition_point(|k| k.t <= frame) - 1;
        let (key, next) = (&keys[n], &keys[n + 1]);
        if key.hold || next.t <= key.t {
            return key.from.clone();
        }
        let x = (frame - key.t) / (next.t - key.t);
        let parts: Vec<f32> = (0..key.out.len().max(key.into.len()).max(1))
            .map(|d| match (key.out.get(d).or(key.out.last()), key.into.get(d).or(key.into.last())) {
                (Some(o), Some(i)) => ease(*o, *i, x),
                _ => x,
            })
            .collect();
        if let Some(path) = &key.path
            && let Some(v) = along(&key.from, &key.to, path, parts[0])
        {
            return v;
        }
        mix(&key.from, &key.to, &parts)
    }
}

// `&Vec`, not a slice: it is handed over as a way to mix whole values of an `Animated<Vec<f32>>`
#[allow(clippy::ptr_arg)]
fn mix_numbers(a: &Vec<f32>, b: &Vec<f32>, parts: &[f32]) -> Vec<f32> {
    a.iter()
        .enumerate()
        .map(|(d, v)| {
            let k = *parts.get(d).or(parts.last()).unwrap_or(&0.0);
            v + (b.get(d).copied().unwrap_or(*v) - v) * k
        })
        .collect()
}

/// A point `part` of the way along the cubic from `a` to `b` with handles `a + to` and `b + ti`,
/// by length.
#[allow(clippy::ptr_arg)]
fn along_path(a: &Vec<f32>, b: &Vec<f32>, path: &(Vec<f32>, Vec<f32>), part: f32) -> Option<Vec<f32>> {
    if a.len() < 2 || b.len() < 2 || path.0.len() < 2 || path.1.len() < 2 {
        return None;
    }
    const STEPS: usize = 48;
    let point = |t: f32| -> Vec<f32> {
        let u = 1.0 - t;
        (0..a.len())
            .map(|d| {
                let (p0, p3) = (a[d], b.get(d).copied().unwrap_or(a[d]));
                let (p1, p2) = (p0 + path.0.get(d).copied().unwrap_or(0.0), p3 + path.1.get(d).copied().unwrap_or(0.0));
                u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
            })
            .collect()
    };
    let pts: Vec<Vec<f32>> = (0..=STEPS).map(|k| point(k as f32 / STEPS as f32)).collect();
    let d = |p: &Vec<f32>, q: &Vec<f32>| p.iter().zip(q).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt();
    let lengths: Vec<f32> = pts.windows(2).map(|w| d(&w[0], &w[1])).collect();
    let total: f32 = lengths.iter().sum();
    if total <= 0.0 {
        return Some(a.clone());
    }
    // easing may overshoot: carry on straight past either end
    let mut want = part * total;
    if want <= 0.0 {
        return Some(pts[0].clone());
    }
    for (k, l) in lengths.iter().enumerate() {
        if want <= *l || k + 1 == lengths.len() {
            let f = if *l > 0.0 { want / l } else { 0.0 };
            return Some(pts[k].iter().zip(&pts[k + 1]).map(|(x, y)| x + (y - x) * f).collect());
        }
        want -= l;
    }
    None
}

/// Numbers: a scalar is a list of one.
pub(crate) type Numbers = Animated<Vec<f32>>;

impl Numbers {
    pub fn read(v: Option<&Value>) -> Option<Numbers> {
        Animated::parse(v, &|v| Some(numbers(v)).filter(|n| !n.is_empty()))
    }

    pub fn fixed(v: &[f32]) -> Numbers {
        Animated::Fixed(v.to_vec())
    }

    pub fn at(&self, frame: f32) -> Vec<f32> {
        self.at_with(frame, &mix_numbers, &along_path)
    }

    /// The first number.
    pub fn one(&self, frame: f32) -> f32 {
        self.at(frame).first().copied().unwrap_or(0.0)
    }

    /// The first two numbers.
    pub fn two(&self, frame: f32) -> P {
        let v = self.at(frame);
        [v.first().copied().unwrap_or(0.0), v.get(1).or(v.first()).copied().unwrap_or(0.0)]
    }

    /// Every stored value divided by 255, for the colours older files wrote as bytes.
    pub fn scaled(self, k: f32) -> Numbers {
        let f = |v: Vec<f32>| v.into_iter().map(|x| x * k).collect::<Vec<f32>>();
        match self {
            Animated::Fixed(v) => Animated::Fixed(f(v)),
            Animated::Keyed(keys, end) => Animated::Keyed(
                keys.into_iter().map(|key| Key { from: f(key.from), to: f(key.to), ..key }).collect(),
                f(end),
            ),
        }
    }
}

/// A path that may change shape.
pub(crate) type Path = Animated<Bez>;

fn points(v: Option<&Value>) -> Vec<P> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|p| {
                    let n = numbers(p);
                    [n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0)]
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bez(v: &Value, closed: Option<bool>) -> Option<Bez> {
    // keys hold the shape in a list of one
    let v = if let Some(a) = v.as_array() { a.first()? } else { v };
    let b = Bez {
        v: points(v.get("v")),
        i: points(v.get("i")),
        o: points(v.get("o")),
        closed: closed.or(v.get("c").and_then(|c| c.as_bool().or(c.as_f64().map(|n| n != 0.0)))).unwrap_or(false),
    };
    (b.v.len() == b.i.len() && b.v.len() == b.o.len() && !b.v.is_empty()).then_some(b)
}

impl Path {
    /// Reads a path; `closed` says so for the files old enough to keep that beside the path.
    pub fn read(v: Option<&Value>, closed: Option<bool>) -> Option<Path> {
        Animated::parse(v, &|v| bez(v, closed))
    }

    pub fn at(&self, frame: f32) -> Bez {
        self.at_with(
            frame,
            &|a, b, parts| {
                if a.v.len() != b.v.len() {
                    return a.clone();
                }
                let k = parts[0];
                let mix = |x: &[P], y: &[P]| x.iter().zip(y).map(|(p, q)| crate::geom::lerp(*p, *q, k)).collect();
                Bez { v: mix(&a.v, &b.v), i: mix(&a.i, &b.i), o: mix(&a.o, &b.o), closed: a.closed }
            },
            &|_, _, _, _| None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_ease_hold_and_end() {
        // the newer form: each key's end is the next key's start
        let v = json!({"a": 1, "k": [
            {"t": 0, "s": [0], "o": {"x": [0.5], "y": [0.5]}, "i": {"x": [0.5], "y": [0.5]}},
            {"t": 10, "s": [100], "h": 1},
            {"t": 20, "s": [50]}
        ]});
        let n = Numbers::read(Some(&v)).unwrap();
        assert_eq!(
            (n.one(-5.0), n.one(5.0), n.one(10.0), n.one(15.0), n.one(20.0), n.one(99.0)),
            (0.0, 50.0, 100.0, 100.0, 50.0, 50.0)
        );
        // the older form: each key says where it ends, and the last only when
        let v = json!({"k": [{"t": 0, "s": [0, 0], "e": [10, 20]}, {"t": 10}]});
        let n = Numbers::read(Some(&v)).unwrap();
        assert_eq!((n.two(5.0), n.two(10.0)), ([5.0, 10.0], [10.0, 20.0]));
        // slow out of the first key
        let v =
            json!({"k": [{"t": 0, "s": [0], "e": [100], "o": {"x": 0.8, "y": 0}, "i": {"x": 1, "y": 1}}, {"t": 10}]});
        assert!(Numbers::read(Some(&v)).unwrap().one(5.0) < 30.0);
        assert_eq!(Numbers::read(Some(&json!({"a": 0, "k": 7}))).unwrap().one(3.0), 7.0);
    }

    #[test]
    fn a_point_travels_its_path_by_length() {
        // out to the right and back down: an arc, not the straight line between
        let v = json!({"k": [{"t": 0, "s": [0, 0], "e": [100, 100], "to": [50, 0], "ti": [0, -50]}, {"t": 10}]});
        let n = Numbers::read(Some(&v)).unwrap();
        let mid = n.two(5.0);
        assert!(mid[0] > 60.0 && mid[1] < 40.0, "{mid:?}");
        assert_eq!(n.two(10.0), [100.0, 100.0]);
    }

    #[test]
    fn paths_change_shape() {
        let v = json!({"a": 1, "k": [
            {"t": 0, "s": [{"v": [[0, 0], [10, 0]], "i": [[0, 0], [0, 0]], "o": [[0, 0], [0, 0]], "c": false}],
                      "e": [{"v": [[0, 10], [10, 10]], "i": [[0, 0], [0, 0]], "o": [[0, 0], [0, 0]], "c": false}]},
            {"t": 10}
        ]});
        let p = Path::read(Some(&v), None).unwrap();
        assert_eq!(p.at(5.0).v, vec![[0.0, 5.0], [10.0, 5.0]]);
    }
}
