export const params = {
  title:  { type: "text",   default: "限时特惠", label: "标题" },
  was:    { type: "number", default: 299, label: "原价" },
  price:  { type: "number", default: 199, label: "现价" },
  accent: { type: "color",  default: "#ff3b5c", label: "强调色" },
  tags:   { type: "list",   default: ["包邮", "次日达", "7 天无理由"], label: "标签" },
};

export function build({ params, canvas, scene, timeline }) {
  const W = canvas.width, H = canvas.height;
  const bg    = scene.rect({ width: W, height: H, fill: { type: "linear", from: "#1b1033", to: "#3a1c5c", angle: 90 } });
  const card  = scene.rect({ y: 0, width: 0.82 * W, height: 0.3 * H, radius: 48, fill: "#ffffff",
                             shadow: { color: "rgba(0,0,0,0.35)", offset: [0, 18], blur: 30 } });
  const bar   = scene.rect({ y: -0.15 * H + 60, width: 0.82 * W, height: 120, radius: 48, fill: params.accent });
  const title = scene.text(params.title, { y: -0.15 * H + 60, size: 64, color: "#fff" });
  const price = scene.text("¥{n}", { y: -20, size: 220, color: params.accent,
                                     counter: { from: params.was, to: params.price, duration: 0.9 } });
  const was   = scene.text(`原价 ¥${params.was}`, { y: 130, size: 48, color: "#999" });
  const strike = scene.line({ y: 132, width: 300, stroke: { color: "#999", width: 5 } });
  const tags  = params.tags.map((t, i) =>
    scene.text(t, { x: (i - (params.tags.length - 1) / 2) * 0.26 * W, y: 0.15 * H - 70, size: 40, color: "#444",
                    background: "#f1eef7", radius: 0.6 }));
  const star  = scene.star({ x: 0.33 * W, y: -0.15 * H, width: 200, height: 200, points: 12, inner: 0.8, fill: "#ffd23f" });
  const off   = scene.text(`-${Math.round((1 - params.price / params.was) * 100)}%`, { x: 0.33 * W, y: -0.15 * H, size: 54, color: "#b3261e", rotation: -12 });

  const tl = timeline({ defaults: { duration: 0.5, ease: "power3.out" } });
  tl.from(bg, { opacity: 0, duration: 0.3 })
    .from([card, bar], { y: "+=600", opacity: 0, ease: "back.out(1.4)" }, "<0.1")
    .from(title, { scale: 0.6, opacity: 0 }, "-=0.25")
    .from(price, { scale: 0, duration: 0.8, ease: "elastic.out(1, 0.5)" }, "<0.1")
    .from(was, { opacity: 0, x: "-=80" }, "-=0.5")
    .fromTo(strike, { drawEnd: 0 }, { drawEnd: 1, duration: 0.35, ease: "power2.inOut" }, ">-0.1")
    .from(tags, { y: "+=60", opacity: 0, stagger: 0.08 }, "-=0.2")
    .from([star, off], { scale: 0, rotation: "-=180", ease: "back.out(2)", duration: 0.6 }, "-=0.3")
    .to([star, off], { scale: 1.12, duration: 0.4, ease: "sine.inOut", yoyo: true, repeat: 3 }, "+=0.1")
    .to([card, bar, title, price, was, strike, star, off, ...tags], { opacity: 0, y: "-=120", duration: 0.35, ease: "power2.in", stagger: 0.02 }, "+=0.6");
}
