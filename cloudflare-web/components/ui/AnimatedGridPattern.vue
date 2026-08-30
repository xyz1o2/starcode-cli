<script setup lang="ts">
import { cn } from "~/lib/utils";

interface Props {
  class?: string;
  width?: number;
  height?: number;
  numSquares?: number;
  maxOpacity?: number;
  duration?: number;
  repeatDelay?: number;
}

const props = withDefaults(defineProps<Props>(), {
  width: 40,
  height: 40,
  numSquares: 30,
  maxOpacity: 0.1,
  duration: 3,
  repeatDelay: 0.5,
});
</script>

<template>
  <svg
    :class="cn('pointer-events-none absolute inset-0 h-full w-full', props.class)"
    xmlns="http://www.w3.org/2000/svg"
    :width="width"
    :height="height"
    class="opacity-30"
  >
    <defs>
      <pattern
        id="grid-pattern"
        :width="width"
        :height="height"
        patternUnits="userSpaceOnUse"
      >
        <path
          :d="`M ${width} 0 L 0 0 0 ${height}`"
          fill="none"
          stroke="rgba(255,255,255,0.08)"
          stroke-width="0.5"
        />
      </pattern>
    </defs>
    <rect width="100%" height="100%" fill="url(#grid-pattern)" />
  </svg>
</template>
