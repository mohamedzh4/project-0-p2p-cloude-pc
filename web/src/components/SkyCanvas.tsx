import { useEffect, useRef } from "react";

/**
 * Slow, blurred clouds of light drifting across a night sky. Drawn small and
 * blurred by CSS, so it costs little. Pauses when off screen or in a hidden
 * tab, and draws one still frame when the visitor prefers reduced motion.
 */
interface Cloud {
  x: number;
  y: number;
  r: number;
  speed: number;
  sway: number;
  phase: number;
  hue: string;
  alpha: number;
}

const COLORS = ["158, 182, 255", "120, 150, 240", "196, 212, 255", "92, 122, 220"];

function makeClouds(count: number): Cloud[] {
  return Array.from({ length: count }, (_, index) => ({
    x: Math.random(),
    y: 0.15 + Math.random() * 0.8,
    r: 0.12 + Math.random() * 0.22,
    speed: 0.006 + Math.random() * 0.012,
    sway: 0.01 + Math.random() * 0.03,
    phase: Math.random() * Math.PI * 2,
    hue: COLORS[index % COLORS.length],
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

    const resize = () => {
      // A quarter-size canvas; CSS scales it up and blurs it.
      const rect = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(rect.width / 4));
      canvas.height = Math.max(1, Math.round(rect.height / 4));
    };

    const draw = () => {
      const { width: w, height: h } = canvas;
      const sky = ctx.createLinearGradient(0, 0, 0, h);
      sky.addColorStop(0, "#070d1a");
      sky.addColorStop(0.55, "#0d1830");
      sky.addColorStop(1, "#1a2c55");
      ctx.fillStyle = sky;
      ctx.fillRect(0, 0, w, h);

      const unit = Math.max(w, h);
      for (const cloud of clouds) {
        const span = 1 + cloud.r * 2;
        const x = (((cloud.x + cloud.speed * time) % span) + span) % span - cloud.r;
        const y = cloud.y + Math.sin(time * cloud.sway * 6 + cloud.phase) * 0.03;
        const cx = x * w;
        const cy = y * h;
        const radius = cloud.r * unit;
        const glow = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius);
        glow.addColorStop(0, `rgba(${cloud.hue}, ${cloud.alpha})`);
        glow.addColorStop(1, `rgba(${cloud.hue}, 0)`);
        ctx.fillStyle = glow;
        ctx.beginPath();
        ctx.ellipse(cx, cy, radius * 1.6, radius * 0.7, 0, 0, Math.PI * 2);
        ctx.fill();
      }
    };

    const tick = (now: number) => {
      time += Math.min(now - last, 100) / 1000;
      last = now;
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
      draw();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("resize", onResize);

    return () => {
      stop();
      observer.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return <canvas ref={ref} className="sky" aria-hidden="true" />;
}
