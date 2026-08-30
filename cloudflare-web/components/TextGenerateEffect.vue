<script setup lang="ts">
import { cn } from "~/lib/utils";
import { computed, onMounted, ref } from "vue";

interface Props {
  words: string;
  filter?: boolean;
  duration?: number;
  delay?: number;
  class?: string;
}

const props = withDefaults(defineProps<Props>(), {
  duration: 0.7,
  delay: 0,
  filter: true,
});

const wordsArray = computed(() => props.words.split(" "));
const visibleCount = ref(0);

onMounted(() => {
  setTimeout(() => {
    wordsArray.value.forEach((_, i) => {
      setTimeout(() => {
        visibleCount.value = i + 1;
      }, i * 200);
    });
  }, props.delay);
});
</script>

<template>
  <div :class="cn('leading-snug tracking-wide', props.class)">
    <span
      v-for="(word, idx) in wordsArray"
      :key="idx"
      class="inline-block transition-[opacity,filter]"
      :class="idx < visibleCount ? 'opacity-100' : 'opacity-0'"
      :style="{
        filter: idx < visibleCount
          ? 'blur(0px)'
          : props.filter ? 'blur(10px)' : 'none',
        transitionDuration: `${props.duration}s`,
      }"
    >{{ word }}&nbsp;</span>
  </div>
</template>
