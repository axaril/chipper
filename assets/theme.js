// Light/dark toggle with a circular reveal. Ported from axaril.github.io
// (theme.ts); uses event delegation because the button is rendered by wasm.

const root = document.documentElement;
const dark = matchMedia("(prefers-color-scheme: dark)");

try {
  const t = localStorage.getItem("theme");
  if (t === "light" || t === "dark") root.dataset.theme = t;
} catch {}

function current() {
  return root.dataset.theme || (dark.matches ? "dark" : "light");
}

function apply(next) {
  root.dataset.theme = next;
  try {
    localStorage.setItem("theme", next);
  } catch {}
}

document.addEventListener("click", (e) => {
  const toggle = e.target.closest && e.target.closest("#theme-toggle");
  if (!toggle) return;
  const next = current() === "dark" ? "light" : "dark";
  const animate =
    typeof document.startViewTransition === "function" &&
    !matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!animate) return apply(next);

  const box = toggle.getBoundingClientRect();
  const x = box.left + box.width / 2;
  const y = box.top + box.height / 2;
  const radius = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
  const transition = document.startViewTransition(() => apply(next));
  transition.ready.then(() =>
    root.animate(
      { clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${radius}px at ${x}px ${y}px)`] },
      { duration: 600, easing: "ease-in-out", pseudoElement: "::view-transition-new(root)" },
    ),
  );
});
