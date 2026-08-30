<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";

// ── State ──
const scrollY = ref(0);
const installPlatform = ref("linux");
const mounted = ref(false);

// ── Data ──
const installCmd = "npm install -g starcode-cli";

// ── Copy to clipboard ──
function copyText(text: string, btn: HTMLElement) {
  navigator.clipboard.writeText(text).then(() => {
    const orig = btn.innerHTML;
    btn.innerHTML = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>`;
    btn.style.color = "#22c55e";
    setTimeout(() => {
      btn.innerHTML = orig;
      btn.style.color = "";
    }, 1800);
  });
}

// ── Reveal on scroll ──
let observer: IntersectionObserver | null = null;

onMounted(() => {
  mounted.value = true;
  window.addEventListener("scroll", () => { scrollY.value = window.scrollY; });

  observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((e) => {
        if (e.isIntersecting) {
          e.target.classList.add("revealed");
          observer?.unobserve(e.target);
        }
      });
    },
    { threshold: 0.1 }
  );
  document.querySelectorAll(".reveal").forEach((el) => observer?.observe(el));
});

onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="relative min-h-screen bg-[#000000] text-white overflow-x-hidden font-sans">

    <!-- Fullscreen starfield background -->
    <Starfield />

    <!-- ==================== NAVBAR ==================== -->
    <nav
      class="fixed top-0 left-0 right-0 z-50 transition-all duration-500"
      :class="scrollY > 50 ? 'bg-[#000000]/85 backdrop-blur-2xl border-b border-white/[0.15]' : 'bg-transparent'"
    >
      <div class="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between">
        <a href="/" class="flex items-center gap-2 font-bold no-underline text-white">
          StarCode CLI
        </a>
        <div class="flex items-center gap-5">
          <a href="/docs" class="text-sm text-white/50 hover:text-white no-underline transition-colors">Docs</a>
          <a
            href="https://github.com/xyz1o2/starcode-cli"
            target="_blank"
            class="text-sm font-semibold px-4 py-1.5 rounded-full border border-white/30 text-white hover:bg-white/10 no-underline transition-all"
          >GitHub</a>
        </div>
      </div>
    </nav>

    <!-- ==================== HERO ==================== -->
    <section class="relative min-h-screen flex flex-col items-center justify-center text-center px-6 pt-24 pb-20">
      <!-- Centered radial glow -->
      <div class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[700px] h-[700px] rounded-full bg-white/[0.03] blur-[140px] pointer-events-none" />

      <!-- 3D-style CSS title -->
      <h1 class="relative z-10 text-[clamp(2.6rem,8vw,5.5rem)] font-black leading-[1] tracking-tight mb-6">
        <span class="hero-3d block">vibe coding</span>
        <span class="hero-3d-accent block">in your Terminal</span>
      </h1>

      <p class="relative z-10 max-w-xl text-base sm:text-lg text-white/50 mb-10 leading-relaxed">
        More tools, more freedom. A Rust-powered coding companion precisely tuned for Claude, GPT, DeepSeek, Xiaomi MiMo — and more. Code without limits, stay in flow.
      </p>

      <!-- Install command -->
      <div class="relative z-10 w-full max-w-[520px] mb-12">
        <div class="flex items-center justify-between gap-3 bg-[#0a0a0a] border border-white/[0.15] rounded-xl px-4 py-3.5 font-mono text-sm text-zinc-200">
          <code class="truncate">{{ installCmd }}</code>
          <button
            @click="copyText(installCmd, $event.currentTarget as HTMLElement)"
            class="flex-shrink-0 p-1.5 rounded-md border border-white/[0.15] bg-white/[0.04] cursor-pointer text-white/40 hover:text-white/80 transition-all"
            title="Copy"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect x="9" y="9" width="13" height="13" rx="2"/>
              <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
            </svg>
          </button>
        </div>
      </div>

      <!-- 3D feature cards -->
      <div class="relative z-10 grid grid-cols-3 gap-3 sm:gap-5 max-w-2xl w-full">
        <TiltCard :intensity="18" class="feature-3d">
          <div class="feature-inner">
            <div class="text-2xl sm:text-3xl font-extrabold bg-gradient-to-br from-white to-[#888888] bg-clip-text text-transparent">Free</div>
            <div class="text-[10px] sm:text-xs font-medium text-white/40 uppercase tracking-wider mt-1">All Features</div>
          </div>
        </TiltCard>
        <TiltCard :intensity="18" class="feature-3d">
          <div class="feature-inner">
            <div class="text-2xl sm:text-3xl font-extrabold bg-gradient-to-br from-white to-[#888888] bg-clip-text text-transparent">BYOLLM</div>
            <div class="text-[10px] sm:text-xs font-medium text-white/40 uppercase tracking-wider mt-1">Your Choice</div>
          </div>
        </TiltCard>
        <TiltCard :intensity="18" class="feature-3d">
          <div class="feature-inner">
            <div class="text-2xl sm:text-3xl font-extrabold bg-gradient-to-br from-white to-[#888888] bg-clip-text text-transparent">∞</div>
            <div class="text-[10px] sm:text-xs font-medium text-white/40 uppercase tracking-wider mt-1">Code Without Limits</div>
          </div>
        </TiltCard>
      </div>

      <!-- Scroll cue -->
      <div class="absolute bottom-8 left-1/2 -translate-x-1/2 animate-bounce text-white/20">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M7 13l5 5 5-5M7 6l5 5 5-5"/>
        </svg>
      </div>
    </section>



    <!-- ==================== WHY ==================== -->
    <section class="py-24 px-6 bg-[#0a0a0a]">
      <div class="max-w-6xl mx-auto">
        <div class="text-center mb-14 reveal">
          <div class="text-[11px] font-semibold tracking-[2.5px] uppercase text-white mb-3">Why StarCode</div>
          <h2 class="text-[clamp(1.8rem,4vw,2.6rem)] font-black tracking-tight">
            More choices. More freedom.<br/>
            <span class="gradient-text">Built to last.</span>
          </h2>
        </div>

        <div class="grid sm:grid-cols-2 lg:grid-cols-3 gap-5">
          <TiltCard
            v-for="(f, i) in [
              { title: '100% Free, For Now', desc: 'Every feature is free today. Subscription plans coming later — but the core stays open and accessible.' },
              { title: 'Bring Your Own LLM', desc: 'Precisely tuned for Claude, GPT, DeepSeek, Xiaomi MiMo — plus any OpenAI-compatible API. You pick the brain.' },
              { title: 'Rust-Powered Speed', desc: 'Native binary, async I/O, zero-copy parsing. Millisecond startup, rock-solid stability over long sessions.' },
              { title: 'Constantly Evolving', desc: 'Continuous updates, new providers, better tooling. Your workflow keeps improving without switching apps.' },
              { title: 'Unlimited Coding', desc: 'No usage caps, no artificial limits. Code as long as you want, as deep as you need.' },
              { title: 'One Goal, One Tool', desc: 'A single CLI that does one thing well — AI-assisted coding in your terminal, without the bloat.' },
            ]"
            :key="i"
            :intensity="10"
            :scale="1.015"
            class="why-card reveal"
            :style="{ transitionDelay: `${i * 0.08}s` }"
          >
            <div class="why-inner">
              <div class="w-8 h-0.5 bg-white/50 mb-4 rounded-full"/>
              <h3 class="text-base font-bold mb-2">{{ f.title }}</h3>
              <p class="text-sm text-white/45 leading-relaxed">{{ f.desc }}</p>
            </div>
          </TiltCard>
        </div>
      </div>
    </section>

    <!-- ==================== PROVIDERS ==================== -->
    <section class="py-24 px-6">
      <div class="max-w-5xl mx-auto">
        <div class="text-center mb-14 reveal">
          <div class="text-[11px] font-semibold tracking-[2.5px] uppercase text-white mb-3">Providers</div>
          <h2 class="text-[clamp(1.8rem,4vw,2.6rem)] font-black tracking-tight">
            Precisely tuned for<br/>
            <span class="gradient-text">Claude, GPT, DeepSeek, Xiaomi — and more.</span>
          </h2>
        </div>

        <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3 reveal">
          <div
            v-for="p in [
              { name: 'Claude', desc: 'Opus · Sonnet · Haiku' },
              { name: 'GPT', desc: 'GPT-4o · GPT-4 Turbo' },
              { name: 'DeepSeek', desc: 'Coder · Chat' },
              { name: 'Xiaomi MiMo', desc: 'Global · CN · SG · EU' },
              { name: 'MiniMax', desc: 'Chat · Completion' },
              { name: 'StepFun', desc: 'Chat · Reasoning' },
              { name: 'OpenAI Compat.', desc: 'Any endpoint' },
              { name: '+ More', desc: 'Always expanding' },
            ]"
            :key="p.name"
            class="provider-card"
          >
            <div class="text-sm font-semibold text-white/80 mb-0.5">{{ p.name }}</div>
            <div class="text-[11px] text-white/30">{{ p.desc }}</div>
          </div>
        </div>

        <p class="text-center text-sm text-white/25 mt-8 reveal">Each provider is individually optimized for the best coding experience.</p>
      </div>
    </section>

    <!-- ==================== FOOTER ==================== -->
    <footer class="py-10 border-t border-white/[0.15]">
      <div class="max-w-6xl mx-auto px-6 flex flex-col sm:flex-row items-center justify-between gap-3 text-center sm:text-left">
        <div class="flex items-center gap-2 font-bold">
          StarCode CLI
        </div>
        <div class="flex gap-5 text-xs text-white/30">
          <a href="/docs" class="hover:text-white/50 no-underline transition-colors">Docs</a>
          <a href="https://github.com/xyz1o2/starcode-cli" target="_blank" class="hover:text-white/50 no-underline transition-colors">GitHub</a>
          <a href="https://github.com/xyz1o2/starcode-cli/issues" target="_blank" class="hover:text-white/50 no-underline transition-colors">Issues</a>
        </div>
        <p class="text-xs text-white/20">© 2026 · Built with Rust</p>
      </div>
    </footer>
  </div>
</template>

<style>
/* ── 3D hero title ── */
.hero-3d {
  background: linear-gradient(180deg, #ffffff 0%, rgba(255, 255, 255, 0.7) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  text-shadow: 0 1px 0 rgba(255, 255, 255, 0.05);
  transform: translateZ(0);
  animation: floatY 6s ease-in-out infinite;
}
.hero-3d-accent {
  background: linear-gradient(135deg, #ffffff 0%, #cccccc 60%, #888888 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  text-shadow: 0 2px 20px rgba(255, 255, 255, 0.3);
  animation: floatY 6s ease-in-out infinite reverse;
  animation-delay: -3s;
}
@keyframes floatY {
  0%, 100% { transform: translateY(0); }
  50%      { transform: translateY(-8px); }
}

/* ── 3D feature cards ── */
.feature-3d {
  border-radius: 14px;
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.01));
  border: 1px solid rgba(255, 255, 255, 0.08);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.05) inset,
    0 10px 30px -10px rgba(0, 0, 0, 0.5),
    0 0 0 1px rgba(255, 255, 255, 0.05);
  transition: box-shadow 0.4s ease, border-color 0.4s ease;
}
.feature-3d:hover {
  border-color: rgba(255, 255, 255, 0.3);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.08) inset,
    0 20px 50px -10px rgba(255, 255, 255, 0.25),
    0 0 0 1px rgba(255, 255, 255, 0.2);
}
.feature-inner {
  padding: 1.25rem 1rem;
  text-align: center;
  transform: translateZ(30px);
}

/* ── 3D "why" cards ── */
.why-card {
  border-radius: 14px;
  background: linear-gradient(135deg, #0a0a0a, #000000);
  border: 1px solid rgba(255, 255, 255, 0.06);
  box-shadow: 0 4px 20px -8px rgba(0, 0, 0, 0.5);
  transition: border-color 0.4s ease, box-shadow 0.4s ease;
}
.why-card:hover {
  border-color: rgba(255, 255, 255, 0.25);
  box-shadow: 0 10px 30px -10px rgba(255, 255, 255, 0.2);
}
.why-inner {
  padding: 1.75rem;
  transform: translateZ(20px);
}

/* ── Provider cards ── */
.provider-card {
  padding: 1rem 1.25rem;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.02);
  border: 1px solid rgba(255, 255, 255, 0.05);
  transition: border-color 0.3s ease, background 0.3s ease;
}
.provider-card:hover {
  border-color: rgba(255, 255, 255, 0.25);
  background: rgba(255, 255, 255, 0.04);
}
</style>
