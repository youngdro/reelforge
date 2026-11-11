use super::*;

fn go(src: &str) -> Result<Built, ScriptError> {
    go_with(src, json!({}))
}

fn go_with(src: &str, params: Value) -> Result<Built, ScriptError> {
    run(src, &Input { params: &params, canvas: [1000, 2000], fps: 30.0, seed: 7, duration: None })
}

fn err(src: &str) -> String {
    go(src).unwrap_err().to_string()
}

/// A script whose `build` is `body`.
fn body(body: &str) -> String {
    format!("export function build({{ params, canvas, scene, timeline, random }}) {{\n{body}\n}}")
}

fn keys<'a>(b: &'a Built, node: usize, prop: &str) -> &'a [KeyOut] {
    b.nodes[node].keys.get(prop).map_or(&[], Vec::as_slice)
}

fn tv(list: &[KeyOut]) -> Vec<(f64, f64)> {
    list.iter().map(|k| ((k.t * 1000.0).round() / 1000.0, (k.v * 10000.0).round() / 10000.0)).collect()
}

#[test]
fn a_tween_is_two_keys_and_its_ease_a_bezier() {
    let b = go(&body(
        r#"const a = scene.rect({ width: 200, height: 100, x: 100 });
           timeline().to(a, { x: 600, opacity: 0.5, duration: 1, ease: "power2.out" });"#,
    ))
    .unwrap();
    assert_eq!(b.duration, 1.0);
    assert_eq!(b.nodes[0].kind, "shape");
    assert_eq!(b.nodes[0].add["shape"]["size"], json!([200, 100]));
    // x is a share of the canvas, as clips keep it
    assert_eq!(b.nodes[0].add["x"], json!(0.1));
    assert_eq!(tv(keys(&b, 0, "x")), [(0.0, 0.1), (1.0, 0.6)]);
    assert_eq!(tv(keys(&b, 0, "opacity")), [(0.0, 1.0), (1.0, 0.5)]);
    // a cubic is said exactly: thirds across, the slopes up
    let e: [f64; 4] = serde_json::from_value(keys(&b, 0, "x")[0].ease.clone()).unwrap();
    assert!((e[0] - 1.0 / 3.0).abs() < 1e-9 && e[1] == 1.0 && (e[2] - 2.0 / 3.0).abs() < 1e-9 && e[3] == 1.0);
    assert_eq!(keys(&b, 0, "x")[1].ease, json!("linear"));
}

#[test]
fn from_ends_where_the_node_is_and_relative_values_add() {
    let b = go(&body(
        r#"const a = scene.rect({ y: 100, opacity: 0.8 });
           timeline().from(a, { y: "+=400", opacity: 0, duration: 0.5 })
                     .to(a, { y: "-=100", duration: 0.5 });"#,
    ))
    .unwrap();
    assert_eq!(tv(keys(&b, 0, "y")), [(0.0, 0.25), (0.5, 0.05), (1.0, 0.0)]);
    assert_eq!(tv(keys(&b, 0, "opacity")), [(0.0, 0.0), (0.5, 0.8)]);
}

#[test]
fn positions_place_tweens_as_gsap_does() {
    let b = go(&body(
        r#"const [a, b, c, d, e, f] = [1, 2, 3, 4, 5, 6].map(() => scene.rect());
           const tl = timeline({ defaults: { duration: 1, ease: "none" } });
           tl.to(a, { x: 10 })              // 0 → 1
             .to(b, { x: 10 }, "<")         // with a: 0 → 1
             .to(c, { x: 10 }, "<0.5")      // 0.5 → 1.5
             .to(d, { x: 10 }, "-=0.5")     // 1 → 2
             .addLabel("mid", 3)
             .to(e, { x: 10 }, "mid+=0.5")  // 3.5 → 4.5
             .to(f, { x: 10 }, ">")         // after e: 4.5 → 5.5
             .to(a, { x: 20 }, 6);          // absolute"#,
    ))
    .unwrap();
    let span = |n: usize| {
        let k = keys(&b, n, "x");
        (k[0].t, k[1].t)
    };
    assert_eq!(span(1), (0.0, 1.0));
    assert_eq!(span(2), (0.5, 1.5));
    assert_eq!(span(3), (1.0, 2.0));
    assert_eq!(span(4), (3.5, 4.5));
    assert_eq!(span(5), (4.5, 5.5));
    assert_eq!(tv(keys(&b, 0, "x")), [(0.0, 0.0), (1.0, 0.01), (6.0, 0.01), (7.0, 0.02)]);
    assert_eq!(b.duration, 7.0);
}

#[test]
fn a_stagger_starts_each_node_later_and_from_the_end_backwards() {
    let b = go(&body(
        r#"const n = [0, 1, 2].map(() => scene.rect());
           timeline().from(n, { opacity: 0, duration: 0.5, stagger: 0.1 })
                     .to(n, { scale: 2, duration: 0.5, stagger: { each: 0.2, from: "end" } }, 1);"#,
    ))
    .unwrap();
    let starts = |prop: &str| (0..3).map(|i| keys(&b, i, prop)[0].t).collect::<Vec<_>>();
    assert_eq!(starts("opacity"), [0.0, 0.1, 0.2]);
    assert_eq!(starts("scale"), [1.4, 1.2, 1.0]);
}

#[test]
fn a_yoyo_goes_there_and_back_and_a_repeat_starts_over() {
    let b = go(&body(
        r#"const a = scene.rect(), c = scene.rect();
           timeline().to(a, { scale: 2, duration: 0.5, ease: "none", yoyo: true, repeat: 3 })
                     .to(c, { rotation: 90, duration: 0.5, ease: "none", repeat: 1 }, 0);"#,
    ))
    .unwrap();
    assert_eq!(tv(keys(&b, 0, "scale")), [(0.0, 1.0), (0.5, 2.0), (1.0, 1.0), (1.5, 2.0), (2.0, 1.0)]);
    // starting over is a jump: the turn holds from the frame before, then is back at nothing
    let r = keys(&b, 1, "rotation");
    let frame = (0.5 * 30.0 - 1.0) / 30.0;
    assert_eq!(tv(&r[..3]), [(0.0, 0.0), ((frame * 1000.0_f64).round() / 1000.0, 90.0), (0.5, 0.0)]);
    assert_eq!(r[1].ease, json!("hold"));
    assert_eq!(r.last().map(|k| (k.t, k.v)), Some((1.0, 90.0)));
}

#[test]
fn an_ease_no_bezier_says_is_a_key_a_frame() {
    let b = go(&body(
        r#"const a = scene.rect(), c = scene.rect(), d = scene.rect();
           timeline().from(a, { scale: 0, duration: 1, ease: "elastic.out(1, 0.3)" })
                     .to(c, { x: 300, duration: 1, ease: "steps(3)" }, 0)
                     .to(d, { y: 200, duration: 1, ease: (t) => t * t }, 0);"#,
    ))
    .unwrap();
    let s = keys(&b, 0, "scale");
    assert_eq!(s.len(), 31);
    assert!(s.iter().any(|k| k.v > 1.05), "an elastic ease passes its end");
    assert_eq!((s[0].v, s[30].v), (0.0, 1.0));
    let x = keys(&b, 1, "x");
    assert!(x.iter().all(|k| k.ease == json!("hold")));
    assert_eq!((x[9].v, x[10].v, x[30].v), (0.0, 0.1, 0.3));
    let y = keys(&b, 2, "y");
    assert!((y[15].v - 0.1 * 0.25).abs() < 1e-9);
}

#[test]
fn set_holds_the_node_as_it_was_until_then() {
    let b = go(&body(
        r#"const a = scene.rect();
           timeline().set(a, { opacity: 0 }, 1).to(a, { opacity: 1, duration: 0.5 }, 2);"#,
    ))
    .unwrap();
    let o = keys(&b, 0, "opacity");
    assert_eq!(tv(o), [(0.0, 1.0), (1.0, 0.0), (2.0, 0.0), (2.5, 1.0)]);
    assert_eq!(o[0].ease, json!("hold"));
}

#[test]
fn parameters_have_defaults_and_are_checked() {
    let src = r##"
        export const params = {
          price: { type: "number", default: 199, min: 0, label: "Price" },
          tint: { type: "color", default: "#ff0000" },
          tags: { type: "list", default: ["a", "b"] },
        };
        export function build({ params, scene }) {
          scene.text(`¥${params.price}`, { color: params.tint });
          params.tags.forEach((t) => scene.text(t));
        }"##;
    let b = go(src).unwrap();
    assert_eq!(b.nodes.len(), 3);
    assert_eq!(b.nodes[0].add["text"], json!("¥199"));
    assert_eq!(b.nodes[0].add["color"], json!([1, 0, 0, 1]));
    assert_eq!(b.params["price"]["label"], json!("Price"));

    let b = go_with(src, json!({"price": 88, "tags": ["x"]})).unwrap();
    assert_eq!(b.nodes.len(), 2);
    assert_eq!(b.nodes[0].add["text"], json!("¥88"));
    assert_eq!(b.values["price"], json!(88));

    let e = go_with(src, json!({"cost": 1})).unwrap_err().to_string();
    assert_eq!(e, "the script has no parameter `cost`; it has price, tint, tags");
    let e = go_with(src, json!({"price": -1})).unwrap_err().to_string();
    assert!(e.contains("`price` must be from 0"), "{e}");
    let e = go_with(src, json!({"tint": "reddish"})).unwrap_err().to_string();
    assert!(e.contains("is not a colour"), "{e}");
}

#[test]
fn colours_are_read_as_css_writes_them() {
    let b = go(&body(
        r##"scene.rect({ fill: "#fff" });
            scene.rect({ fill: "rgba(255, 0, 0, 0.5)" });
            scene.rect({ fill: { type: "linear", from: "black", to: "#00ff0080", angle: 90 } });
            scene.line({ width: 300, stroke: { color: "#0000ff", width: 4 } });
            scene.text("a", { color: [0, 0, 0, 1], background: "white", outline: { color: "red", width: 0.1 } });"##,
    ))
    .unwrap();
    assert_eq!(b.nodes[0].add["shape"]["fill"], json!([1, 1, 1, 1]));
    assert_eq!(b.nodes[1].add["shape"]["fill"], json!([1, 0, 0, 0.5]));
    assert_eq!(b.nodes[2].add["shape"]["fill"]["to"][1], json!(1));
    assert_eq!(b.nodes[3].add["shape"]["stroke"]["color"], json!([0, 0, 1, 1]));
    assert_eq!(b.nodes[3].add["shape"]["size"], json!([300, 0]));
    assert_eq!(b.nodes[4].add["outline"]["color"], json!([1, 0, 0, 1]));
    // a title is plain words unless it says otherwise
    assert_eq!(b.nodes[4].add["shadow"], Value::Null);
}

#[test]
fn a_counter_is_the_title_s_and_can_be_tweened() {
    let b = go(&body(
        r#"const p = scene.text("¥{n}", { counter: { from: 299, to: 199, duration: 0.9, ease: "power2.out" } });
           timeline().to(p, { count: 99, duration: 1, ease: "none" }, 2);"#,
    ))
    .unwrap();
    let c = b.nodes[0].counter.as_ref().unwrap();
    assert_eq!(c["from"], json!(299));
    assert_eq!(c["over"], json!({"seconds": 0.9}));
    assert!(c["ease"].is_array());
    assert_eq!(tv(keys(&b, 0, "count")), [(2.0, 299.0), (3.0, 99.0)]);
}

#[test]
fn the_same_script_builds_the_same_thing() {
    let src = body(
        r#"for (let i = 0; i < 5; i++) {
             const d = scene.circle({ radius: 10 + random() * 40, x: (Math.random() - 0.5) * 800 });
             timeline().to(d, { y: 500 * random(), duration: 1 });
           }"#,
    );
    let (a, b) = (go(&src).unwrap(), go(&src).unwrap());
    assert_eq!(a, b);
    let other = run(&src, &Input { params: &json!({}), canvas: [1000, 2000], fps: 30.0, seed: 8, duration: None });
    assert_ne!(a.nodes[0].add["x"], other.unwrap().nodes[0].add["x"]);
}

#[test]
fn how_long_it_is_is_the_caller_s_then_the_script_s_then_its_tweens() {
    let src = body(r#"const a = scene.rect(); timeline().to(a, { x: 5, duration: 2 }); scene.setDuration(3);"#);
    assert_eq!(go(&src).unwrap().duration, 3.0);
    let asked = Input { params: &json!({}), canvas: [1000, 2000], fps: 30.0, seed: 1, duration: Some(1.0) };
    let b = run(&src, &asked).unwrap();
    assert_eq!(b.duration, 1.0);
    // keys past the end are left out
    assert_eq!(keys(&b, 0, "x").len(), 1);
    assert_eq!(go(&body("scene.rect();")).unwrap().duration, 1.0 / 30.0);
}

#[test]
fn what_is_wrong_with_a_script_is_said() {
    assert!(err("export function build({ scene }) { scene.rect({); }").starts_with("the script does not parse: "));
    assert_eq!(
        err("const a = 1;"),
        "the script must have `export function build({ params, canvas, scene, timeline })`"
    );
    assert_eq!(err(&body("")), "the script made nothing: call scene.rect, scene.text, … in build");
    assert_eq!(
        err(&body("scene.rect({ widht: 3 });")),
        "scene.rect: no option `widht`; it takes x, y, scale, rotation, opacity, scaleX, scaleY, rotationX, rotationY, skewX, skewY, perspective, motionBlur, transformOrigin, origin, parent, mask, animate, name, width, height, radius, points, inner, fill, stroke, shadow, trim"
    );
    let e = err(&body("const a = scene.rect({ name: 'card' }); timeline().to(a, { wobble: 3 });"));
    assert!(e.starts_with("to: card has no `wobble` to tween; it has x, y, scale, rotation, opacity"), "{e}");
    let e = err(&body("const a = scene.rect(); timeline().to(a, { x: 3, ease: 'wobble.out' });"));
    assert!(e.starts_with("ease \"wobble.out\" is not known; use one of: none, linear, power1–4"), "{e}");
    let e = err(&body("const a = scene.rect(); timeline().to(a, { x: 3 }, 'later');"));
    assert!(e.contains("there is no label \"later\""), "{e}");
    let e = err(&body("const a = scene.rect(); timeline().to(a, { x: 3, repeat: -1 });"));
    assert!(e.contains("a tween that never ends has no length"), "{e}");
    let e = err(&body("timeline().to({}, { x: 3 });"));
    assert!(e.contains("a node, or a list of nodes, that scene made"), "{e}");
    let e = err(&body("scene.rect(); null.x;"));
    assert!(e.starts_with("TypeError"), "{e}");
    assert_eq!(err(&body("throw new Error('no stock');")), "no stock");
}

#[test]
fn a_script_is_given_nothing_of_the_machine_and_cannot_run_away() {
    assert_eq!(
        err("import fs from 'fs';\nexport function build({ scene }) { scene.rect(); }"),
        "a script cannot import anything: it is one file, given `scene` and `timeline`"
    );
    assert_eq!(
        err("import x from './Cargo.toml';\nexport function build({ scene }) { scene.rect(); }"),
        "a script cannot import anything: it is one file, given `scene` and `timeline`"
    );
    assert!(err(&body("scene.text(String(Date.now()));")).starts_with("a script has no clock"));
    assert!(err(&body("scene.text(String(new Date()));")).starts_with("a script has no clock"));
    for name in ["fetch", "require", "process", "XMLHttpRequest", "setTimeout", "Deno", "WebSocket"] {
        let b = go(&body(&format!("scene.text(typeof {name});"))).unwrap();
        assert_eq!(b.nodes[0].add["text"], json!("undefined"), "{name}");
    }
    assert_eq!(
        err(&body("while (true) {}")),
        format!("a loop went round more than {LOOP_LIMIT} times; the script is stopped")
    );
    assert_eq!(
        err(&body("const f = () => f(); f();")),
        format!("calls nested deeper than {RECURSION_LIMIT}; the script is stopped")
    );
    assert!(err(&body("for (let i = 0; i < 500; i++) scene.rect();")).starts_with("a script makes at most 400 nodes"));
    let long = "x".repeat(SOURCE_LIMIT + 1);
    assert!(err(&long).contains("longer than"));
}

#[test]
fn console_log_comes_back() {
    let b = go(&body("scene.rect(); console.log('made', { n: 1 });")).unwrap();
    assert_eq!(b.log, ["made {\"n\":1}"]);
}

#[test]
fn a_node_is_stretched_turned_in_space_and_carried_with_gsap_s_names() {
    let b = go(&body(
        r#"const card = scene.group({ name: "card", perspective: 4000, rotate: 10 });
           const bar = scene.rect({ parent: card, width: 400, height: 20, transformOrigin: "left center",
             scaleX: 0.5, motionBlur: true });
           const top = scene.text("hi", { parent: bar, origin: [0.5, 1] });
           timeline().to(card, { rotateY: 180, duration: 1, ease: "none" })
             .to(bar, { scaleX: 1, scaleY: -1, autoAlpha: 0.5, duration: 1 }, 0);"#,
    ))
    .unwrap();
    let [card, bar, top] = &b.nodes[..] else { panic!("three nodes") };
    assert_eq!(
        (card.kind.as_str(), card.add["opacity"].clone(), card.add["rotation"].clone()),
        ("group", json!(0), json!(10))
    );
    // the eye's distance in canvas heights
    assert_eq!(card.add["perspective"].as_f64(), Some(2.0));
    assert_eq!(tv(keys(&b, 0, "rotationY")), [(0.0, 0.0), (1.0, 180.0)]);
    assert_eq!((bar.parent, bar.origin, bar.add["scaleX"].clone()), (Some(0), Some([0.0, 0.5]), json!(0.5)));
    assert_eq!(bar.add["motionBlur"], json!(true));
    assert!(bar.add.get("scaleY").is_none());
    assert_eq!(tv(keys(&b, 1, "scaleX")), [(0.0, 0.5), (1.0, 1.0)]);
    assert_eq!(tv(keys(&b, 1, "scaleY")), [(0.0, 1.0), (1.0, -1.0)]);
    assert_eq!(tv(keys(&b, 1, "opacity")), [(0.0, 1.0), (1.0, 0.5)]);
    assert_eq!((top.parent, top.origin), (Some(1), Some([0.5, 1.0])));

    for (src, said) in [
        (r#"scene.rect({ transformOrigin: "left left" });"#, "`transformOrigin` is"),
        (r#"scene.rect({ rotate: 1, rotation: 2 });"#, "are the same property"),
        (r#"scene.rect({ parent: 3 });"#, "`parent` is a node"),
        (r#"scene.group({ opacity: 0.5 });"#, "a group has no opacity"),
        (r#"const g = scene.group(); timeline().to(g, { opacity: 1 });"#, "has no `opacity` to tween"),
        (r#"const a = scene.rect(); timeline().to(a, { skew: 10 });"#, "write `skewX` and `skewY`"),
        (r#"scene.rect({ skewX: 90 });"#, "within ±89"),
    ] {
        let e = err(&body(src));
        assert!(e.contains(said), "{src}: {e}");
    }
}

#[test]
fn a_node_travels_a_path_from_where_it_is() {
    let b = go(&body(
        r#"const a = scene.circle({ radius: 20, x: -500, y: 1000 });
           const c = scene.circle({ radius: 20 });
           timeline()
             .to(a, { motionPath: { path: [{ x: 0, y: 0 }, [500, -1000]], autoRotate: true }, rotation: 90,
               duration: 2, ease: "power2.out" }, 0.5)
             .to(c, { motionPath: { path: [[0, 500], [250, 0]], fromCurrent: false, closed: true }, duration: 1, ease: "none" });"#,
    ))
    .unwrap();
    let path = b.nodes[0].path.as_ref().unwrap();
    assert_eq!(path.points, [[-0.5, 0.5], [0.0, 0.0], [0.5, -0.5]]);
    assert_eq!((path.start, path.duration, path.orient, path.closed), (0.5, 2.0, true, false));
    assert_eq!(path.ease, json!([1.0 / 3.0, 1, 2.0 / 3.0, 1]));
    // what is not where it is goes on being keyed
    assert_eq!(tv(keys(&b, 0, "rotation")), [(0.5, 0.0), (2.5, 90.0)]);
    assert!(keys(&b, 0, "x").is_empty());
    let path = b.nodes[1].path.as_ref().unwrap();
    assert_eq!(
        (path.points.clone(), path.start, path.closed, path.ease.clone()),
        (vec![[0.0, 0.25], [0.25, 0.0]], 2.5, true, json!("linear"))
    );
    assert_eq!(b.duration, 3.5);

    let one = r#"const a = scene.rect(); const tl = timeline();"#;
    for (src, said) in [
        (
            "tl.to(a, { motionPath: [[0, 0]], duration: 1 }).to(a, { motionPath: [[1, 1]], duration: 1 });",
            "already travels a path",
        ),
        ("tl.to(a, { motionPath: [[9, 9]], x: 5 });", "leave `x` and `y` out"),
        ("tl.to(a, { motionPath: [[9, 9]] }).to(a, { y: 5 });", "do not tween its `y` as well"),
        ("tl.to(a, { motionPath: [[9, 9]], ease: 'bounce.out' });", "a path's ease must be one a curve can say"),
        ("tl.to(a, { motionPath: [[9, 9]], repeat: 1 });", "a path is travelled once"),
        ("tl.from(a, { motionPath: [[9, 9]] });", "travelled with `to`"),
        ("tl.to(a, { motionPath: { path: [[9, 9]], curviness: 2 } });", "motionPath has no `curviness`"),
        ("tl.to(a, { motionPath: 'M0,0' });", "`motionPath` is a list of points"),
    ] {
        let e = err(&body(&format!("{one}\n{src}")));
        assert!(e.contains(said), "{src}: {e}");
    }
}

#[test]
fn a_node_is_masked_and_takes_the_editor_s_own_animations() {
    let b = go(&body(
        r#"const band = scene.rect({ mask: { shape: "rectangle", x: 0, width: 0.01, height: 3 } });
           const head = scene.text("hello there", { mask: "circle", animate: { in: { kind: "charRise", duration: 0.8 }, loop: "wave" } });
           timeline().to(band, { maskWidth: 6, duration: 1, ease: "none" });"#,
    ))
    .unwrap();
    let steps = |i: usize| -> Vec<Value> { b.nodes[i].after.iter().map(|s| json!([s.command, s.params])).collect() };
    assert_eq!(steps(0), [json!(["clip.setMask", {"shape": "rectangle", "x": 0, "width": 0.01, "height": 3}])]);
    assert_eq!(tv(keys(&b, 0, "maskWidth")), [(0.0, 0.01), (1.0, 6.0)]);
    assert_eq!(
        steps(1),
        [
            json!(["clip.setMask", {"shape": "circle"}]),
            json!(["clip.setAnimation", {"slot": "in", "kind": "charRise", "duration": {"seconds": 0.8}}]),
            json!(["clip.setAnimation", {"slot": "loop", "kind": "wave"}]),
        ]
    );
    for (src, said) in [
        ("scene.rect({ mask: { x: 1 } });", "`mask` is a shape's name, or"),
        ("scene.rect({ mask: { shape: 'circle', blur: 1 } });", "mask has no `blur`"),
        ("scene.rect({ animate: { middle: 'pop' } });", "animate has no `middle`"),
        ("const a = scene.rect(); timeline().to(a, { maskWidth: 2 });", "has no `maskWidth` to tween"),
    ] {
        let e = err(&body(src));
        assert!(e.contains(said), "{src}: {e}");
    }
}

#[test]
fn a_colour_is_tweened_as_its_four_parts() {
    let b = go(&body(
        r##"const card = scene.rect({ fill: "#0000ff" });
           const grad = scene.rect({ fill: { type: "linear", from: "#ff0000", to: "#ffff00" } });
           const word = scene.text("hi");
           timeline({ defaults: { duration: 1, ease: "none" } })
             .to(card, { fill: "rgba(255, 0, 0, 0.5)" })
             .to(grad, { fill: "#00ff00" }, 0)
             .fromTo(word, { color: "black" }, { color: [1, 1, 1, 0] }, 0);"##,
    ))
    .unwrap();
    let ends = |i: usize| {
        ["colorR", "colorG", "colorB", "colorA"].map(|p| tv(keys(&b, i, p)).iter().map(|k| k.1).collect::<Vec<_>>())
    };
    assert_eq!(ends(0), [vec![0.0, 1.0], vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 0.5]]);
    // of a gradient, the colour it starts from
    assert_eq!(ends(1), [vec![1.0, 0.0], vec![0.0, 1.0], vec![0.0, 0.0], vec![1.0, 1.0]]);
    assert_eq!(ends(2), [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0], vec![1.0, 0.0]]);
    for (src, said) in [
        ("const a = scene.rect(); timeline().to(a, { fill: 'red' });", "give a shape its `fill` when making it"),
        ("const a = scene.line(); timeline().to(a, { fill: 'red' });", "this node has no fill to tween"),
        ("const a = scene.rect({ fill: 'red' }); timeline().to(a, { color: 'red' });", "only a title has a `color`"),
        ("const a = scene.rect({ fill: 'red' }); timeline().to(a, { fill: 'reddish' });", "is not a colour"),
        (
            "const a = scene.rect({ fill: 'red' }); timeline().to(a, { backgroundColor: 'red' });",
            "tween `fill` on a shape",
        ),
    ] {
        let e = err(&body(src));
        assert!(e.contains(said), "{src}: {e}");
    }
}
