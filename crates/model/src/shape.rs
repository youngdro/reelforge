//! Shapes and numbers: the building blocks of price tags, discount badges, banners and counters.
//!
//! A shape clip ([`crate::ClipContent::Shape`]) draws a box, ellipse, line, polygon, star, price
//! tag or arrow, filled with a colour or a gradient and outlined; its outline can draw itself on
//! (the `drawStart` and `drawEnd` properties, and the `draw` animation). A title can show a number
//! that runs from one value to another ([`Counter`]): a price, a count of sales, a countdown.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Ease;

/// What a shape clip draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Shape {
    /// Which shape.
    pub kind: ShapeKind,
    /// Width and height in canvas pixels, before the clip's scale.
    pub size: [f32; 2],
    /// Corner radius of a rectangle, a tag or an arrow, in canvas pixels.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f32,
    /// Sides of a polygon, points of a star: 3 to 64.
    #[serde(default = "five")]
    pub points: u32,
    /// How far in a star's inner corners are, 0.05 to 1.
    #[serde(default = "half")]
    pub inner: f32,
    /// What fills it; none leaves the outline alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    /// The outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// A soft shadow beneath it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    /// The part of the outline drawn, from its start: `[start, end]`, each 0 to 1.
    #[serde(default = "whole", skip_serializing_if = "is_whole")]
    pub trim: [f32; 2],
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn five() -> u32 {
    5
}

fn half() -> f32 {
    0.5
}

fn whole() -> [f32; 2] {
    [0.0, 1.0]
}

fn is_whole(t: &[f32; 2]) -> bool {
    *t == whole()
}

/// Shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    /// A box, its corners rounded by `radius`.
    Rectangle,
    /// An ellipse (a circle in a square box).
    Ellipse,
    /// A line across the box, left to right; it shows by its outline.
    Line,
    /// A regular polygon of `points` sides.
    Polygon,
    /// A star of `points` points; with many, a burst badge.
    Star,
    /// A price tag: pointed at its left end, with a hole.
    Tag,
    /// An arrow pointing right.
    Arrow,
}

impl ShapeKind {
    /// Every shape, in menu order.
    pub const ALL: [ShapeKind; 7] = [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Polygon,
        ShapeKind::Star,
        ShapeKind::Tag,
        ShapeKind::Arrow,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "rectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Line => "line",
            ShapeKind::Polygon => "polygon",
            ShapeKind::Star => "star",
            ShapeKind::Tag => "tag",
            ShapeKind::Arrow => "arrow",
        }
    }

    /// The shape a command names.
    pub fn parse(name: &str) -> Option<ShapeKind> {
        ShapeKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// How a shape is filled. Colours are straight sRGB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Fill {
    /// One colour.
    Solid {
        /// The colour.
        color: [f32; 4],
    },
    /// From one colour to another across the shape.
    Linear {
        /// Where it starts.
        from: [f32; 4],
        /// Where it ends.
        to: [f32; 4],
        /// The way it runs, degrees clockwise from left-to-right (90: top to bottom).
        #[serde(default)]
        angle: f32,
    },
    /// From one colour at the centre to another at the edge.
    Radial {
        /// At the centre.
        from: [f32; 4],
        /// At the edge.
        to: [f32; 4],
    },
}

/// A shape's outline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stroke {
    /// Straight sRGB colour.
    pub color: [f32; 4],
    /// Width in canvas pixels, centred on the edge.
    pub width: f32,
}

/// A shape's shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeShadow {
    /// Straight sRGB colour.
    pub color: [f32; 4],
    /// Offset `[right, down]` in canvas pixels.
    pub offset: [f32; 2],
    /// Softness in canvas pixels.
    #[serde(default)]
    pub blur: f32,
}

impl Shape {
    /// A plain filled shape of `kind`, `size` canvas pixels.
    pub fn new(kind: ShapeKind, size: [f32; 2], color: [f32; 4]) -> Shape {
        let filled = kind != ShapeKind::Line;
        Shape {
            kind,
            size,
            radius: 0.0,
            points: if kind == ShapeKind::Star { 5 } else { 6 },
            inner: 0.5,
            fill: filled.then_some(Fill::Solid { color }),
            stroke: (!filled).then_some(Stroke { color, width: 8.0 }),
            shadow: None,
            trim: whole(),
        }
    }

    /// What is wrong with it, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        let fine = |v: f32| v.is_finite() && (0.0..=16384.0).contains(&v);
        if !(self.size.iter().all(|v| fine(*v)) && fine(self.radius)) {
            return Some("a shape's size and corner radius must be 0 to 16384 pixels");
        }
        if !(3..=64).contains(&self.points) {
            return Some("a polygon or star has 3 to 64 points");
        }
        if !(self.inner.is_finite() && (0.05..=1.0).contains(&self.inner)) {
            return Some("a star's inner corners must be 0.05 to 1 of the way out");
        }
        let colours: Vec<[f32; 4]> = match &self.fill {
            Some(Fill::Solid { color }) => vec![*color],
            Some(Fill::Linear { from, to, .. } | Fill::Radial { from, to }) => vec![*from, *to],
            None => vec![],
        };
        let colours =
            colours.into_iter().chain(self.stroke.iter().map(|s| s.color)).chain(self.shadow.iter().map(|s| s.color));
        if colours.flatten().any(|c| !(0.0..=1.0).contains(&c)) {
            return Some("colours are 0 to 1");
        }
        if let Some(Fill::Linear { angle, .. }) = &self.fill
            && !angle.is_finite()
        {
            return Some("a gradient's angle must be a number");
        }
        if self.stroke.as_ref().is_some_and(|s| !(s.width.is_finite() && (0.0..=2048.0).contains(&s.width))) {
            return Some("an outline is 0 to 2048 pixels wide");
        }
        if self
            .shadow
            .as_ref()
            .is_some_and(|s| !(s.offset.iter().all(|o| o.is_finite() && o.abs() <= 4096.0) && fine(s.blur)))
        {
            return Some("a shadow's offset and softness must be within reach");
        }
        if !self.trim.iter().all(|v| (0.0..=1.0).contains(v)) {
            return Some("an outline is drawn from 0 to 1 of the way round");
        }
        None
    }
}

/// A number a title shows, running from one value to another: wherever its words say `{n}`, or as
/// all of its words when they do not. Keys on the `count` property, where it has some, say the
/// value instead.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Counter {
    /// The value at the clip's start.
    pub from: f32,
    /// The value it arrives at.
    pub to: f32,
    /// How long it takes to get there from the clip's start; the whole clip when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<Tick>,
    /// How it gets there.
    #[serde(default = "out")]
    pub ease: Ease,
    /// Digits after the decimal point, 0 to 4.
    #[serde(default)]
    pub decimals: u8,
    /// Thousands separated by commas.
    #[serde(default, skip_serializing_if = "crate::is_no")]
    pub group: bool,
    /// Written as a clock of whole seconds (`01:30`): a countdown runs `from` 90 `to` 0.
    #[serde(default, skip_serializing_if = "crate::is_no")]
    pub clock: bool,
}

fn out() -> Ease {
    Ease::OUT
}

impl Counter {
    /// The value at `t` from the start of a clip `len` long.
    pub fn at(&self, t: Tick, len: Tick) -> f32 {
        let over = self.over.unwrap_or(len);
        let f = if over <= Tick::ZERO { 1.0 } else { (t.0 as f64 / over.0 as f64).clamp(0.0, 1.0) as f32 };
        let e = if self.ease == Ease::Hold { if f >= 1.0 { 1.0 } else { 0.0 } } else { self.ease.apply(f) };
        self.from + (self.to - self.from) * e
    }

    /// What is wrong with it, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if !(self.from.is_finite() && self.to.is_finite() && self.from.abs() < 1e12 && self.to.abs() < 1e12) {
            return Some("a counter's values must be numbers");
        }
        if self.decimals > 4 {
            return Some("a counter shows 0 to 4 decimals");
        }
        if self.over.is_some_and(|o| o < Tick::ZERO) {
            return Some("a counter cannot take less than no time");
        }
        if !self.ease.is_valid() {
            return Some("a counter's curve must go forward in time");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_counter_runs_from_one_value_to_the_other_and_holds() {
        let c = Counter {
            from: 0.0,
            to: 100.0,
            over: Some(Tick(100)),
            ease: Ease::Linear,
            decimals: 0,
            group: false,
            clock: false,
        };
        assert_eq!(
            [c.at(Tick(0), Tick(500)), c.at(Tick(50), Tick(500)), c.at(Tick(300), Tick(500))],
            [0.0, 50.0, 100.0]
        );
        // without `over` it takes the whole clip, slowing into place
        let c = Counter { over: None, ease: Ease::OUT, ..c };
        assert!(c.at(Tick(250), Tick(500)) > 60.0 && c.at(Tick(500), Tick(500)) == 100.0);
        assert!(c.problem().is_none() && Counter { decimals: 9, ..c }.problem().is_some());
    }

    #[test]
    fn shapes_name_themselves_and_check_their_values() {
        for k in ShapeKind::ALL {
            assert_eq!(ShapeKind::parse(k.name()), Some(k));
        }
        let s = Shape::new(ShapeKind::Star, [200.0, 200.0], [1.0, 0.8, 0.0, 1.0]);
        assert!(s.problem().is_none() && s.fill.is_some() && s.stroke.is_none());
        let line = Shape::new(ShapeKind::Line, [200.0, 0.0], [1.0; 4]);
        assert!(line.fill.is_none() && line.stroke.is_some());
        assert!(Shape { points: 2, ..s.clone() }.problem().is_some());
        assert!(Shape { trim: [0.0, 1.5], ..s }.problem().is_some());
    }
}
