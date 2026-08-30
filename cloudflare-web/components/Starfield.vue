<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";

const canvasRef = ref<HTMLCanvasElement | null>(null);
let animId = 0;

// 4 soft orbs — sparse, large, slow
const ORBS = [
  { bx: 0.3, by: 0.35, r: 255, g: 255, b: 255,  size: 0.40, spd: 0.3, ph: 0 },
  { bx: 0.7, by: 0.30, r: 200, g: 200, b: 200, size: 0.35, spd: 0.25, ph: 2 },
  { bx: 0.5, by: 0.65, r: 150, g: 150, b: 150, size: 0.38, spd: 0.22, ph: 4 },
  { bx: 0.2, by: 0.60, r: 100, g: 100, b: 100, size: 0.30, spd: 0.18, ph: 6 },
];

onMounted(() => {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  let w = 0, h = 0;
  function resize() {
    const dpr = window.devicePixelRatio || 1;
    w = window.innerWidth;
    h = window.innerHeight;
    canvas!.width = w * dpr;
    canvas!.height = h * dpr;
    canvas!.style.width = w + "px";
    canvas!.style.height = h + "px";
    ctx!.setTransform(dpr, 0, 0, dpr, 0, 0);
  }
  resize();
  window.addEventListener("resize", resize);

  let mx = 0.5, my = 0.5;
  window.addEventListener("mousemove", (e) => {
    mx = e.clientX / window.innerWidth;
    my = e.clientY / window.innerHeight;
  });

  let t = 0;
  function draw() {
    t += 0.008;

    // Dark base
    ctx!.fillStyle = "#000000";
    ctx!.fillRect(0, 0, w, h);
    ctx!.globalCompositeOperation = "screen";

    for (const o of ORBS) {
      // Lissajous drift
      const lx = Math.sin(t * o.spd + o.ph) * 0.10;
      const ly = Math.cos(t * o.spd * 0.8 + o.ph + 1) * 0.08;

      // Mouse pull
      const dx = mx - o.bx;
      const dy = my - o.by;

      const cx = (o.bx + lx + dx * 0.05) * w;
      const cy = (o.by + ly + dy * 0.05) * h;
      const r = o.size * Math.max(w, h);
      const a = 0.30 + 0.10 * Math.sin(t * 0.7 + o.ph);

      const grad = ctx!.createRadialGradient(cx, cy, 0, cx, cy, r);
      grad.addColorStop(0,   `rgba(${o.r},${o.g},${o.b},${a})`);
      grad.addColorStop(0.4, `rgba(${o.r},${o.g},${o.b},${a * 0.4})`);
      grad.addColorStop(1,   `rgba(${o.r},${o.g},${o.b},0)`);
      ctx!.fillStyle = grad;
      ctx!.beginPath();
      ctx!.arc(cx, cy, r, 0, Math.PI * 2);
      ctx!.fill();
    }

    ctx!.globalCompositeOperation = "source-over";
    animId = requestAnimationFrame(draw);
  }
  draw();

  onBeforeUnmount(() => {
    cancelAnimationFrame(animId);
    window.removeEventListener("resize", resize);
  });
});
</script>

<template>
  <canvas ref="canvasRef" class="aurora-canvas" aria-hidden="true" />
</template>

<style scoped>
.aurora-canvas {
  position: fixed;
  inset: 0;
  width: 100vw;
  height: 100vh;
  z-index: 0;
  pointer-events: none;
}
</style>
