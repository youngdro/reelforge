use super::*;

struct NoFiles;

impl Files for NoFiles {
    fn decode(&self, _: &[u8]) -> Option<Bitmap> {
        None
    }
}

fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * w + x) * 4) as usize;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

/// A red square that slides right and fades in, on a 100 × 100 picture, 30 frames at 30 a second.
const SLIDE: &str = r#"{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[
  {"ty":4,"ind":1,"ip":0,"op":30,"st":0,"ks":{"p":{"a":1,"k":[{"t":0,"s":[20,50]},{"t":30,"s":[80,50]}]},
      "o":{"a":0,"k":100}},
   "shapes":[{"ty":"gr","it":[
      {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":0}},
      {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}
]}"#;

#[test]
fn a_shape_layer_moves_as_its_keys_say() {
    let a = Animation::open(SLIDE.as_bytes(), &NoFiles).unwrap();
    assert_eq!((a.width, a.height, a.fps, a.frames()), (100.0, 100.0, 30.0, 30.0));
    assert!((a.seconds() - 1.0).abs() < 1e-9 && a.unsupported().is_empty());
    let f0 = a.render(0.0, 100, 100);
    assert_eq!(pixel(&f0, 100, 20, 50), [255, 0, 0, 255]);
    assert_eq!(pixel(&f0, 100, 50, 50), [0, 0, 0, 0]);
    let f15 = a.render(15.0, 100, 100);
    assert_eq!(pixel(&f15, 100, 50, 50), [255, 0, 0, 255]);
    assert_eq!(pixel(&f15, 100, 20, 50), [0, 0, 0, 0]);
    // twice the size, and it goes round again past its end
    let big = a.render(a.frame_at(1.5), 200, 200);
    assert_eq!(pixel(&big, 200, 100, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&big, 200, 100, 70), [0, 0, 0, 0]);
}

#[test]
fn masks_mattes_and_precomps_cut_what_shows() {
    // a full blue solid, masked to its left half, shown through a circle in the middle
    let json = r##"{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,
      "assets":[{"id":"inner","layers":[
        {"ty":1,"ind":1,"ip":0,"op":300,"st":0,"sw":100,"sh":100,"sc":"#00ff00","ks":{}}]}],
      "layers":[
      {"ty":4,"ind":1,"ip":0,"op":30,"st":0,"td":1,"ks":{"p":{"a":0,"k":[50,50]}},
       "shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[60,60]}},{"ty":"fl","c":{"a":0,"k":[1,1,1,1]},"o":{"a":0,"k":100}}]},
      {"ty":1,"ind":2,"ip":0,"op":30,"st":0,"tt":1,"sw":100,"sh":100,"sc":"#0000ff","ks":{},
       "masksProperties":[{"mode":"a","inv":false,"o":{"a":0,"k":100},
         "pt":{"a":0,"k":{"c":true,"v":[[0,0],[50,0],[50,100],[0,100]],"i":[[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0]]}}}]},
      {"ty":0,"ind":3,"ip":0,"op":30,"st":0,"refId":"inner","w":100,"h":100,
       "ks":{"p":{"a":0,"k":[90,90]},"a":{"a":0,"k":[50,50]},"s":{"a":0,"k":[10,10]},"o":{"a":0,"k":50}}}
    ]}"##;
    let a = Animation::open(json.as_bytes(), &NoFiles).unwrap();
    let f = a.render(0.0, 100, 100);
    assert_eq!(pixel(&f, 100, 40, 50), [0, 0, 255, 255]); // in the mask and the circle
    assert_eq!(pixel(&f, 100, 60, 50), [0, 0, 0, 0]); // in the circle, outside the mask
    assert_eq!(pixel(&f, 100, 10, 10), [0, 0, 0, 0]); // in the mask, outside the circle
    // the precomposition, a tenth the size and half there
    assert_eq!(pixel(&f, 100, 90, 90), [0, 255, 0, 128]);
    assert_eq!(pixel(&f, 100, 80, 90), [0, 0, 0, 0]);
}

#[test]
fn strokes_trim_and_repeat() {
    // a horizontal line from 10 to 90, 10 wide, the first half of it drawn, three copies 20 apart
    let json = r#"{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[
      {"ty":4,"ind":1,"ip":0,"op":30,"st":0,"ks":{},"shapes":[
        {"ty":"sh","ks":{"a":0,"k":{"c":false,"v":[[10,20],[90,20]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}}},
        {"ty":"tm","s":{"a":0,"k":0},"e":{"a":0,"k":50},"o":{"a":0,"k":0},"m":1},
        {"ty":"st","c":{"a":0,"k":[0,0,0,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":10},"lc":1,"lj":1},
        {"ty":"rp","c":{"a":0,"k":3},"o":{"a":0,"k":0},"m":1,
         "tr":{"p":{"a":0,"k":[0,20]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},
               "so":{"a":0,"k":100},"eo":{"a":0,"k":100}}}
      ]}]}"#;
    let a = Animation::open(json.as_bytes(), &NoFiles).unwrap();
    let f = a.render(0.0, 100, 100);
    for y in [20, 40, 60] {
        assert_eq!(pixel(&f, 100, 30, y), [0, 0, 0, 255], "row {y}");
        assert_eq!(pixel(&f, 100, 70, y), [0, 0, 0, 0], "row {y}");
    }
    assert_eq!(pixel(&f, 100, 30, 80), [0, 0, 0, 0]);
    assert_eq!(pixel(&f, 100, 30, 30), [0, 0, 0, 0]);
}

#[test]
fn files_that_are_not_animations_are_refused_and_what_is_not_drawn_is_said() {
    assert!(Animation::open(b"{\"name\": \"package\"}", &NoFiles).is_err());
    assert!(Animation::open(b"not json", &NoFiles).is_err());
    assert!(
        is_lottie("a.json", SLIDE.as_bytes())
            && !is_lottie("a.json", b"{\"name\": 1}")
            && !is_lottie("a.txt", SLIDE.as_bytes())
    );
    let json = r#"{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[
      {"ty":4,"ind":1,"ip":0,"op":30,"st":0,"ef":[{"ty":5}],"ks":{"r":{"a":0,"k":0,"x":"var $bm_rt = time * 10;"}},"shapes":[]},
      {"ty":2,"ind":2,"ip":0,"op":30,"st":0,"refId":"img","ks":{}}],
      "assets":[{"id":"img","w":10,"h":10,"u":"images/","p":"img_0.png"}]}"#;
    let a = Animation::open(json.as_bytes(), &NoFiles).unwrap();
    assert_eq!(a.unsupported(), ["effects", "expressions", "pictures that are not in the file"]);
}

#[test]
fn a_dotlottie_archive_gives_its_animation() {
    let manifest = br#"{"animations":[{"id":"slide"}]}"#;
    let zip = crate::zip::tests::archive(&[
        ("manifest.json", manifest.as_slice(), false),
        ("animations/slide.json", SLIDE.as_bytes(), true),
    ]);
    assert!(is_lottie("slide.lottie", &zip));
    let a = Animation::open(&zip, &NoFiles).unwrap();
    assert_eq!(pixel(&a.render(15.0, 100, 100), 100, 50, 50), [255, 0, 0, 255]);
}

#[test]
fn text_is_drawn_from_the_letter_shapes_in_the_file_with_its_outline_over_or_under() {
    // "II" in a face whose I is a bar 20 wide and 70 tall, blue with a red outline 6 wide
    let json = |over: &str| {
        format!(
            r#"{{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,
      "fonts":{{"list":[{{"fName":"Test-Regular","fFamily":"Test","fStyle":"Regular","ascent":70}}]}},
      "chars":[{{"ch":"I","size":100,"style":"Regular","w":30,"fFamily":"Test","data":{{"shapes":[{{"ty":"gr","it":[
        {{"ty":"sh","ks":{{"a":0,"k":{{"c":true,"v":[[5,-70],[25,-70],[25,0],[5,0]],
          "i":[[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0]]}}}}}}]}}]}}}}],
      "layers":[{{"ty":5,"ind":1,"ip":0,"op":30,"st":0,"ks":{{"p":{{"a":0,"k":[20,90]}}}},
        "t":{{"d":{{"k":[{{"s":{{"s":100,"f":"Test-Regular","t":"II","j":0,"tr":0,"lh":120,
          "fc":[0,0,1],"sc":[1,0,0],"sw":6{over}}},"t":0}}]}},"p":{{}},"m":{{"g":1,"a":{{"a":0,"k":[0,0]}}}},"a":[]}}}}]}}"#
        )
    };
    let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);
    for (over, inner_edge) in [("", red), (r#","of":true"#, red), (r#","of":false"#, blue)] {
        let a = Animation::open(json(over).as_bytes(), &NoFiles).unwrap();
        assert!(a.unsupported().is_empty(), "{:?}", a.unsupported());
        let f = a.render(0.0, 100, 100);
        // the first letter is from 25 to 45 across; the second an advance of 30 further on
        assert_eq!(pixel(&f, 100, 35, 50), blue);
        assert_eq!(pixel(&f, 100, 65, 50), blue);
        assert_eq!(pixel(&f, 100, 50, 50), [0, 0, 0, 0]);
        // the outline is centred on the edge: outside it always shows, inside only when it is over
        assert_eq!(pixel(&f, 100, 23, 50), red);
        assert_eq!(pixel(&f, 100, 26, 50), inner_edge, "{over}");
    }
    // a letter the file has no shape for is said, not guessed
    let a = Animation::open(json("").replace(r#""t":"II""#, r#""t":"IX""#).as_bytes(), &NoFiles).unwrap();
    assert_eq!(a.unsupported(), ["text whose letter shapes are not in the file"]);
}
