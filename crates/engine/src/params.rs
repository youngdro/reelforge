//! Reading command parameters.

use reelforge_time::{FrameRate, Tick};
use serde_json::Value;

use crate::EngineError;

/// Reads a time: an integer is ticks; `{"seconds": 1.5}`, `{"frame": 45}` (at `fps`) and
/// `{"ticks": n}` are also accepted, so agents need not know the tick base.
pub fn parse_time(v: &Value, fps: FrameRate) -> Result<Tick, String> {
    if let Some(n) = v.as_i64() {
        return Ok(Tick(n));
    }
    let obj = v.as_object().ok_or("a time is ticks, or {\"seconds\": s} or {\"frame\": n}")?;
    if let Some(s) = obj.get("seconds").and_then(Value::as_f64) {
        return Ok(Tick::from_seconds(s));
    }
    if let Some(f) = obj.get("frame").and_then(Value::as_i64) {
        return Ok(fps.tick_of(f));
    }
    if let Some(t) = obj.get("ticks").and_then(Value::as_i64) {
        return Ok(Tick(t));
    }
    Err("a time is ticks, or {\"seconds\": s} or {\"frame\": n}".into())
}

pub(crate) struct P<'a> {
    pub v: &'a Value,
    pub fps: FrameRate,
}

impl<'a> P<'a> {
    pub fn new(v: &'a Value, fps: FrameRate) -> P<'a> {
        P { v, fps }
    }

    fn missing(name: &str) -> EngineError {
        EngineError::Params(format!("`{name}` is required"))
    }

    fn wrong(name: &str, what: &str) -> EngineError {
        EngineError::Params(format!("`{name}` must be {what}"))
    }

    pub fn opt(&self, name: &str) -> Option<&'a Value> {
        self.v.get(name).filter(|v| !v.is_null())
    }

    pub fn str(&self, name: &str) -> Result<&'a str, EngineError> {
        self.opt_str(name)?.ok_or_else(|| Self::missing(name))
    }

    pub fn opt_str(&self, name: &str) -> Result<Option<&'a str>, EngineError> {
        self.opt(name).map(|v| v.as_str().ok_or_else(|| Self::wrong(name, "a string"))).transpose()
    }

    pub fn opt_bool(&self, name: &str) -> Result<Option<bool>, EngineError> {
        self.opt(name).map(|v| v.as_bool().ok_or_else(|| Self::wrong(name, "true or false"))).transpose()
    }

    pub fn opt_f64(&self, name: &str) -> Result<Option<f64>, EngineError> {
        self.opt(name)
            .map(|v| v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| Self::wrong(name, "a finite number")))
            .transpose()
    }

    pub fn f64(&self, name: &str) -> Result<f64, EngineError> {
        self.opt_f64(name)?.ok_or_else(|| Self::missing(name))
    }

    pub fn opt_u32(&self, name: &str) -> Result<Option<u32>, EngineError> {
        self.opt(name)
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| Self::wrong(name, "a non-negative integer"))
            })
            .transpose()
    }

    pub fn opt_time(&self, name: &str) -> Result<Option<Tick>, EngineError> {
        self.opt(name).map(|v| parse_time(v, self.fps).map_err(|e| Self::wrong(name, &e))).transpose()
    }

    pub fn time(&self, name: &str) -> Result<Tick, EngineError> {
        self.opt_time(name)?.ok_or_else(|| Self::missing(name))
    }

    pub fn opt_color(&self, name: &str) -> Result<Option<[f32; 4]>, EngineError> {
        let Some(v) = self.opt(name) else { return Ok(None) };
        let arr = v
            .as_array()
            .filter(|a| a.len() == 3 || a.len() == 4)
            .ok_or_else(|| Self::wrong(name, "[r, g, b] or [r, g, b, a], each 0 to 1"))?;
        let mut c = [1.0f32; 4];
        for (i, x) in arr.iter().enumerate() {
            let f = x
                .as_f64()
                .filter(|f| (0.0..=1.0).contains(f))
                .ok_or_else(|| Self::wrong(name, "[r, g, b] or [r, g, b, a], each 0 to 1"))?;
            c[i] = f as f32;
        }
        Ok(Some(c))
    }

    pub fn strings(&self, name: &str) -> Result<Vec<String>, EngineError> {
        match self.opt(name) {
            None => Err(Self::missing(name)),
            Some(Value::String(s)) => Ok(vec![s.clone()]),
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| {
                    x.as_str().map(str::to_string).ok_or_else(|| Self::wrong(name, "a string or a list of strings"))
                })
                .collect(),
            Some(_) => Err(Self::wrong(name, "a string or a list of strings")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_in_every_spelling() {
        let fps = FrameRate::FPS_30;
        assert_eq!(parse_time(&json!(5), fps).unwrap(), Tick(5));
        assert_eq!(parse_time(&json!({"seconds": 1}), fps).unwrap(), Tick::SECOND);
        assert_eq!(parse_time(&json!({"frame": 30}), fps).unwrap(), Tick::SECOND);
        assert!(parse_time(&json!("1s"), fps).is_err());
    }
}
