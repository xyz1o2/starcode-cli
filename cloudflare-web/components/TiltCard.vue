<script setup lang="ts">
import { ref } from "vue";

interface Props {
  intensity?: number;
  scale?: number;
  perspective?: number;
  glare?: boolean;
  class?: string;
}

const props = withDefaults(defineProps<Props>(), {
  intensity: 14,
  scale: 1.02,
  perspective: 1200,
  glare: true,
});

const innerRef = ref<HTMLElement | null>(null);
const glareRef = ref<HTMLElement | null>(null);

function onMove(e: MouseEvent) {
  const el = innerRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const y = e.clientY - rect.top;
  const cx = rect.width / 2;
  const cy = rect.height / 2;
  const rotateY = ((x - cx) / cx) * props.intensity;
  const rotateX = -((y - cy) / cy) * props.intensity;
  el.style.transform = `perspective(${props.perspective}px) rotateX(${rotateX}deg) rotateY(${rotateY}deg) scale3d(${props.scale}, ${props.scale}, ${props.scale})`;

  if (glareRef.value) {
    glareRef.value.style.setProperty("--mx", `${(x / rect.width) * 100}%`);
    glareRef.value.style.setProperty("--my", `${(y / rect.height) * 100}%`);
  }
}

function onLeave() {
  const el = innerRef.value;
  if (!el) return;
  el.style.transform = `perspective(${props.perspective}px) rotateX(0deg) rotateY(0deg) scale3d(1, 1, 1)`;
}
</script>

<template>
  <div :class="['tilt-wrap', props.class]" @mousemove="onMove" @mouseleave="onLeave">
    <div ref="innerRef" class="tilt-inner">
      <slot />
      <div v-if="glare" ref="glareRef" class="tilt-glare" />
    </div>
  </div>
</template>

<style scoped>
.tilt-wrap {
  position: relative;
}
.tilt-inner {
  position: relative;
  transform-style: preserve-3d;
  will-change: transform;
  transition: transform 0.5s cubic-bezier(0.2, 0.8, 0.2, 1);
}
.tilt-glare {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  pointer-events: none;
  background: radial-gradient(
    circle at var(--mx, 50%) var(--my, 50%),
    rgba(255, 255, 255, 0.14),
    transparent 55%
  );
  opacity: 0;
  transition: opacity 0.3s ease;
  mix-blend-mode: overlay;
}
.tilt-wrap:hover .tilt-glare {
  opacity: 1;
}
</style>
