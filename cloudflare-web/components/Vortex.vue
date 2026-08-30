<script setup lang="ts">
import { cn } from "~/lib/utils";
import { createNoise3D } from "simplex-noise";
import { onMounted, onBeforeUnmount, shallowRef, useTemplateRef } from "vue";

interface Props {
  class?: string;
  containerClass?: string;
  particleCount?: number;
  rangeY?: number;
  baseHue?: number;
  baseSpeed?: number;
  rangeSpeed?: number;
  baseRadius?: number;
  rangeRadius?: number;
  backgroundColor?: string;
}

const props = withDefaults(defineProps<Props>(), {
  particleCount: 500,
  rangeY: 100,
  baseSpeed: 0.0,
  rangeSpeed: 1.5,
  baseRadius: 0.8,
  rangeRadius: 2,
  baseHue: 15,
  backgroundColor: "transparent",
});

const TAU = 2 * Math.PI;
const BASE_TTL = 50;
const RANGE_TTL = 150;
const PARTICLE_PROP_COUNT = 9;
const RANGE_HUE = 60;
const NOISE_STEPS = 3;
const X_OFF = 0.00125;
const Y_OFF = 0.00125;
const Z_OFF = 0.0005;

let tick = 0;
let animationId: number | null = null;
const particleProps = shallowRef<Float32Array | null>(null);
const center = shallowRef<[number, number]>([0, 0]);
const ctx = shallowRef<CanvasRenderingContext2D | null>(null);
const canvasRef = useTemplateRef<HTMLCanvasElement>("canvasRef");

const noise3D = createNoise3D();

const particleCache = { x: 0, y: 0, vx: 0, vy: 0, life: 0, ttl: 0, speed: 0, radius: 0, hue: 0 };

function rand(n: number) { return n * Math.random(); }
function randRange(n: number): number { return n - rand(2 * n); }
function fadeInOut(t: number, m: number): number { const hm = 0.5 * m; return Math.abs(((t + hm) % m) - hm) / hm; }

function initParticle(i: number) {
  if (!particleProps.value || !canvasRef.value) return;
  const canvas = canvasRef.value;
  particleCache.x = rand(canvas.width);
  particleCache.y = center.value[1] + randRange(props.rangeY);
  particleCache.vx = 0;
  particleCache.vy = 0;
  particleCache.life = 0;
  particleCache.ttl = BASE_TTL + rand(RANGE_TTL);
  particleCache.speed = props.baseSpeed + rand(props.rangeSpeed);
  particleCache.radius = props.baseRadius + rand(props.rangeRadius);
  particleCache.hue = props.baseHue + rand(RANGE_HUE);
  saveParticle(i);
}

function saveParticle(i: number) {
  if (!particleProps.value) return;
  const arr = particleProps.value;
  const base = i * PARTICLE_PROP_COUNT;
  arr[base] = particleCache.x;
  arr[base + 1] = particleCache.y;
  arr[base + 2] = particleCache.vx;
  arr[base + 3] = particleCache.vy;
  arr[base + 4] = particleCache.life;
  arr[base + 5] = particleCache.ttl;
  arr[base + 6] = particleCache.speed;
  arr[base + 7] = particleCache.radius;
  arr[base + 8] = particleCache.hue;
}

function draw() {
  if (!ctx.value || !particleProps.value || !canvasRef.value) return;
  tick++;
  const canvas = canvasRef.value;
  const context = ctx.value;
  context.clearRect(0, 0, canvas.width, canvas.height);

  center.value = [0.5 * canvas.width, 0.5 * canvas.height];
  const arr = particleProps.value;

  for (let i = 0; i < props.particleCount; i++) {
    const base = i * PARTICLE_PROP_COUNT;
    let x = arr[base];
    let y = arr[base + 1];
    let vx = arr[base + 2];
    let vy = arr[base + 3];
    let life = arr[base + 4];
    let ttl = arr[base + 5];
    const speed = arr[base + 6];
    const radius = arr[base + 7];
    const hue = arr[base + 8];

    const n = NOISE_STEPS;
    for (let j = 0; j < n; j++) {
      const factor = (j + 1) / n;
      const nx = X_OFF * factor * (x - center.value[0]);
      const ny = Y_OFF * factor * (y - center.value[1]);
      const nz = Z_OFF * factor * tick;
      const noiseVal = noise3D(nx, ny, nz) * TAU * factor;
      vx += Math.cos(noiseVal) * 0.005 / factor;
      vy += Math.sin(noiseVal) * 0.005 / factor;
    }

    vx *= 0.999;
    vy *= 0.999;
    x += vx;
    y += vy;
    life++;

    if (life > ttl || x < -50 || x > canvas.width + 50 || y < -50 || y > canvas.height + 50) {
      particleCache.x = rand(canvas.width);
      particleCache.y = center.value[1] + randRange(props.rangeY);
      particleCache.vx = 0;
      particleCache.vy = 0;
      particleCache.life = 0;
      particleCache.ttl = BASE_TTL + rand(RANGE_TTL);
      particleCache.speed = props.baseSpeed + rand(props.rangeSpeed);
      particleCache.radius = props.baseRadius + rand(props.rangeRadius);
      particleCache.hue = props.baseHue + rand(RANGE_HUE);
      saveParticle(i);
      continue;
    }

    arr[base] = x;
    arr[base + 1] = y;
    arr[base + 2] = vx;
    arr[base + 3] = vy;
    arr[base + 4] = life;

    context.beginPath();
    context.arc(x, y, radius * fadeInOut(life, ttl), 0, TAU);
    context.fillStyle = `hsla(0, 0%, ${50 + fadeInOut(life, ttl) * 50}%, ${0.25 + fadeInOut(life, ttl) * 0.2})`;
    context.fill();
  }

  animationId = requestAnimationFrame(draw);
}

function resize() {
  if (!canvasRef.value) return;
  const canvas = canvasRef.value;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = window.innerWidth * dpr;
  canvas.height = window.innerHeight * dpr;
  canvas.style.width = window.innerWidth + "px";
  canvas.style.height = window.innerHeight + "px";
  if (ctx.value) ctx.value.scale(dpr, dpr);

  if (particleProps.value && particleProps.value.length !== props.particleCount * PARTICLE_PROP_COUNT) {
    particleProps.value = new Float32Array(props.particleCount * PARTICLE_PROP_COUNT);
  }
  if (!particleProps.value) {
    particleProps.value = new Float32Array(props.particleCount * PARTICLE_PROP_COUNT);
  }
  for (let i = 0; i < props.particleCount; i++) initParticle(i);
}

onMounted(() => {
  if (!canvasRef.value) return;
  ctx.value = canvasRef.value.getContext("2d");
  resize();
  window.addEventListener("resize", resize);
  animationId = requestAnimationFrame(draw);
});

onBeforeUnmount(() => {
  if (animationId) cancelAnimationFrame(animationId);
  window.removeEventListener("resize", resize);
});
</script>

<template>
  <div :class="cn('absolute inset-0 pointer-events-none', containerClass)">
    <canvas ref="canvasRef" :class="cn('block', props.class)" />
  </div>
</template>
