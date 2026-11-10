//! Running a model read by [`tflite`](crate::tflite): each step on plain rows of numbers.
//!
//! Pictures are rows of pixels of channels. The loops are written so that the innermost one runs
//! over neighbouring numbers without carrying anything from one turn to the next, which the
//! compiler turns into several at a time; rows are shared out over the machine's cores.

use crate::tflite::{Kernel, Model, Op, Tensor};
use rayon::prelude::*;

/// Runs `model` on `picture`, which must be of the size it takes.
pub fn run(model: &Model, picture: Tensor) -> Result<Tensor, String> {
    let (at, [h, w, c]) = model.input;
    if [picture.h, picture.w, picture.c] != [h, w, c] || picture.data.len() != h * w * c {
        return Err(format!("the model takes a picture of {w}×{h} with {c} channels"));
    }
    // when each tensor is read last, so that it can be let go of then
    let mut last = vec![0; model.tensors];
    for (i, op) in model.ops.iter().enumerate() {
        for t in reads(op) {
            *last.get_mut(t).ok_or_else(lost)? = i;
        }
    }
    let mut values: Vec<Option<Tensor>> = vec![None; model.tensors];
    *values.get_mut(at).ok_or_else(lost)? = Some(picture);
    for (i, op) in model.ops.iter().enumerate() {
        let value = |t: usize| values.get(t).and_then(Option::as_ref).ok_or_else(lost);
        let (output, made) = match op {
            Op::Conv { input, output, kernel, bias, stride, same } => {
                (*output, conv(value(*input)?, kernel, bias, *stride, *same)?)
            }
            Op::Depthwise { input, output, kernel, bias, stride, same } => {
                (*output, depthwise(value(*input)?, kernel, bias, *stride, *same)?)
            }
            Op::Deconv { input, output, kernel, bias, stride } => {
                (*output, deconv(value(*input)?, kernel, bias, *stride)?)
            }
            Op::Mean { input, output } => (*output, mean(value(*input)?)),
            Op::HardSwish { input, output } => (*output, each(value(*input)?, |x| x * (x + 3.0).clamp(0.0, 6.0) / 6.0)),
            Op::Relu { input, output } => (*output, each(value(*input)?, |x| x.max(0.0))),
            Op::Logistic { input, output } => (*output, each(value(*input)?, |x| 1.0 / (1.0 + (-x).exp()))),
            Op::Add { a, b, output } => (*output, pairs(value(*a)?, value(*b)?, |x, y| x + y)?),
            Op::Mul { a, b, output } => (*output, pairs(value(*a)?, value(*b)?, |x, y| x * y)?),
            Op::Resize { input, output, h, w } => (*output, resize(value(*input)?, *h, *w)?),
        };
        for t in reads(op) {
            if last[t] == i && t != model.output {
                values[t] = None;
            }
        }
        *values.get_mut(output).ok_or_else(lost)? = Some(made);
    }
    values.get_mut(model.output).and_then(Option::take).ok_or_else(lost)
}

fn lost() -> String {
    "the model reads a tensor that nothing made".to_string()
}

/// The tensors a step reads.
fn reads(op: &Op) -> impl Iterator<Item = usize> {
    let (a, b) = match op {
        Op::Conv { input, .. }
        | Op::Depthwise { input, .. }
        | Op::Deconv { input, .. }
        | Op::Mean { input, .. }
        | Op::HardSwish { input, .. }
        | Op::Relu { input, .. }
        | Op::Logistic { input, .. }
        | Op::Resize { input, .. } => (*input, None),
        Op::Add { a, b, .. } | Op::Mul { a, b, .. } => (*a, Some(*b)),
    };
    std::iter::once(a).chain(b)
}

/// How many windows of `k` fit along `len` stepping by `stride`, and how far before the first
/// value the first window starts. "Same" padding gives `len / stride` rounded up, with the
/// padding split evenly and the odd one after.
fn windows(len: usize, k: usize, stride: usize, same: bool) -> Result<(usize, usize), String> {
    if len == 0 || k == 0 || stride == 0 || (!same && len < k) {
        return Err("a window does not fit its picture".into());
    }
    if same {
        let out = len.div_ceil(stride);
        Ok((out, ((out - 1) * stride + k).saturating_sub(len) / 2))
    } else {
        Ok(((len - k) / stride + 1, 0))
    }
}

/// The input position a window's cell reads, if it is inside the picture.
fn inside(out: usize, stride: usize, cell: usize, before: usize, len: usize) -> Option<usize> {
    (out * stride + cell).checked_sub(before).filter(|at| *at < len)
}

/// `kernel` must be turned (see [`Op::Conv`]).
fn conv(x: &Tensor, k: &Kernel, bias: &[f32], stride: usize, same: bool) -> Result<Tensor, String> {
    if x.c != k.c || bias.len() != k.n || k.data.len() != k.n * k.h * k.w * k.c || k.n == 0 || k.c == 0 {
        return Err("a convolution does not fit its picture".into());
    }
    let ((h, top), (w, left)) = (windows(x.h, k.h, stride, same)?, windows(x.w, k.w, stride, same)?);
    let mut data = vec![0.0; h * w * k.n];
    data.par_chunks_mut(w * k.n).enumerate().for_each(|(oy, row)| {
        for (ox, out) in row.chunks_exact_mut(k.n).enumerate() {
            out.copy_from_slice(bias);
            for ky in 0..k.h {
                let Some(iy) = inside(oy, stride, ky, top, x.h) else { continue };
                for kx in 0..k.w {
                    let Some(ix) = inside(ox, stride, kx, left, x.w) else { continue };
                    let pixel = &x.data[(iy * x.w + ix) * x.c..][..x.c];
                    let weights = &k.data[(ky * k.w + kx) * k.c * k.n..][..k.c * k.n];
                    for (v, to_each) in pixel.iter().zip(weights.chunks_exact(k.n)) {
                        for (o, weight) in out.iter_mut().zip(to_each) {
                            *o += v * weight;
                        }
                    }
                }
            }
        }
    });
    Ok(Tensor { h, w, c: k.n, data })
}

fn depthwise(x: &Tensor, k: &Kernel, bias: &[f32], stride: usize, same: bool) -> Result<Tensor, String> {
    if x.c != k.c || bias.len() != k.c || k.n != 1 || k.data.len() != k.h * k.w * k.c || k.c == 0 {
        return Err("a channel-wise convolution does not fit its picture".into());
    }
    let ((h, top), (w, left)) = (windows(x.h, k.h, stride, same)?, windows(x.w, k.w, stride, same)?);
    let mut data = vec![0.0; h * w * k.c];
    data.par_chunks_mut(w * k.c).enumerate().for_each(|(oy, row)| {
        for (ox, out) in row.chunks_exact_mut(k.c).enumerate() {
            out.copy_from_slice(bias);
            for ky in 0..k.h {
                let Some(iy) = inside(oy, stride, ky, top, x.h) else { continue };
                for kx in 0..k.w {
                    let Some(ix) = inside(ox, stride, kx, left, x.w) else { continue };
                    let pixel = &x.data[(iy * x.w + ix) * x.c..][..x.c];
                    let weights = &k.data[(ky * k.w + kx) * k.c..][..k.c];
                    for ((o, v), weight) in out.iter_mut().zip(pixel).zip(weights) {
                        *o += v * weight;
                    }
                }
            }
        }
    });
    Ok(Tensor { h, w, c: k.c, data })
}

/// Every input pixel lays a `stride × stride` window down, the windows side by side.
fn deconv(x: &Tensor, k: &Kernel, bias: &[f32], stride: usize) -> Result<Tensor, String> {
    if x.c != k.c || bias.len() != k.n || k.h != stride || k.w != stride || k.data.len() != k.n * k.h * k.w * k.c {
        return Err("a transposed convolution does not fit its picture".into());
    }
    let (h, w) = (x.h * stride, x.w * stride);
    let mut data = vec![0.0; h * w * k.n];
    if data.is_empty() {
        return Err("a transposed convolution does not fit its picture".into());
    }
    data.par_chunks_mut(w * k.n).enumerate().for_each(|(oy, row)| {
        let (iy, ky) = (oy / stride, oy % stride);
        for (ox, out) in row.chunks_exact_mut(k.n).enumerate() {
            let (ix, kx) = (ox / stride, ox % stride);
            let pixel = &x.data[(iy * x.w + ix) * x.c..][..x.c];
            for (n, o) in out.iter_mut().enumerate() {
                let weights = &k.data[((n * k.h + ky) * k.w + kx) * k.c..][..k.c];
                *o = bias[n] + pixel.iter().zip(weights).map(|(v, weight)| v * weight).sum::<f32>();
            }
        }
    });
    Ok(Tensor { h, w, c: k.n, data })
}

fn mean(x: &Tensor) -> Tensor {
    let mut data = vec![0.0; x.c];
    for pixel in x.data.chunks_exact(x.c.max(1)) {
        for (sum, v) in data.iter_mut().zip(pixel) {
            *sum += v;
        }
    }
    let count = (x.h * x.w).max(1) as f32;
    data.iter_mut().for_each(|sum| *sum /= count);
    Tensor { h: 1, w: 1, c: x.c, data }
}

fn each(x: &Tensor, f: impl Fn(f32) -> f32) -> Tensor {
    Tensor { data: x.data.iter().map(|v| f(*v)).collect(), ..*x }
}

/// Two pictures of one size value by value, or a picture and one pixel's worth of channels
/// (either way around), every pixel with those.
fn pairs(a: &Tensor, b: &Tensor, f: impl Fn(f32, f32) -> f32) -> Result<Tensor, String> {
    let (whole, one) = if (b.h, b.w) == (1, 1) { (a, b) } else { (b, a) };
    if whole.c != one.c || whole.c == 0 || ((one.h, one.w) != (1, 1) && (one.h, one.w) != (whole.h, whole.w)) {
        return Err("two pictures of different sizes cannot be put together".into());
    }
    // `one` taken around as often as it takes covers both cases
    let data = whole.data.iter().zip(one.data.iter().cycle()).map(|(x, y)| f(*x, *y)).collect();
    Ok(Tensor { data, ..*whole })
}

/// Bigger or smaller between the pixels' centres, the edge pixels held beyond them.
fn resize(x: &Tensor, h: usize, w: usize) -> Result<Tensor, String> {
    if x.h == 0 || x.w == 0 || x.c == 0 || h == 0 || w == 0 {
        return Err("an empty picture cannot be resized".into());
    }
    // for an output position: the two inputs it lies between and how far towards the second
    let between = |out: usize, len: usize, of: usize| {
        let at = (out as f32 + 0.5) * of as f32 / len as f32 - 0.5;
        let low = at.floor();
        ((low.max(0.0) as usize).min(of - 1), (at.ceil().max(0.0) as usize).min(of - 1), at - low)
    };
    let mut data = vec![0.0; h * w * x.c];
    data.par_chunks_mut(w * x.c).enumerate().for_each(|(oy, row)| {
        let (y0, y1, fy) = between(oy, h, x.h);
        for (ox, out) in row.chunks_exact_mut(x.c).enumerate() {
            let (x0, x1, fx) = between(ox, w, x.w);
            let pixel = |py: usize, px: usize| &x.data[(py * x.w + px) * x.c..][..x.c];
            let (a, b, c, d) = (pixel(y0, x0), pixel(y0, x1), pixel(y1, x0), pixel(y1, x1));
            for (i, o) in out.iter_mut().enumerate() {
                let (top, bottom) = (a[i] + (b[i] - a[i]) * fx, c[i] + (d[i] - c[i]) * fx);
                *o = top + (bottom - top) * fy;
            }
        }
    });
    Ok(Tensor { h, w, c: x.c, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(h: usize, w: usize, c: usize, data: &[f32]) -> Tensor {
        Tensor { h, w, c, data: data.to_vec() }
    }

    #[test]
    fn windows_are_padded_as_the_format_says() {
        // "same": the output is the input over the stride, rounded up; the odd padding goes after
        assert_eq!(windows(256, 3, 2, true), Ok((128, 0)));
        assert_eq!(windows(5, 3, 2, true), Ok((3, 1)));
        assert_eq!(windows(8, 5, 1, true), Ok((8, 2)));
        assert_eq!(windows(8, 1, 1, true), Ok((8, 0)));
        // "valid": only where the window fits
        assert_eq!(windows(8, 3, 1, false), Ok((6, 0)));
        assert_eq!(windows(8, 3, 2, false), Ok((3, 0)));
        assert!(windows(2, 3, 1, false).is_err() && windows(0, 1, 1, true).is_err());
    }

    #[test]
    fn a_convolution_sums_its_window_with_nothing_beyond_the_edges() {
        // one channel, a 3×3 kernel of ones: each pixel becomes the sum of its neighbourhood
        let x = tensor(3, 3, 1, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let k = Kernel { n: 1, h: 3, w: 3, c: 1, data: vec![1.0; 9] };
        let y = conv(&x, &k, &[0.5], 1, true).unwrap();
        assert_eq!(y.data, [12.5, 21.5, 16.5, 27.5, 45.5, 33.5, 24.5, 39.5, 28.5]);
        // stepping by two from an even size starts on the first pixel
        let x = tensor(2, 2, 1, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(conv(&x, &k, &[0.0], 2, true).unwrap().data, [10.0]);
        // two filters over two channels, turned: each channel's weights for both filters together
        let x = tensor(1, 1, 2, &[2.0, 3.0]);
        let k = Kernel { n: 2, h: 1, w: 1, c: 2, data: vec![1.0, 10.0, 100.0, 1000.0] };
        assert_eq!(conv(&x, &k, &[0.0, 1.0], 1, true).unwrap().data, [302.0, 3021.0]);
        assert!(conv(&x, &k, &[0.0], 1, true).is_err());
    }

    #[test]
    fn a_channel_wise_convolution_keeps_channels_apart() {
        let x = tensor(1, 2, 2, &[1.0, 10.0, 2.0, 20.0]);
        let k = Kernel { n: 1, h: 1, w: 3, c: 2, data: vec![1.0, 1.0, 2.0, 0.0, 4.0, 1.0] };
        // a window of three centred on each pixel. The first channel's weights are 1, 2, 4:
        // 2·x0 + 4·x1, then 1·x0 + 2·x1; the second's are 1, 0, 1 and it has a bias
        assert_eq!(depthwise(&x, &k, &[0.0, 0.5], 1, true).unwrap().data, [10.0, 20.5, 5.0, 10.5]);
    }

    #[test]
    fn a_transposed_convolution_lays_windows_side_by_side() {
        let x = tensor(1, 2, 1, &[1.0, 2.0]);
        let k = Kernel { n: 1, h: 2, w: 2, c: 1, data: vec![1.0, 2.0, 3.0, 4.0] };
        let y = deconv(&x, &k, &[0.5], 2).unwrap();
        assert_eq!((y.h, y.w), (2, 4));
        assert_eq!(y.data, [1.5, 2.5, 2.5, 4.5, 3.5, 4.5, 6.5, 8.5]);
    }

    #[test]
    fn the_small_steps() {
        let x = tensor(1, 2, 2, &[1.0, -4.0, 3.0, 2.0]);
        assert_eq!(mean(&x).data, [2.0, -1.0]);
        assert_eq!(each(&x, |v| v.max(0.0)).data, [1.0, 0.0, 3.0, 2.0]);
        let gains = tensor(1, 1, 2, &[2.0, 0.5]);
        assert_eq!(pairs(&x, &gains, |a, b| a * b).unwrap().data, [2.0, -2.0, 6.0, 1.0]);
        assert_eq!(pairs(&gains, &x, |a, b| a * b).unwrap().data, [2.0, -2.0, 6.0, 1.0]);
        assert_eq!(pairs(&x, &x, |a, b| a + b).unwrap().data, [2.0, -8.0, 6.0, 4.0]);
        assert!(pairs(&x, &tensor(1, 1, 1, &[1.0]), |a, b| a + b).is_err());
        // twice as wide: the edges held, a quarter and three quarters between
        let y = resize(&tensor(1, 2, 1, &[0.0, 1.0]), 1, 4).unwrap();
        assert_eq!(y.data, [0.0, 0.25, 0.75, 1.0]);
    }
}
