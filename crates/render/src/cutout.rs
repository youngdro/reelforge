//! `key.person@1`: the people in a layer's picture kept, the rest made see-through.
//!
//! Unlike a colour key this cannot be decided pixel by pixel: a segmentation model
//! (`reelforge-matte`) looks at the whole picture. So it is done to the picture before a
//! compositor places it, as a blur is, and CPU and GPU draw the same cut-out.
//!
//! Working one out takes tens of milliseconds, and a paused preview draws the same picture again
//! and again as its layer is moved, so the last few are kept for as long as the pictures they
//! were cut from are. Something that knows which pictures are drawn next (an editor reading ahead
//! of its playhead) can call [`person`] early to have them ready.

use std::sync::{Arc, Mutex, Weak};

use crate::Image;

/// Cut-outs kept at most.
const KEPT: usize = 8;
/// Pixels of cut-outs kept at most (16 bytes each), whatever their number; the newest always is.
const KEPT_PIXELS: usize = 12_000_000;

/// Cut-outs by the picture they were made from, oldest first.
static MADE: Mutex<Vec<(Weak<Image>, Arc<Image>)>> = Mutex::new(Vec::new());

/// `picture` with everything but the people in it see-through. The same picture (the same `Arc`)
/// asked for again is answered from the few kept.
pub fn person(picture: &Arc<Image>) -> Result<Arc<Image>, String> {
    let kept = |made: &Vec<(Weak<Image>, Arc<Image>)>| {
        made.iter().find(|(from, _)| std::ptr::eq(from.as_ptr(), Arc::as_ptr(picture))).map(|(_, cut)| cut.clone())
    };
    if let Some(cut) = kept(&MADE.lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(cut);
    }
    // not under the lock: several pictures may be cut out at once
    let cut = Arc::new(cut_out(picture)?);
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(first) = kept(&made) {
        return Ok(first);
    }
    // a cut-out whose picture is gone cannot be asked for again
    made.retain(|(from, _)| from.strong_count() > 0);
    made.push((Arc::downgrade(picture), cut.clone()));
    let pixels = |made: &[(Weak<Image>, Arc<Image>)]| made.iter().map(|(_, cut)| cut.pixels.len()).sum::<usize>();
    while made.len() > 1 && (made.len() > KEPT || pixels(&made) > KEPT_PIXELS) {
        made.remove(0);
    }
    Ok(cut)
}

/// `picture` times how much of each pixel is a person.
fn cut_out(picture: &Image) -> Result<Image, String> {
    let (w, h) = (picture.width as usize, picture.height as usize);
    let alpha = reelforge_matte::person_linear(w, h, &picture.pixels)?;
    let pixels = picture.pixels.iter().zip(&alpha).map(|(p, a)| p.map(|v| v * a)).collect();
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
        assert!(made.iter().any(|(from, _)| std::ptr::eq(from.as_ptr(), Arc::as_ptr(&last))));
        assert!(!made.iter().any(|(from, _)| from.strong_count() == 0));
    }
}
