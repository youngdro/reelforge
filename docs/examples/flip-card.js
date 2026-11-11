// A card that swings open like a door, carries its words with it, and a star that flies in
// along a curve: stretch, turns in space, an origin, a group and a path, in one script.
//   reelforge-cli call script.add -p ad.reel.json source=@docs/examples/flip-card.js

export const params = {
  title: { type: "text", default: "新品上市", label: "Title" },
  price: { type: "number", default: 199, min: 0, label: "Price" },
  accent: { type: "color", default: "#ff3b5c", label: "Accent" },
};

export function build({ params, canvas, scene, timeline }) {
  const W = canvas.width;
  // everything on the card rides on this group: turn it, and the card turns as one object
  const card = scene.group({ name: "card", y: -80, perspective: 1.2 * canvas.height });
  const back = scene.rect({ parent: card, name: "back", width: 0.76 * W, height: 520, radius: 48, fill: "#ffffff",
    shadow: { blur: 60, offset: [0, 24], color: "rgba(0,0,0,0.25)" } });
  const title = scene.text(params.title, { parent: card, name: "title", y: -120, size: 110, weight: 800, color: "#111111" });
  const price = scene.text("¥{n}", { parent: card, name: "price", y: 70, size: 190, weight: 900, color: params.accent,
    counter: { from: 0, to: params.price, duration: 0.9 } });
  // a bar that grows from its left end
  const bar = scene.rect({ parent: card, name: "bar", y: 200, width: 0.5 * W, height: 14, radius: 7, fill: params.accent,
    transformOrigin: "left center" });
  const star = scene.star({ name: "star", x: -0.7 * W, y: 500, width: 150, points: 5, inner: 0.5, fill: "#ffd23f", motionBlur: true });

  const tl = timeline({ defaults: { duration: 0.6, ease: "power3.out" } });
  tl.from(card, { rotationY: -100, scale: 0.7, duration: 0.9, ease: "back.out(1.2)" })
    .from([back, title, price], { opacity: 0, duration: 0.25 }, "<")
    .from(bar, { scaleX: 0, duration: 0.5 }, "-=0.3")
    .to(star, { motionPath: { path: [{ x: -0.1 * W, y: 150 }, { x: 0.2 * W, y: -420 }, { x: 0.3 * W, y: -330 }], autoRotate: false },
      rotation: 360, duration: 1.1, ease: "power2.inOut" }, 0.3)
    .to(star, { scaleX: -1, duration: 0.3, repeat: 3, yoyo: true, ease: "sine.inOut" }, ">")
    .to(card, { rotationX: 12, duration: 0.4, yoyo: true, repeat: 1, ease: "sine.inOut" }, "<");
  scene.setDuration(4);
}
