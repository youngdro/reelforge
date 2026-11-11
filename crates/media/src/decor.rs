//! The built-in decorations: small Lottie animations drawn for this library (confetti, sparkles,
//! a tap ring, a drawn underline…), so they need no downloads and no licences. A project refers
//! to one as `library:decor/<id>`, a Lottie animation like any other; [`Footage`] reads it from
//! the program itself, the same everywhere, the browser included.
//!
//! Each is drawn in two placeholder colours, red and yellow, on fills and strokes called
//! `primary` and `secondary`. A URI may say other ones, as a brand would have them:
//! `library:decor/pulse?primary=1463ff&secondary=ffd21f` (sRGB, two hex digits a channel). The
//! parts in neither colour (white paper, a coin's shine) stay as drawn.
//!
//! Lottie footage goes round again past its end, so an animation that plays once (a burst, a
//! line being drawn) is ten seconds long: still, or empty, after its moment.
//!
//! [`Footage`]: crate::Footage

use reelforge_lottie::Animation;

/// The URI prefix of library decorations (within [`crate::library::SCHEME`]).
pub const PREFIX: &str = "library:decor/";

/// A library decoration.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LibraryDecor {
    /// The id commands use.
    pub id: &'static str,
    /// English name.
    pub en: &'static str,
    /// Chinese name.
    pub zh: &'static str,
    /// How long a clip of it is unless told, in seconds.
    pub seconds: f32,
    /// Whether it plays once and then stays (or is gone), rather than going round.
    pub once: bool,
    /// How large it starts, 1 fitting the canvas; `None` covers the canvas, as falling confetti
    /// should.
    pub scale: Option<f32>,
    /// The frame that shows it best, for a picture of it.
    pub telling: f32,
    /// Which of the two colours it is drawn in: the primary, the secondary.
    pub colours: [bool; 2],
    /// Its Bodymovin JSON.
    json: &'static str,
}

macro_rules! decor {
    ($id:literal, $en:literal, $zh:literal, $seconds:literal, $once:literal, $scale:expr, $telling:literal, $colours:expr) => {
        LibraryDecor {
            id: $id,
            en: $en,
            zh: $zh,
            seconds: $seconds,
            once: $once,
            scale: $scale,
            telling: $telling,
            colours: $colours,
            json: include_str!(concat!("../decor/", $id, ".json")),
        }
    };
}

/// Every decoration, in panel order.
pub const DECOR: [LibraryDecor; 20] = [
    decor!("confettiBurst", "Confetti burst", "彩纸喷发", 3.0, true, Some(1.0), 22.0, [true, true]),
    decor!("confettiRain", "Confetti rain", "彩纸雨", 3.0, false, None, 40.0, [true, true]),
    decor!("fireworks", "Fireworks", "烟花", 2.4, false, Some(0.8), 16.0, [true, true]),
    decor!("sparkles", "Sparkles", "闪烁星光", 3.2, false, Some(0.5), 27.0, [false, true]),
    decor!("starBurst", "Star burst", "星星迸发", 2.0, true, Some(0.5), 14.0, [true, true]),
    decor!("heartBurst", "Heart burst", "爱心迸发", 2.0, true, Some(0.45), 14.0, [true, true]),
    decor!("tapRing", "Tap here", "点击提示", 4.0, false, Some(0.3), 18.0, [true, false]),
    decor!("pulse", "Pulse", "脉冲光圈", 3.0, false, Some(0.3), 20.0, [true, false]),
    decor!("arrowBounce", "Bouncing arrow", "跳动箭头", 3.0, false, Some(0.2), 8.0, [true, false]),
    decor!("swipeUp", "Swipe up", "上滑提示", 3.0, false, Some(0.25), 22.0, [true, false]),
    decor!("emphasis", "Emphasis strokes", "强调线", 2.0, false, Some(0.25), 8.0, [false, true]),
    decor!("underline", "Drawn underline", "手绘下划线", 3.0, true, Some(0.6), 40.0, [true, false]),
    decor!("circleMark", "Drawn circle", "手绘圈重点", 3.0, true, Some(0.6), 40.0, [true, false]),
    decor!("checkDraw", "Drawn check", "打勾", 3.0, true, Some(0.25), 40.0, [true, false]),
    decor!("coinDrop", "Falling coins", "金币掉落", 4.0, false, Some(0.8), 30.0, [false, true]),
    decor!("flame", "Flame", "火焰", 3.0, false, Some(0.25), 8.0, [true, true]),
    decor!("bolt", "Lightning strike", "闪电", 2.0, false, Some(0.25), 10.0, [false, true]),
    decor!("giftShake", "Shaking gift", "礼盒晃动", 4.0, false, Some(0.4), 20.0, [true, true]),
    decor!("clockTick", "Ticking clock", "时钟走动", 4.0, false, Some(0.25), 20.0, [true, false]),
    decor!("loadingDots", "Hopping dots", "跳动圆点", 2.4, false, Some(0.25), 9.0, [true, false]),
];

/// The decoration with this id.
pub fn find(id: &str) -> Option<&'static LibraryDecor> {
    DECOR.iter().find(|d| d.id == id)
}

/// The decoration a `library:decor/` URI names, whatever colours it says.
pub fn of(uri: &str) -> Option<&'static LibraryDecor> {
    let rest = uri.strip_prefix(PREFIX)?;
    find(rest.split_once('?').map_or(rest, |(id, _)| id))
}

/// A colour as six hex digits, each channel rounded to a 255th.
fn hex(c: [f32; 3]) -> String {
    c.iter().map(|v| format!("{:02x}", (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect()
}

/// The colour six hex digits say.
fn unhex(s: &str) -> Option<[f32; 3]> {
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok().map(|v| f32::from(v) / 255.0);
    Some([channel(0)?, channel(1)?, channel(2)?])
}

/// The URI of a decoration in these colours (straight sRGB, 0 to 1); `None` leaves that colour
/// as drawn. A colour the decoration is not drawn in is left out, so that one decoration in one
/// look has one URI.
pub fn uri(decor: &LibraryDecor, primary: Option<[f32; 3]>, secondary: Option<[f32; 3]>) -> String {
    let said: Vec<String> = [("primary", primary), ("secondary", secondary)]
        .into_iter()
        .zip(decor.colours)
        .filter_map(|((name, colour), used)| Some(format!("{name}={}", hex(colour.filter(|_| used)?))))
        .collect();
    match said.is_empty() {
        true => format!("{PREFIX}{}", decor.id),
        false => format!("{PREFIX}{}?{}", decor.id, said.join("&")),
    }
}

/// The colours a URI says, primary and secondary.
pub fn colours(uri: &str) -> [Option<[f32; 3]>; 2] {
    let said = uri.split_once('?').map_or("", |(_, said)| said);
    let colour = |name: &str| {
        said.split('&').filter_map(|pair| pair.split_once('=')).find(|(n, _)| *n == name).and_then(|(_, v)| unhex(v))
    };
    [colour("primary"), colour("secondary")]
}

/// The Bodymovin JSON of the decoration a URI names, in the colours it says: the same bytes for
/// the same URI, so its hash is stable.
pub fn file(uri: &str) -> Option<Vec<u8>> {
    let decor = of(uri)?;
    let [primary, secondary] = colours(uri);
    if primary.is_none() && secondary.is_none() {
        return Some(decor.json.as_bytes().to_vec());
    }
    let given: Vec<(&str, [f32; 3])> =
        [("primary", primary), ("secondary", secondary)].into_iter().filter_map(|(n, c)| Some((n, c?))).collect();
    reelforge_lottie::tinted(decor.json, &given).map(String::into_bytes)
}

/// The animation a URI names, read.
pub fn open(uri: &str) -> Option<Animation> {
    crate::lottie::open(&file(uri)?, None).ok()
}

/// The size pictures of a decoration are drawn at: as any animation's ([`picture_size`]), but
/// one that covers the canvas is drawn 1920 on its longer side.
///
/// [`picture_size`]: crate::lottie::picture_size
pub fn picture_size(decor: &LibraryDecor, a: &Animation) -> [u32; 2] {
    match decor.scale {
        Some(_) => crate::lottie::picture_size(a),
        None => {
            let k = 1920.0 / a.width.max(a.height);
            let even = |v: f32| (((v * k).round() as u32).max(2) / 2) * 2;
            [even(a.width), even(a.height)]
        }
    }
}

/// A decoration read once to be shown in a panel: still at its telling frame, or playing.
pub struct Preview {
    decor: &'static LibraryDecor,
    animation: Animation,
}

impl Preview {
    /// Reads a decoration, as drawn.
    pub fn of(decor: &'static LibraryDecor) -> Option<Preview> {
        Some(Preview { decor, animation: open(&uri(decor, None, None))? })
    }

    /// Which decoration it is.
    pub fn decor(&self) -> &'static LibraryDecor {
        self.decor
    }

    /// Its picture `side` pixels on its longer side: `seconds` into playing it (going round as
    /// a clip of its usual length would), or at its telling frame.
    pub fn picture(&self, seconds: Option<f32>, side: u32) -> crate::Thumbnail {
        let a = &self.animation;
        let k = side.clamp(1, 2048) as f32 / a.width.max(a.height);
        let (width, height) = (((a.width * k).round() as u32).max(1), ((a.height * k).round() as u32).max(1));
        let frame = match seconds {
            // seconds only here, where the animation's own frames begin
            Some(t) => a.frame_at(f64::from(t.max(0.0) % self.decor.seconds)),
            None => self.decor.telling,
        };
        crate::Thumbnail { width, height, rgba: a.render(frame, width, height) }
    }
}

/// A picture of a decoration at its telling frame, as drawn, `side` pixels on its longer side.
pub fn picture(decor: &'static LibraryDecor, side: u32) -> Option<crate::Thumbnail> {
    Some(Preview::of(decor)?.picture(None, side))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_decoration_is_read_whole_and_draws_something_at_its_telling_frame() {
        for d in &DECOR {
            let a = open(&uri(d, None, None)).unwrap_or_else(|| panic!("{}", d.id));
            assert!(a.unsupported().is_empty(), "{}: {:?}", d.id, a.unsupported());
            let p = picture(d, 96).unwrap();
            assert_eq!(p.width.max(p.height), 96, "{}", d.id);
            assert!(p.rgba.as_chunks::<4>().0.iter().filter(|px| px[3] > 128).count() > 40, "{} shows nothing", d.id);
            // one that plays once is long enough to stay still past any clip of it
            assert_eq!(d.once, a.seconds() >= 10.0, "{}", d.id);
            assert!(d.seconds > 0.0 && d.scale.is_none_or(|s| s > 0.0 && s <= 1.0), "{}", d.id);
            // it says which colours it is drawn in
            for (name, used) in ["\"nm\":\"primary\"", "\"nm\":\"secondary\""].into_iter().zip(d.colours) {
                assert_eq!(d.json.contains(name), used, "{} {name}", d.id);
            }
        }
        let mut ids: Vec<&str> = DECOR.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DECOR.len());
    }

    #[test]
    fn a_uri_says_which_decoration_in_which_colours() {
        let pulse = find("pulse").unwrap();
        assert_eq!(uri(pulse, None, None), "library:decor/pulse");
        // pulse is drawn in the primary only: the secondary is not said
        let blue = uri(pulse, Some([0.08, 0.39, 1.0]), Some([1.0, 0.82, 0.12]));
        assert_eq!(blue, "library:decor/pulse?primary=1463ff");
        assert_eq!(of(&blue), Some(pulse));
        assert_eq!(colours(&blue), [Some([20.0 / 255.0, 99.0 / 255.0, 1.0]), None]);
        assert_eq!(of("library:decor/none"), None);
        assert_eq!(of("library:sticker/heart"), None);
        assert_eq!(colours("library:decor/pulse?primary=zzzzzz&secondary=fff"), [None, None]);
        // its rings are the colour said, where they were red
        let at = |uri: &str| {
            let a = open(uri).unwrap();
            let f = a.render(pulse.telling, 100, 100);
            let px = f.as_chunks::<4>().0.iter().max_by_key(|px| px[3]).unwrap();
            [px[0], px[1], px[2]]
        };
        let (red, said) = (at("library:decor/pulse"), at(&blue));
        assert!(red[0] > 200 && red[2] < 100, "{red:?}");
        assert!(said[2] > 200 && said[0] < 100, "{said:?}");
        // the same bytes each time
        assert_eq!(file(&blue), file(&blue));
        assert_ne!(file(&blue), file("library:decor/pulse"));
    }

    #[test]
    fn footage_draws_a_decoration_with_no_file_at_the_size_its_asset_says() {
        use reelforge_render::MediaSource;
        use reelforge_renderspec::{Asset, AssetKind, Probe};
        use reelforge_time::Tick;

        let rain = find("confettiRain").unwrap();
        let uri = uri(rain, Some([0.0, 0.0, 1.0]), None);
        let size = picture_size(rain, &open(&uri).unwrap());
        assert_eq!(size, [1440, 1920]);
        let asset = Asset {
            kind: AssetKind::Lottie,
            sha256: "d".repeat(64),
            uri,
            of: None,
            probe: Some(Probe { width: Some(size[0]), height: Some(size[1]), duration: Some(Tick::SECOND * 3) }),
        };
        let media = crate::Footage::new();
        let img = media.picture_shrunk("d", &asset, Tick::SECOND, 4).expect("picture").expect("drawn");
        assert_eq!((img.width, img.height), (360, 480));
    }
}
