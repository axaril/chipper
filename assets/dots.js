// Faint dot grid background; dots near the pointer swell and light up, and an
// occasional ripple spreads out. Ported from axaril.github.io (dots.ts).

const RIPPLE = {
  amp: 12,
  tint: 1,
  width: 70,
  k: (2 * Math.PI) / 56,
  speed: 0.2,
  duration: 3500,
  max: 6,
  pause: 2000,
};

class DotGrid {
  constructor(canvas, { gap = 18, radius = 140, baseVar = "--muted", accentVar = "--accent" } = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.root = document.documentElement;
    this.gap = gap;
    this.radius = radius;
    this.reach = radius + gap;
    this.baseVar = baseVar;
    this.accentVar = accentVar;
    this.view = { width: 0, height: 0 };
    this.colors = { base: "", accent: "" };
    this.pointer = { x: -1e4, y: -1e4 };
    this.glow = { strength: 0, target: 0 };
    this.frame = 0;
    this.resizing = 0;
    this.moved = false;
    this.last = null;
    this.now = 0;
    this.ripples = [];

    this.readColors();
    this.resize();
    addEventListener("resize", this.queueResize);
    new MutationObserver(this.recolor).observe(this.root, { attributes: true, attributeFilter: ["data-theme"] });
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", this.recolor);

    if (!matchMedia("(prefers-reduced-motion: reduce)").matches) {
      this.listen();
      setTimeout(this.startRipple, RIPPLE.pause);
    }
  }

  listen() {
    addEventListener("pointermove", (e) => {
      if (e.pointerType !== "touch") this.aim(e.clientX, e.clientY);
    });
    addEventListener("pointerdown", (e) => this.ripplePoint(e.clientX, e.clientY));
    this.root.addEventListener("mouseleave", this.release);
    const touch = (e) => this.aim(e.touches[0].clientX, e.touches[0].clientY);
    addEventListener("touchstart", touch, { passive: true });
    addEventListener("touchmove", touch, { passive: true });
    addEventListener("touchend", this.release);
    addEventListener("touchcancel", this.release);
  }

  resolve(name) {
    this.canvas.style.color = `var(${name})`;
    return getComputedStyle(this.canvas).color;
  }

  readColors() {
    this.colors.base = this.resolve(this.baseVar);
    this.colors.accent = this.resolve(this.accentVar);
  }

  paint({ x0, y0, x1, y1 }) {
    const { ctx, gap, radius, pointer, glow, colors } = this;
    ctx.clearRect(x0, y0, x1 - x0, y1 - y0);
    let style = "";
    for (let y = y0 + gap / 2; y < y1; y += gap) {
      for (let x = x0 + gap / 2; x < x1; x += gap) {
        const dx = x - pointer.x;
        const dy = y - pointer.y;
        const d = Math.hypot(dx, dy);
        const t = Math.max(0, 1 - d / radius) ** 2 * glow.strength;
        const push = d ? (t * 10) / d : 0;
        let rx = 0;
        let ry = 0;
        let tint = 0;
        for (const ripple of this.ripples) {
          const ox = x - ripple.x;
          const oy = y - ripple.y;
          const od = Math.hypot(ox, oy);
          const s = od ? this.push(od, ripple) : 0;
          rx += (ox / od) * s || 0;
          ry += (oy / od) * s || 0;
          tint += Math.abs(s) / RIPPLE.amp;
        }
        const fill = t > 0.02 || tint > 0.08 ? colors.accent : colors.base;
        if (fill !== style) ctx.fillStyle = style = fill;
        ctx.globalAlpha = Math.min(1, 0.28 + t * 0.6 + tint * RIPPLE.tint);
        const half = 1.1 + t * 2 + tint * 0.8;
        ctx.fillRect(x + rx + dx * push - half, y + ry + dy * push - half, half * 2, half * 2);
      }
    }
  }

  push(d, ripple) {
    const age = this.now - ripple.start;
    const p = age / RIPPLE.duration;
    if (p >= 1) return 0;
    const u = d - age * RIPPLE.speed;
    const ring = Math.exp(-((u / RIPPLE.width) ** 2));
    return RIPPLE.amp * (1 - p) ** 2 * ring * Math.sin(u * RIPPLE.k);
  }

  spread({ x, y, start }) {
    const reach = (this.now - start) * RIPPLE.speed + RIPPLE.width * 2.5 + RIPPLE.amp;
    return this.around(x, y, reach);
  }

  startRipple = () => {
    const { width, height } = this.view;
    this.ripplePoint(Math.random() * width, Math.random() * height);
    setTimeout(this.startRipple, RIPPLE.pause + Math.random() * RIPPLE.pause);
  };

  ripplePoint(x, y) {
    this.ripples.push({ start: performance.now(), x, y });
    if (this.ripples.length > RIPPLE.max) this.ripples.shift();
    this.wake();
  }

  snap(v, max, round) {
    return Math.min(Math.max(round(v / this.gap) * this.gap, 0), max);
  }

  around(x, y, reach = this.reach) {
    const { gap, view } = this;
    const maxX = Math.ceil(view.width / gap) * gap;
    const maxY = Math.ceil(view.height / gap) * gap;
    return {
      x0: this.snap(x - reach, maxX, Math.floor),
      y0: this.snap(y - reach, maxY, Math.floor),
      x1: this.snap(x + reach, maxX, Math.ceil),
      y1: this.snap(y + reach, maxY, Math.ceil),
    };
  }

  union(a, b) {
    return { x0: Math.min(a.x0, b.x0), y0: Math.min(a.y0, b.y0), x1: Math.max(a.x1, b.x1), y1: Math.max(a.y1, b.y1) };
  }

  full() {
    return { x0: 0, y0: 0, x1: this.view.width, y1: this.view.height };
  }

  tick = () => {
    const { glow, last } = this;
    this.now = performance.now();
    glow.strength += (glow.target - glow.strength) * 0.14;
    const settled = Math.abs(glow.target - glow.strength) < 0.004;
    if (settled) glow.strength = glow.target;

    const box = this.around(this.pointer.x, this.pointer.y);
    this.last = box;
    let dirty = last ? this.union(box, last) : box;
    for (const ripple of this.ripples) dirty = this.union(dirty, this.spread(ripple));
    this.paint(dirty);
    this.ripples = this.ripples.filter((r) => this.now - r.start < RIPPLE.duration);

    this.frame = settled && !this.moved && !this.ripples.length ? 0 : requestAnimationFrame(this.tick);
    this.moved = false;
  };

  wake() {
    if (!this.frame) this.frame = requestAnimationFrame(this.tick);
  }

  aim(x, y) {
    this.pointer.x = x;
    this.pointer.y = y;
    this.glow.target = 1;
    this.moved = true;
    this.wake();
  }

  release = () => {
    this.glow.target = 0;
    this.wake();
  };

  resize() {
    const { canvas, view } = this;
    if (innerWidth === view.width && innerHeight <= view.height) return;
    view.width = innerWidth;
    view.height = innerHeight;
    canvas.width = view.width;
    canvas.height = view.height;
    canvas.style.width = `${view.width}px`;
    canvas.style.height = `${view.height}px`;
    this.last = null;
    this.paint(this.full());
  }

  queueResize = () => {
    if (!this.resizing)
      this.resizing = requestAnimationFrame(() => {
        this.resizing = 0;
        this.resize();
      });
  };

  recolor = () => {
    this.readColors();
    this.paint(this.full());
  };
}

// The canvas is rendered by the wasm app, so wait for it to appear.
(function start() {
  const canvas = document.getElementById("dots");
  if (canvas) new DotGrid(canvas);
  else requestAnimationFrame(start);
})();
