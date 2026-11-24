import { useEffect, useRef } from "react";

/**
 * Slow, blurred clouds of light drifting across the sky. Drawn small and
 * blurred by CSS, so it costs little. The pointer leans the clouds (deeper
 * clouds move more) and carries a soft light with it. Pauses when off screen or in a hidden
 * tab, and draws one still frame when the visitor prefers reduced motion.
 */
interface Cloud {
  x: number;
  y: number;
  r: number;
  speed: number;
  sway: number;
  phase: number;
  hue: number;
  alpha: number;
}

/** Sky colours come from CSS (--sky-top, --sky-mid, --sky-low, --cloud-1..3), so Night and Day differ. */
interface Palette {
  top: string;
  mid: string;
  low: string;
  clouds: string[];
}

function readPalette(): Palette {
  const css = getComputedStyle(document.documentElement);
  const get = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    top: get("--sky-top", "#070d1a"),
    mid: get("--sky-mid", "#0d1830"),
    low: get("--sky-low", "#1a2c55"),
    clouds: [
      get("--cloud-1", "158, 182, 255"),
      get("--cloud-2", "120, 150, 240"),
      get("--cloud-3", "196, 212, 255"),
    ],
  };
}

function makeClouds(count: number): Cloud[] {
  return Array.from({ length: count }, (_, index) => ({
    x: Math.random(),
    y: 0.15 + Math.random() * 0.8,
    r: 0.12 + Math.random() * 0.22,
    speed: 0.006 + Math.random() * 0.012,
    sway: 0.01 + Math.random() * 0.03,
    phase: Math.random() * Math.PI * 2,
    hue: index % 3,
    alpha: 0.16 + Math.random() * 0.22,
  }));
}

export function SkyCanvas() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    const clouds = makeClouds(11);
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let frame = 0;
    let running = false;
    let last = performance.now();
    let time = 0;
    let palette = readPalette();
    let frames = 0;
    // Pointer position in 0..1, eased toward the target each frame.
    const pointer = { x: 0.5, y: 0.5, tx: 0.5, ty: 0.5, glow: 0, tglow: 0 };

    const resize = () => {
      // A quarter-size canvas; CSS scales it up and blurs it.
      const rect = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(rect.width / 4));
      canvas.height = Math.max(1, Math.round(rect.height / 4));
    };

    const draw = () => {
      const { width: w, height: h } = canvas;
      const sky = ctx.createLinearGradient(0, 0, 0, h);
      sky.addColorStop(0, palette.top);
      sky.addColorStop(0.55, palette.mid);
      sky.addColorStop(1, palette.low);
      ctx.fillStyle = sky;
      ctx.fillRect(0, 0, w, h);

      const unit = Math.max(w, h);
      const leanX = (pointer.x - 0.5) * 0.12;
      const leanY = (pointer.y - 0.5) * 0.08;
      for (const cloud of clouds) {
        const span = 1 + cloud.r * 2;
        const depth = cloud.r / 0.34;
        const x = ((((cloud.x + cloud.speed * time) % span) + span) % span) - cloud.r - leanX * depth;
        const y = cloud.y + Math.sin(time * cloud.sway * 6 + cloud.phase) * 0.03 - leanY * depth;
        const cx = x * w;
        const cy = y * h;
        const radius = cloud.r * unit;
        const glow = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius);
        const rgb = palette.clouds[cloud.hue];
        glow.addColorStop(0, `rgba(${rgb}, ${cloud.alpha})`);
        glow.addColorStop(1, `rgba(${rgb}, 0)`);
        ctx.fillStyle = glow;
        ctx.beginPath();
        ctx.ellipse(cx, cy, radius * 1.6, radius * 0.7, 0, 0, Math.PI * 2);
        ctx.fill();
      }

      if (pointer.glow > 0.01) {
        const gx = pointer.x * w;
        const gy = pointer.y * h;
        const gr = unit * 0.22;
        const light = ctx.createRadialGradient(gx, gy, 0, gx, gy, gr);
        light.addColorStop(0, `rgba(${palette.clouds[2]}, ${0.32 * pointer.glow})`);
        light.addColorStop(1, `rgba(${palette.clouds[2]}, 0)`);
        ctx.fillStyle = light;
        ctx.fillRect(gx - gr, gy - gr, gr * 2, gr * 2);
      }
    };

    const tick = (now: number) => {
      time += Math.min(now - last, 100) / 1000;
      last = now;
      // Pick up a theme change within half a second.
      if (++frames % 30 === 0) palette = readPalette();
      pointer.x += (pointer.tx - pointer.x) * 0.06;
      pointer.y += (pointer.ty - pointer.y) * 0.06;
      pointer.glow += (pointer.tglow - pointer.glow) * 0.08;
      draw();
      frame = requestAnimationFrame(tick);
    };

    const start = () => {
      if (running || reduce || document.hidden) return;
      running = true;
      last = performance.now();
      frame = requestAnimationFrame(tick);
    };

    const stop = () => {
      running = false;
      cancelAnimationFrame(frame);
    };

    resize();
    draw();

    const observer = new IntersectionObserver(([entry]) => (entry.isIntersecting ? start() : stop()));
    observer.observe(canvas);
    const onVisibility = () => (document.hidden ? stop() : start());
    const onResize = () => {
      resize();
      palette = readPalette();
      draw();
    };
    // Redraw the still frame when the theme changes (reduced motion, or paused).
    const themeWatch = new MutationObserver(() => {
      palette = readPalette();
      draw();
    });
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    document.addEventListener("visibilitychange", onVisibility);

    // In the agent the sky sits behind everything, so the whole window drives it.
    const area = window;
    const onMove = (event: PointerEvent) => {
      if (event.pointerType === "touch") return;
      const rect = canvas.getBoundingClientRect();
      pointer.tx = (event.clientX - rect.left) / rect.width;
      pointer.ty = (event.clientY - rect.top) / rect.height;
      pointer.tglow = 1;
    };
    const onLeave = () => {
      pointer.tx = 0.5;
      pointer.ty = 0.5;
      pointer.tglow = 0;
    };
    if (!reduce) {
      area.addEventListener("pointermove", onMove);
      area.addEventListener("pointerleave", onLeave);
    }
    window.addEventListener("resize", onResize);

    return () => {
      stop();
      observer.disconnect();
      themeWatch.disconnect();
      area.removeEventListener("pointermove", onMove);
      area.removeEventListener("pointerleave", onLeave);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return <canvas ref={ref} className="sky" aria-hidden="true" />;
}
