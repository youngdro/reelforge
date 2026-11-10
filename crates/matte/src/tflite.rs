//! Reading a TensorFlow Lite model: just enough of the FlatBuffers format and of the schema for
//! the operations the segmentation model uses.
//!
//! A FlatBuffers table starts with a signed offset back to its vtable, which lists where each
//! field is from the table's start (0: not there, so its default). Strings, vectors and other
//! tables are reached through unsigned offsets from where they are written. Every read is checked
//! against the file's length, so a damaged file is an error and never a panic.

/// A table in a FlatBuffers file.
#[derive(Clone, Copy)]
struct Table<'a> {
    buf: &'a [u8],
    at: usize,
}

fn bad() -> String {
    "the model file is damaged".to_string()
}

/// `len` bytes of `buf` from `at`, if they are all there.
fn span(buf: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    buf.get(at..at.checked_add(len).ok_or_else(bad)?).ok_or_else(bad)
}

fn u16_at(buf: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(span(buf, at, 2)?.try_into().map_err(|_| bad())?))
}

fn u32_at(buf: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(span(buf, at, 4)?.try_into().map_err(|_| bad())?))
}

/// Where the offset written at `at` leads: a place in the file.
fn jump(buf: &[u8], at: usize) -> Result<usize, String> {
    at.checked_add(u32_at(buf, at)? as usize).filter(|to| *to < buf.len()).ok_or_else(bad)
}

impl<'a> Table<'a> {
    /// The file's root table.
    fn root(buf: &'a [u8]) -> Result<Table<'a>, String> {
        Ok(Table { buf, at: jump(buf, 0)? })
    }

    /// Where field `id` is written, if it is.
    fn field(&self, id: usize) -> Result<Option<usize>, String> {
        let back = u32_at(self.buf, self.at)? as i32;
        let vtable = usize::try_from(self.at as i64 - i64::from(back)).map_err(|_| bad())?;
        let slot = 4 + 2 * id;
        if slot + 2 > u16_at(self.buf, vtable)? as usize {
            return Ok(None);
        }
        let offset = u16_at(self.buf, vtable.checked_add(slot).ok_or_else(bad)?)? as usize;
        Ok((offset != 0).then_some(self.at + offset))
    }

    fn i32(&self, id: usize, default: i32) -> Result<i32, String> {
        Ok(match self.field(id)? {
            Some(at) => u32_at(self.buf, at)? as i32,
            None => default,
        })
    }

    fn i8(&self, id: usize, default: i8) -> Result<i8, String> {
        Ok(match self.field(id)? {
            Some(at) => *self.buf.get(at).ok_or_else(bad)? as i8,
            None => default,
        })
    }

    fn flag(&self, id: usize) -> Result<bool, String> {
        Ok(self.i8(id, 0)? != 0)
    }

    /// The table field `id` points to.
    fn table(&self, id: usize) -> Result<Option<Table<'a>>, String> {
        Ok(match self.field(id)? {
            Some(at) => Some(Table { buf: self.buf, at: jump(self.buf, at)? }),
            None => None,
        })
    }

    /// Where the elements of vector field `id` start, and how many there are.
    fn vector(&self, id: usize) -> Result<(usize, usize), String> {
        Ok(match self.field(id)? {
            Some(at) => {
                let start = jump(self.buf, at)?;
                (start + 4, u32_at(self.buf, start)? as usize)
            }
            None => (0, 0),
        })
    }

    fn bytes(&self, id: usize) -> Result<&'a [u8], String> {
        let (start, len) = self.vector(id)?;
        span(self.buf, start, len)
    }

    fn i32s(&self, id: usize) -> Result<Vec<i32>, String> {
        let (start, len) = self.vector(id)?;
        span(self.buf, start, len.checked_mul(4).ok_or_else(bad)?)?;
        (0..len).map(|i| Ok(u32_at(self.buf, start + 4 * i)? as i32)).collect()
    }

    fn tables(&self, id: usize) -> Result<Vec<Table<'a>>, String> {
        let (start, len) = self.vector(id)?;
        span(self.buf, start, len.checked_mul(4).ok_or_else(bad)?)?;
        (0..len).map(|i| Ok(Table { buf: self.buf, at: jump(self.buf, start + 4 * i)? })).collect()
    }
}

/// Numbers in rows of columns of channels: one picture's worth (the model works on one at a time).
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    /// Rows.
    pub h: usize,
    /// Columns.
    pub w: usize,
    /// Channels.
    pub c: usize,
    /// `h * w * c` values, row by row, a pixel's channels together.
    pub data: Vec<f32>,
}

/// What a kernel is: `n` filters of `h × w × c` weights, filter by filter as the file has them.
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    /// Filters.
    pub n: usize,
    /// Rows.
    pub h: usize,
    /// Columns.
    pub w: usize,
    /// Channels.
    pub c: usize,
    /// `n * h * w * c` weights.
    pub data: Vec<f32>,
}

impl Kernel {
    /// The same kernel with its weights the other way around: by row, column and channel, a
    /// filter's last. A convolution then adds one input value to every output channel in one
    /// run over neighbouring weights, which is the order a processor does many at a time in.
    fn turned(mut self) -> Kernel {
        let (n, rest) = (self.n, self.h * self.w * self.c);
        let mut data = vec![0.0; self.data.len()];
        for (f, filter) in self.data.chunks_exact(rest.max(1)).enumerate() {
            for (i, weight) in filter.iter().enumerate() {
                data[i * n + f] = *weight;
            }
        }
        self.data = data;
        self
    }
}

/// One step of the model. Tensors are named by their number in the file.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A convolution: every output channel a weighted sum over a window of all input channels.
    /// Its kernel's weights are turned: by row, column and channel, a filter's last.
    Conv { input: usize, output: usize, kernel: Kernel, bias: Vec<f32>, stride: usize, same: bool },
    /// A convolution channel by channel.
    Depthwise { input: usize, output: usize, kernel: Kernel, bias: Vec<f32>, stride: usize, same: bool },
    /// A convolution backwards: every input pixel lays its window down, `stride` apart.
    Deconv { input: usize, output: usize, kernel: Kernel, bias: Vec<f32>, stride: usize },
    /// Each channel's mean over the whole picture.
    Mean { input: usize, output: usize },
    /// `x * min(max(x + 3, 0), 6) / 6`.
    HardSwish { input: usize, output: usize },
    /// `max(x, 0)`.
    Relu { input: usize, output: usize },
    /// `1 / (1 + e^-x)`.
    Logistic { input: usize, output: usize },
    /// Two pictures added.
    Add { a: usize, b: usize, output: usize },
    /// A picture's channels each multiplied by a number, or two pictures multiplied.
    Mul { a: usize, b: usize, output: usize },
    /// Made larger, between the pixels' centres.
    Resize { input: usize, output: usize, h: usize, w: usize },
}

/// A model read: what to do, in order, and where the picture goes in and the answer comes out.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// The steps.
    pub ops: Vec<Op>,
    /// The tensor the picture is put in, and its size (rows, columns, channels).
    pub input: (usize, [usize; 3]),
    /// The tensor the answer is read from.
    pub output: usize,
    /// How many tensors the file numbers.
    pub tensors: usize,
}

// Operator codes of the schema.
const ADD: i32 = 0;
const AVERAGE_POOL_2D: i32 = 1;
const CONV_2D: i32 = 3;
const DEPTHWISE_CONV_2D: i32 = 4;
const DEQUANTIZE: i32 = 6;
const LOGISTIC: i32 = 14;
const MUL: i32 = 18;
const RELU: i32 = 19;
const RESIZE_BILINEAR: i32 = 23;
const CUSTOM: i32 = 32;
const MEAN: i32 = 40;
const HARD_SWISH: i32 = 117;

// Tensor types of the schema.
const FLOAT32: i8 = 0;
const FLOAT16: i8 = 1;
const INT32: i8 = 2;

/// A half-precision number, as the model keeps its weights.
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let (exponent, fraction) = (i32::from((bits >> 10) & 0x1f), f32::from(bits & 0x3ff) / 1024.0);
    sign * match exponent {
        0 => fraction * 2f32.powi(-14),
        31 => {
            if fraction == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + fraction) * 2f32.powi(e - 15),
    }
}

/// The tensors of a graph as the file has them, and the numbers of those that are written in it.
struct Tensors<'a> {
    list: Vec<Table<'a>>,
    buffers: Vec<Table<'a>>,
    /// Constants made by a step that only unpacks them: by the tensor they are unpacked into.
    unpacked: std::collections::HashMap<usize, Vec<f32>>,
}

impl Tensors<'_> {
    fn shape(&self, t: usize) -> Result<Vec<usize>, String> {
        let shape = self.list.get(t).ok_or_else(bad)?.i32s(0)?;
        shape.into_iter().map(|d| usize::try_from(d).map_err(|_| bad())).collect()
    }

    /// The shape of a tensor that is a picture: rows, columns, channels.
    fn picture(&self, t: usize) -> Result<[usize; 3], String> {
        match self.shape(t)?[..] {
            [1, h, w, c] => Ok([h, w, c]),
            _ => Err(format!("tensor {t} is not one picture")),
        }
    }

    /// The numbers written in the file for tensor `t`.
    fn constant(&self, t: usize) -> Result<Vec<f32>, String> {
        if let Some(v) = self.unpacked.get(&t) {
            return Ok(v.clone());
        }
        let tensor = self.list.get(t).ok_or_else(bad)?;
        let bytes = self.buffers.get(tensor.i32(2, 0)? as usize).ok_or_else(bad)?.bytes(0)?;
        let count: usize = self.shape(t)?.iter().product();
        let values: Vec<f32> = match tensor.i8(1, FLOAT32)? {
            FLOAT32 => bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(),
            FLOAT16 => bytes.as_chunks::<2>().0.iter().map(|b| half(u16::from_le_bytes(*b))).collect(),
            INT32 => bytes.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b) as f32).collect(),
            other => return Err(format!("tensor {t} has numbers of a kind ({other}) this reader does not know")),
        };
        if values.len() != count || count == 0 {
            return Err(format!("tensor {t} is not written in the file"));
        }
        Ok(values)
    }

    fn kernel(&self, t: usize) -> Result<Kernel, String> {
        match self.shape(t)?[..] {
            [n, h, w, c] => Ok(Kernel { n, h, w, c, data: self.constant(t)? }),
            _ => Err(format!("tensor {t} is not a kernel")),
        }
    }
}

/// Reads a model: its first graph, whose steps must all be ones this crate can take.
pub fn read(file: &[u8]) -> Result<Model, String> {
    let root = Table::root(file)?;
    let codes = root.tables(1)?;
    let graph = *root.tables(2)?.first().ok_or_else(bad)?;
    let mut tensors = Tensors { list: graph.tables(0)?, buffers: root.tables(4)?, unpacked: Default::default() };
    let (inputs, outputs) = (graph.i32s(1)?, graph.i32s(2)?);
    let (&[input], &[output]) = (&inputs[..], &outputs[..]) else {
        return Err("the model must take one picture and give one answer".into());
    };
    let mut ops = Vec::new();
    for op in graph.tables(3)? {
        let code = codes.get(op.i32(0, 0)? as usize).ok_or_else(bad)?;
        // newer files write the code in a wider field and leave the old one at its most
        let code = code.i32(3, 0)?.max(i32::from(code.i8(0, 0)?));
        let ins: Vec<usize> = op.i32s(1)?.into_iter().map(|t| t.max(0) as usize).collect();
        let outs = op.i32s(2)?;
        let (Some(&first), Some(&out)) = (ins.first(), outs.first()) else { return Err(bad()) };
        let output = usize::try_from(out).map_err(|_| bad())?;
        let options = op.table(4)?;
        let option = |id: usize, default: i32| options.map_or(Ok(default), |o| o.i32(id, default));
        // options start with how the edges are padded: 0 is "same" (the output as large as the
        // input over the stride), 1 is "valid" (only where the window fits)
        let same = options.map_or(Ok(0), |o| o.i8(0, 0))? == 0;
        let unfused = |id: usize| match options.map_or(Ok(0), |o| o.i8(id, 0))? {
            0 => Ok(()),
            _ => Err("a step with an activation folded into it is not supported".to_string()),
        };
        let second = || ins.get(1).copied().ok_or_else(bad);
        let third = || ins.get(2).copied().ok_or_else(bad);
        match code {
            DEQUANTIZE => {
                let values = tensors.constant(first)?;
                tensors.unpacked.insert(output, values);
            }
            CONV_2D | DEPTHWISE_CONV_2D => {
                let (kernel, bias) = (tensors.kernel(second()?)?, tensors.constant(third()?)?);
                let stride = option(1, 1)?;
                if stride != option(2, 1)? || stride < 1 {
                    return Err("a convolution must step the same way across and down".into());
                }
                let stride = stride as usize;
                if code == CONV_2D {
                    unfused(3)?;
                    if (option(4, 1)?, option(5, 1)?) != (1, 1) || bias.len() != kernel.n {
                        return Err("a convolution of a kind this reader does not know".into());
                    }
                    ops.push(Op::Conv { input: first, output, kernel: kernel.turned(), bias, stride, same });
                } else {
                    unfused(4)?;
                    if (option(3, 1)?, option(5, 1)?, option(6, 1)?) != (1, 1, 1)
                        || kernel.n != 1
                        || bias.len() != kernel.c
                    {
                        return Err("a channel-wise convolution of a kind this reader does not know".into());
                    }
                    ops.push(Op::Depthwise { input: first, output, kernel, bias, stride, same });
                }
            }
            AVERAGE_POOL_2D => {
                // only over the whole picture: a mean
                let [h, w, _] = tensors.picture(first)?;
                if (option(3, 0)? as usize, option(4, 0)? as usize) != (w, h) {
                    return Err("an average over part of a picture is not supported".into());
                }
                unfused(5)?;
                ops.push(Op::Mean { input: first, output });
            }
            MEAN => {
                if tensors.picture(output)?[..2] != [1, 1] {
                    return Err("a mean must be over the whole picture and keep its place".into());
                }
                ops.push(Op::Mean { input: first, output });
            }
            HARD_SWISH => ops.push(Op::HardSwish { input: first, output }),
            RELU => ops.push(Op::Relu { input: first, output }),
            LOGISTIC => ops.push(Op::Logistic { input: first, output }),
            ADD | MUL => {
                unfused(0)?;
                let b = second()?;
                ops.push(if code == ADD { Op::Add { a: first, b, output } } else { Op::Mul { a: first, b, output } });
            }
            RESIZE_BILINEAR => {
                // corners not aligned, pixels' centres halfway: the only way this crate resizes
                if options.map_or(Ok(false), |o| o.flag(2))? || !options.map_or(Ok(false), |o| o.flag(3))? {
                    return Err("a resize of a kind this reader does not know".into());
                }
                let [h, w, _] = tensors.picture(output)?;
                ops.push(Op::Resize { input: first, output, h, w });
            }
            CUSTOM => {
                // MediaPipe's transposed convolution with a bias; its options are three numbers:
                // the padding, and the stride across and down
                let name = codes.get(op.i32(0, 0)? as usize).ok_or_else(bad)?.bytes(1)?;
                let custom = op.bytes(5)?;
                let number = |i: usize| u32_at(custom, 4 * i);
                if name != b"Convolution2DTransposeBias" || custom.len() != 12 || number(1)? != number(2)? {
                    return Err(format!("a step this reader does not know: {}", String::from_utf8_lossy(name)));
                }
                let (kernel, bias, stride) =
                    (tensors.kernel(second()?)?, tensors.constant(third()?)?, number(1)? as usize);
                if kernel.h != stride || kernel.w != stride || bias.len() != kernel.n || stride == 0 {
                    return Err("a transposed convolution whose windows overlap is not supported".into());
                }
                ops.push(Op::Deconv { input: first, output, kernel, bias, stride });
            }
            other => return Err(format!("a step this reader does not know: operator {other}")),
        }
    }
    let input = usize::try_from(input).map_err(|_| bad())?;
    Ok(Model {
        ops,
        input: (input, tensors.picture(input)?),
        output: usize::try_from(output).map_err(|_| bad())?,
        tensors: tensors.list.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_precision_numbers_are_read() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0x3555), 0.333_251_95);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert_eq!(half(0x0000), 0.0);
        assert!(half(0x7c00).is_infinite() && half(0x7e00).is_nan());
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_panic() {
        let whole = crate::MODEL;
        assert!(read(&[]).is_err() && read(&[9; 3]).is_err());
        // cut short, and with its offsets pointing nowhere
        for len in [8, 64, 1000, whole.len() / 2, whole.len() - 1] {
            let _ = read(&whole[..len]);
        }
        let mut broken = whole.to_vec();
        for (i, b) in broken.iter_mut().enumerate().take(4000) {
            if i % 7 == 0 {
                *b = 0xff;
            }
        }
        assert!(read(&broken).is_err());
    }
}
