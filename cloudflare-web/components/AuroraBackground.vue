<script setup lang="ts">
import { cn } from "~/lib/utils";

interface Props {
  class?: string;
}

const props = defineProps<Props>();
</script>

<template>
  <div
    :class="cn(
      'absolute inset-0 overflow-hidden pointer-events-none',
      props.class,
    )"
  >
    <div
      class="pointer-events-none absolute -inset-2.5 opacity-50 invert filter blur-[10px] will-change-transform after:animate-aurora after:absolute after:inset-0 after:mix-blend-difference after:content-[''] [background-image:var(--white-gradient),var(--aurora)] [background-size:300%,200%] [background-position:50%_50%,50%_50%] dark:invert-0 after:dark:[background-image:var(--dark-gradient),var(--aurora)]"
      :style="{
        '--aurora': 'repeating-linear-gradient(100deg,#ffffff_10%,#cccccc_15%,#999999_20%,#666666_25%,#ffffff_30%)',
        '--white-gradient': 'repeating-linear-gradient(100deg,#fff_0%,#fff_7%,transparent_10%,transparent_12%,#fff_16%)',
        '--dark-gradient': 'repeating-linear-gradient(100deg,#000_0%,#000_7%,transparent_10%,transparent_12%,#000_16%)',
        '--transparent': 'transparent',
        '--black': '#000',
        '--white': '#fff',
        maskImage: 'radial-gradient(ellipse_at_100%_0%,black_10%,transparent_70%)',
        WebkitMaskImage: 'radial-gradient(ellipse_at_100%_0%,black_10%,transparent_70%)',
      }"
    />
  </div>
</template>

<style scoped>
@keyframes aurora {
  0%, 100% { transform: translate(0, 0) scale(1); }
  25% { transform: translate(5%, -2%) scale(1.05); }
  50% { transform: translate(-3%, 3%) scale(0.95); }
  75% { transform: translate(2%, 1%) scale(1.02); }
}
.animate-aurora {
  animation: aurora 60s linear infinite;
}
</style>
