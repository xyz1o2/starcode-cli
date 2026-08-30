<script setup lang="ts">
import { cn } from "~/lib/utils";
import { computed } from "vue";

interface Props {
  borderWidth?: number;
  colors?: string[];
  duration?: number;
  borderRadius?: number;
  blur?: number;
  class?: string;
  bgColor?: string;
}

const props = withDefaults(defineProps<Props>(), {
  colors: () => ["#ffffff", "#cccccc", "#999999", "#666666", "#cccccc", "#ffffff"],
  duration: 2500,
  borderWidth: 2,
  borderRadius: 8,
  blur: 4,
  bgColor: "#000000",
});

const durationMs = computed(() => `${props.duration}ms`);
const allColors = computed(() => props.colors.join(", "));
const bw = computed(() => `${props.borderWidth}px`);
const br = computed(() => `${props.borderRadius}px`);
const blurPx = computed(() => `${props.blur}px`);
</script>

<template>
  <button
    :class="cn(
      'animate-rainbow rainbow-btn relative flex min-h-10 min-w-28 items-center justify-center overflow-hidden before:absolute before:-inset-[200%]',
      props.class,
    )"
  >
    <span class="btn-content inline-flex size-full items-center justify-center px-4 py-2">
      <slot />
    </span>
  </button>
</template>

<style scoped>
.animate-rainbow::before {
  content: "";
  background: conic-gradient(v-bind(allColors));
  animation: rotate-rainbow v-bind(durationMs) linear infinite;
  filter: blur(v-bind(blurPx));
  padding: v-bind(bw);
}
.rainbow-btn {
  padding: v-bind(bw);
  border-radius: v-bind(br);
}
.btn-content {
  border-radius: v-bind(br);
  background-color: v-bind(bgColor);
  z-index: 0;
}
@keyframes rotate-rainbow {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}
</style>
