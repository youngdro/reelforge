//! The keyframe curve: how a clip's values travel from the keyframes at one instant on to the next
//! ones, drawn as a graph. Across is time between the two, up is how far the values have got.
//! Named curves are a click; the two handles shape one by hand (a cubic Bézier, as CSS has).
//!
//! The panel only reads the clip and says what was asked for; the editor sends the command.

use eframe::egui::{self, Pos2, Rect, Sense, Stroke, pos2, vec2};
use reelforge_model::{Clip, Ease};
use reelforge_time::Tick;

use crate::i18n::Strings;
use crate::theme::{self, color, icon};

/// How far the graph reaches below the first key's value and above the next one's: handles can be
/// pulled this far, which is as much overshoot as reads as a movement.
const BELOW: f32 = -0.6;
const ABOVE: f32 = 1.6;

/// The stretch between keyframes that the playhead is in, in the clip's own time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Segment {
    /// Where the keyframes it leaves from are.
    pub from: Tick,
    /// Where the next keyframe is.
    pub to: Tick,
    /// How the values travel.
    pub ease: Ease,
    /// Whether the properties keyed at `from` travel differently from one another.
    pub mixed: bool,
}

/// Every instant of `clip` with a keyframe, in order.
pub(crate) fn key_times(clip: &Clip) -> Vec<Tick> {
    let mut times: Vec<Tick> = clip.keys.values().flatten().map(|k| k.t).collect();
    times.sort();
    times.dedup();
    times
}

/// The stretch `t` is in: from the latest keyframes at or before it that have one after them. Before
/// the first keyframe that is the first stretch, and after the last, the last.
pub(crate) fn segment(clip: &Clip, t: Tick) -> Option<Segment> {
    // per property, the pair of keys around `t`
    let pairs: Vec<_> = clip
        .keys
        .values()
        .filter(|list| list.len() >= 2)
        .map(|list| {
            let i = list.windows(2).position(|w| t < w[1].t).unwrap_or(list.len() - 2);
            (list[i], list[i + 1])
        })
        .collect();
    let from = pairs.iter().map(|(a, _)| a.t).filter(|a| *a <= t).max().or(pairs.iter().map(|(a, _)| a.t).min())?;
    let here: Vec<_> = pairs.iter().filter(|(a, _)| a.t == from).collect();
    let (first, _) = here.first()?;
    Some(Segment {
        from,
        to: here.iter().map(|(_, b)| b.t).min()?,
        ease: first.ease,
        mixed: here.iter().any(|(a, _)| a.ease != first.ease),
    })
}

/// What the panel asks for.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Ask {
    /// Move the playhead to this instant of the clip.
    Jump(Tick),
    /// Shape the way on from the keyframes at this instant of the clip.
    Shape(Tick, Ease),
}

/// The keyframes panel for `clip` with the playhead at `t` (in the clip's time); a keyframe within
/// `half` a frame of the playhead is under it.
pub(crate) fn show(ui: &mut egui::Ui, s: &Strings, clip: &Clip, t: Tick, half: i64) -> Option<Ask> {
    let times = key_times(clip);
    let mut ask = None;
    ui.horizontal(|ui| {
        let before = times.iter().rev().find(|k| k.0 < t.0 - half).copied();
        let after = times.iter().find(|k| k.0 > t.0 + half).copied();
        for (to, glyph, tip) in [(before, icon::FRAME_BACK, s.key_prev), (after, icon::FRAME_ON, s.key_next)] {
            let r = ui.add_enabled(to.is_some(), egui::Button::new(glyph).small()).on_hover_text(tip);
            if let (true, Some(to)) = (r.clicked(), to) {
                ask = Some(Ask::Jump(to));
            }
        }
        // which keyframe the playhead is on, or between which
        let on = times.iter().position(|k| (k.0 - t.0).abs() <= half);
        let label = match on {
            Some(i) => format!("{} / {}", i + 1, times.len()),
            None => format!("– / {}", times.len()),
        };
        ui.label(egui::RichText::new(label).monospace().color(if on.is_some() { color::ACCENT } else { color::WEAK }));
    });
    let Some(seg) = segment(clip, t) else {
        ui.label(egui::RichText::new(s.ease_none).small().color(color::WEAK));
        return ask;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(s.ease_curve).color(color::WEAK));
        let name = match seg.ease.name().and_then(|n| Ease::NAMED.iter().position(|(x, _)| *x == n)) {
            _ if seg.mixed => s.ease_mixed,
            Some(i) => s.ease_names[i],
            None => s.ease_custom,
        };
        ui.label(name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let span = |t: Tick| theme::timecode((clip.start + t).seconds());
            ui.label(
                egui::RichText::new(format!("{} → {}", span(seg.from), span(seg.to)))
                    .small()
                    .monospace()
                    .color(color::WEAK),
            );
        });
    });
    // the named curves, each drawn as it goes
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, (_, ease)) in Ease::NAMED.into_iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(30.0, 24.0), Sense::click());
            let chosen = !seg.mixed && seg.ease == ease;
            let (fill, line) = match (chosen, r.hovered()) {
                (true, _) => (color::ACCENT.gamma_multiply(0.22), color::ACCENT),
                (false, true) => (color::HOVER, color::TEXT),
                (false, false) => (color::RAISED, color::WEAK),
            };
            ui.painter().rect_filled(rect, 4.0, fill);
            trace(ui.painter(), rect.shrink2(vec2(6.0, 7.0)), ease, 0.0, 1.0, Stroke::new(1.4, line));
            if r.on_hover_text(s.ease_names[i]).clicked() {
                ask = Some(Ask::Shape(seg.from, ease));
            }
        }
    });
    ui.add_space(4.0);
    // where the playhead is along the stretch
    let along = ((t - seg.from).0 as f64 / (seg.to - seg.from).0.max(1) as f64).clamp(0.0, 1.0) as f32;
    if let Some(ease) = graph(ui, &clip.id, seg, along).on_hover(s.ease_tip) {
        ask = Some(Ask::Shape(seg.from, ease));
    }
    ask
}

/// What the graph gave back: the curve its handles were let go at, and where to hang a hint.
struct Graphed {
    done: Option<Ease>,
    response: egui::Response,
}

impl Graphed {
    fn on_hover(self, tip: &str) -> Option<Ease> {
        self.response.on_hover_text(tip);
        self.done
    }
}

/// The graph of the curve of `seg` (a stretch of `clip`) with its two handles, and the playhead
/// `along` it (0 to 1). A handle
/// being dragged shapes the curve on screen only; the curve is given back when it is let go.
fn graph(ui: &mut egui::Ui, clip: &str, seg: Segment, along: f32) -> Graphed {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 150.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, color::ROW);
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, color::LINE), egui::StrokeKind::Inside);
    // the unit square: time across, the value's way up; room above and below for overshoot
    let plot = rect.shrink2(vec2(12.0, 8.0));
    let at = |x: f32, y: f32| {
        pos2(plot.left() + x * plot.width(), plot.bottom() - (y - BELOW) / (ABOVE - BELOW) * plot.height())
    };
    for y in [0.0, 1.0] {
        painter.line_segment([at(0.0, y), at(1.0, y)], Stroke::new(1.0, color::LINE));
    }
    for x in [0.0, 1.0] {
        painter.line_segment([at(x, 0.0), at(x, 1.0)], Stroke::new(1.0, color::LINE));
    }
    painter.line_segment([at(0.0, 0.0), at(1.0, 1.0)], Stroke::new(1.0, color::LINE.gamma_multiply(0.8)));

    // the curve as held, or as the handle in hand has it
    let id = egui::Id::unique(("keyframe curve", clip, seg.from.0));
    let held: Option<[f32; 4]> = ui.data(|d| d.get_temp(id));
    let mut points = held.or(seg.ease.points());
    let mut done = None;
    if let Some(p) = &mut points {
        let mut moved = false;
        let mut dropped = false;
        for i in 0..2 {
            let centre = at(p[i * 2], p[i * 2 + 1]);
            let r = ui.interact(Rect::from_center_size(centre, vec2(18.0, 18.0)), id.with(i), Sense::drag());
            if r.dragged()
                && let Some(pointer) = r.interact_pointer_pos()
            {
                p[i * 2] = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                p[i * 2 + 1] =
                    (BELOW + (plot.bottom() - pointer.y) / plot.height() * (ABOVE - BELOW)).clamp(BELOW, ABOVE);
                moved = true;
            }
            dropped |= r.drag_stopped();
            let anchor = if i == 0 { at(0.0, 0.0) } else { at(1.0, 1.0) };
            let centre = at(p[i * 2], p[i * 2 + 1]);
            let hot = r.hovered() || r.dragged();
            painter.line_segment([anchor, centre], Stroke::new(1.0, color::WEAK));
            painter.circle(centre, if hot { 5.5 } else { 4.5 }, color::PANEL, Stroke::new(1.6, color::ACCENT));
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
        if moved {
            ui.data_mut(|d| d.insert_temp(id, *p));
        }
        if dropped {
            ui.data_mut(|d| d.remove::<[f32; 4]>(id));
            // to a hundredth: as fine as a hand places it, and short to read in a project file
            done = Some(Ease::bezier(p.map(|v| (v * 100.0).round() / 100.0)));
        }
    }
    let shown = points.map_or(seg.ease, Ease::Bezier);
    trace(&painter, Rect::from_two_pos(at(0.0, 0.0), at(1.0, 1.0)), shown, 0.0, 1.0, Stroke::new(2.0, color::ACCENT));
    for (x, y) in [(0.0, 0.0), (1.0, 1.0)] {
        crate::timeline::diamond(&painter, at(x, y), 4.0, color::ACCENT, Stroke::NONE);
    }
    // the playhead, on the curve
    let y = if along >= 1.0 { 1.0 } else { shown.apply(along) };
    painter.line_segment([at(along, BELOW), at(along, ABOVE)], Stroke::new(1.0, color::PLAYHEAD.gamma_multiply(0.5)));
    painter.circle_filled(at(along, y), 3.0, color::PLAYHEAD);
    Graphed { done, response }
}

/// Draws `ease` across `unit`, whose bottom-left is the key it leaves and top-right the next one.
/// `lo` and `hi` are the fractions of time drawn.
fn trace(painter: &egui::Painter, unit: Rect, ease: Ease, lo: f32, hi: f32, stroke: Stroke) {
    let at = |x: f32, y: f32| pos2(unit.left() + x * unit.width(), unit.bottom() - y * unit.height());
    if ease == Ease::Hold {
        // level, then up at the next key
        painter.add(egui::Shape::line(vec![at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0)], stroke));
        return;
    }
    const STEPS: usize = 40;
    let line: Vec<Pos2> = (0..=STEPS)
        .map(|i| {
            let x = lo + (hi - lo) * i as f32 / STEPS as f32;
            at(x, ease.apply(x))
        })
        .collect();
    painter.add(egui::Shape::line(line, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_model::{Key, Prop};

    fn clip() -> Clip {
        let mut s = reelforge_engine::Session::default();
        let track = s.project().tracks[0].id.clone();
        let r = s
            .execute("clip.add", &serde_json::json!({"track": track, "color": [1, 0, 0], "duration": {"seconds": 2}}))
            .expect("a clip");
        s.project().clip(r["clip"].as_str().expect("its id")).expect("just added").1.clone()
    }

    #[test]
    fn the_stretch_under_the_playhead_is_the_one_its_latest_keys_start() {
        let mut c = clip();
        let key = |t: i64, ease: Ease| Key { t: Tick(t), v: t as f32, ease };
        assert_eq!(segment(&c, Tick(5)), None);
        c.keys.insert(Prop::X, vec![key(100, Ease::IN), key(400, Ease::Linear), key(800, Ease::Linear)]);
        c.keys.insert(Prop::Scale, vec![key(100, Ease::IN), key(600, Ease::Hold), key(800, Ease::Linear)]);
        // a single key is no stretch
        c.keys.insert(Prop::Opacity, vec![key(300, Ease::OUT)]);
        assert_eq!(key_times(&c), [100, 300, 400, 600, 800].map(Tick));

        let at = |c: &Clip, t: i64| segment(c, Tick(t)).unwrap();
        // both properties leave 100 the same way; the stretch ends at the nearer next key
        assert_eq!(at(&c, 250), Segment { from: Tick(100), to: Tick(400), ease: Ease::IN, mixed: false });
        // on a key, the stretch it starts
        assert_eq!(at(&c, 400).from, Tick(400));
        assert_eq!((at(&c, 500).from, at(&c, 500).to, at(&c, 500).ease), (Tick(400), Tick(800), Ease::Linear));
        assert_eq!((at(&c, 700).from, at(&c, 700).ease), (Tick(600), Ease::Hold));
        // before the first key and after the last, the nearest stretch
        assert_eq!(at(&c, 0).from, Tick(100));
        assert_eq!(at(&c, 900).from, Tick(600));
        // keys at one instant that travel differently are said to
        c.keys.get_mut(&Prop::Scale).unwrap()[0].ease = Ease::OUT;
        assert!(at(&c, 250).mixed);
    }
}
