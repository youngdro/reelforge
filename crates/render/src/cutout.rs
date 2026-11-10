//! `key.person@1` and `key.backdrop@1`: the people in a layer's picture, or what stands before
//! its plain backdrop, kept, and the rest made see-through.
//!
//! Unlike a colour key these cannot be decided pixel by pixel: something looks at the whole
//! picture (`reelforge-matte`: a segmentation model for people, the picture's own edges for a
//! backdrop). So it is done to the picture before a compositor places it, as a blur is, and CPU
//! and GPU draw the same cut-out.
//!
//! Working one out takes tens of milliseconds, and a paused preview draws the same picture again
//! and again as its layer is moved, so the last few are kept for as long as the pictures they
//! were cut from are. Something that knows which pictures are drawn next (an editor reading ahead
//! of its playhead) can call [`of`] early to have them ready.

use std::sync::{Arc, Mutex, Weak};

use reelforge_renderspec::effects;

use crate::Image;

/// Cut-outs kept at most.
const KEPT: usize = 8;
/// Pixels of cut-outs kept at most (16 bytes each), whatever their number; the newest always is.
const KEPT_PIXELS: usize = 12_000_000;

/// How a picture is cut out: the people in it kept, or what stands before its backdrop (told
/// from it with this tolerance and readiness to open holes, as bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum How {
    Person,
    Backdrop(u32, u32),
}

/// One kept: the picture it was made from, how, and the cut-out.
type Made = (Weak<Image>, How, Arc<Image>);

/// Cut-outs by the picture they were made from, oldest first.
static MADE: Mutex<Vec<Made>> = Mutex::new(Vec::new());

/// `picture` with everything but the people in it see-through. The same picture (the same `Arc`)
/// asked for again is answered from the few kept.
pub fn person(picture: &Arc<Image>) -> Result<Arc<Image>, String> {
    kept_or_made(picture, How::Person)
}

/// `picture` with its plain backdrop see-through (`key.backdrop@1` with this `tolerance` and
/// `holes`). The same picture asked for again in the same way is answered from the few kept.
pub fn backdrop(picture: &Arc<Image>, tolerance: f32, holes: f32) -> Result<Arc<Image>, String> {
    kept_or_made(picture, How::Backdrop(tolerance.to_bits(), holes.to_bits()))
}

/// `picture` as a layer's effects cut it out (the effects as a frame plan lists them, each with
/// its parameters at the frame's instant): its people kept, then its backdrop gone. A layer with
/// neither gets the picture back.
pub fn of(fx: &[(String, Vec<(String, f32)>)], picture: &Arc<Image>) -> Result<Arc<Image>, String> {
    let mut out = picture.clone();
    if fx.iter().any(|(id, _)| id == effects::PERSON_KEY) {
        out = person(&out)?;
    }
    if let Some((_, params)) = fx.iter().find(|(id, _)| id == effects::BACKDROP_KEY) {
        let get = |name: &str| params.iter().find(|(k, _)| k == name).map_or(0.5, |(_, v)| v.clamp(0.0, 1.0));
        out = backdrop(&out, get("tolerance"), get("holes"))?;
    }
    Ok(out)
}

/// Whether effects cut a picture out ([`of`] changes it).
pub fn cuts(fx: &[(String, Vec<(String, f32)>)]) -> bool {
    fx.iter().any(|(id, _)| id == effects::PERSON_KEY || id == effects::BACKDROP_KEY)
}

fn kept_or_made(picture: &Arc<Image>, how: How) -> Result<Arc<Image>, String> {
    let kept = |made: &Vec<Made>| {
        made.iter()
            .find(|(from, as_, _)| *as_ == how && std::ptr::eq(from.as_ptr(), Arc::as_ptr(picture)))
            .map(|(_, _, cut)| cut.clone())
    };
    if let Some(cut) = kept(&MADE.lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(cut);
    }
    // not under the lock: several pictures may be cut out at once
    let cut = Arc::new(cut_out(picture, how)?);
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(first) = kept(&made) {
        return Ok(first);
    }
    // a cut-out whose picture is gone cannot be asked for again
    made.retain(|(from, _, _)| from.strong_count() > 0);
    made.push((Arc::downgrade(picture), how, cut.clone()));
    let pixels = |made: &[Made]| made.iter().map(|(_, _, cut)| cut.pixels.len()).sum::<usize>();
    while made.len() > 1 && (made.len() > KEPT || pixels(&made) > KEPT_PIXELS) {
        made.remove(0);
    }
    Ok(cut)
}

/// `picture` times how much of each pixel is a person, or with its backdrop taken out.
fn cut_out(picture: &Image, how: How) -> Result<Image, String> {
    let (w, h) = (picture.width as usize, picture.height as usize);
    let pixels = match how {
        How::Person => {
            let alpha = reelforge_matte::person_linear(w, h, &picture.pixels)?;
            picture.pixels.iter().zip(&alpha).map(|(p, a)| p.map(|v| v * a)).collect()
        }
        How::Backdrop(tolerance, holes) => {
            let how = reelforge_matte::Backdrop { tolerance: f32::from_bits(tolerance), holes: f32::from_bits(holes) };
            reelforge_matte::backdrop_linear(w, h, &picture.pixels, how)?.pixels
        }
    };
    Ok(Image { width: picture.width, height: picture.height, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_out_is_the_picture_times_its_matte_and_the_same_picture_is_cut_out_once() {
        let stripes = |shift: u32| {
            let mut img = Image::filled(96, 64, [0.0, 0.0, 0.0, 1.0]);
            for (i, p) in img.pixels.iter_mut().enumerate() {
                let x = i as u32 % 96;
                *p = if ((x + shift) / 8).is_multiple_of(2) { [0.1, 0.4, 0.8, 1.0] } else { [0.4, 0.35, 0.05, 0.5] };
            }
            Arc::new(img)
        };
        let picture = stripes(0);
        let cut = person(&picture).expect("cut out");
        assert_eq!((cut.width, cut.height, cut.pixels.len()), (96, 64, 96 * 64));
        // every pixel, its alpha too, times what the model and the fitting answer for it
        let matte = reelforge_matte::person_linear(96, 64, &picture.pixels).expect("a matte");
        for ((from, to), m) in picture.pixels.iter().zip(&cut.pixels).zip(&matte) {
            assert_eq!(*to, from.map(|v| v * m));
        }
        // again: the one made before, not another
        assert!(Arc::ptr_eq(&cut, &person(&picture).expect("kept")));
        // another picture with the same pixels is another picture
        let twin = Arc::new((*picture).clone());
        assert!(!Arc::ptr_eq(&cut, &person(&twin).expect("cut out")));
        // many more than are kept: the first is made anew when asked for again, and what is
        // kept stays within its bounds
        let many: Vec<Arc<Image>> = (0..KEPT as u32 + 3).map(stripes).collect();
        for p in &many {
            person(p).expect("cut out");
        }
        assert!(MADE.lock().unwrap().len() <= KEPT);
        assert!(!Arc::ptr_eq(&cut, &person(&picture).expect("made anew")));
        // those whose pictures are gone are let go of with the next made
        drop((many, twin));
        let last = stripes(99);
        person(&last).expect("cut out");
        let made = MADE.lock().unwrap();
        assert!(made.iter().any(|(from, _, _)| std::ptr::eq(from.as_ptr(), Arc::as_ptr(&last))));
        assert!(!made.iter().any(|(from, _, _)| from.strong_count() == 0));
    }
}
