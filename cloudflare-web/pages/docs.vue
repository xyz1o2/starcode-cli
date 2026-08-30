<script setup lang="ts">
import { ref } from "vue";

const activeSection = ref("overview");

const sections = [
  { id: "overview", title: "Overview" },
  { id: "installation", title: "Installation" },
  { id: "quick-start", title: "Quick Start" },
  { id: "usage", title: "Usage" },
  { id: "configuration", title: "Configuration" },
  { id: "providers", title: "LLM Providers" },
  { id: "support", title: "Support" },
];

function scrollTo(id: string) {
  activeSection.value = id;
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth" });
}
</script>

<template>
  <div class="min-h-screen bg-[#000000] text-white">
    <!-- Header -->
    <header class="sticky top-0 z-50 border-b border-white/[0.15] bg-[#000000]/80 backdrop-blur-xl">
      <div class="max-w-7xl mx-auto px-6 h-16 flex items-center justify-between">
        <a href="/" class="flex items-center gap-2 text-base font-bold no-underline">
          StarCode CLI
        </a>
        <nav class="flex items-center gap-6">
          <a href="/docs" class="text-sm text-white/70 hover:text-white no-underline transition-colors">Docs</a>
          <a href="https://github.com/xyz1o2/starcode-cli/issues" target="_blank" class="text-sm text-white/70 hover:text-white no-underline transition-colors">Issues</a>
        </nav>
      </div>
    </header>

    <div class="max-w-7xl mx-auto flex">
      <!-- Sidebar -->
      <aside class="w-64 flex-shrink-0 border-r border-white/[0.15] py-8 px-4 sticky top-16 h-[calc(100vh-4rem)] overflow-y-auto hidden lg:block">
        <nav class="space-y-1">
          <button
            v-for="section in sections"
            :key="section.id"
            @click="scrollTo(section.id)"
            class="w-full text-left px-3 py-2 text-sm rounded-lg transition-all cursor-pointer"
            :class="activeSection === section.id
              ? 'bg-white/10 text-white font-medium'
              : 'text-white/40 hover:text-white/70 hover:bg-white/[0.04]'"
          >
            {{ section.title }}
          </button>
        </nav>
      </aside>

      <!-- Main Content -->
      <main class="flex-1 py-12 px-6 lg:px-12 max-w-4xl">
        <!-- Overview -->
        <section id="overview" class="mb-16">
          <h1 class="text-4xl font-black tracking-tight mb-6">Documentation</h1>
          <p class="text-lg text-white/60 leading-relaxed mb-8">
            A Rust-powered terminal coding assistant — fast, stable, and always evolving. Precisely tuned for Claude, GPT, DeepSeek, Xiaomi MiMo and more. All features are free today, with subscription plans coming in the future. Code without limits.
          </p>
          <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <div class="text-2xl font-bold text-white mb-2">Free</div>
              <div class="text-sm text-white/40">All Features Today</div>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <div class="text-2xl font-bold text-white mb-2">BYOLLM</div>
              <div class="text-sm text-white/40">Your Provider, Your Rules</div>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <div class="text-2xl font-bold text-white mb-2">∞</div>
              <div class="text-sm text-white/40">Unlimited Coding</div>
            </div>
          </div>
        </section>

        <!-- Installation -->
        <section id="installation" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">Installation</h2>

          <div class="space-y-6">
            <!-- npm (Recommended) -->
            <div class="rounded-xl border border-white/20 overflow-hidden">
              <div class="px-4 py-3 bg-white/5 border-b border-white/20 flex items-center gap-2">
                <span class="text-sm font-medium">npm (Recommended)</span>
                <span class="px-2 py-0.5 text-[10px] font-semibold rounded-full bg-green-500/10 text-green-400 border border-green-500/20">All Platforms</span>
              </div>
              <div class="p-4">
                <p class="text-sm text-white/40 mb-3">Install globally with npm (requires Node.js 18+):</p>
                <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                  <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">npm install -g starcode-cli</pre>
                </div>
                <p class="text-sm text-white/40 mt-3 mb-3">Or run directly without installing:</p>
                <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                  <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">npx starcode-cli</pre>
                </div>
              </div>
            </div>

            <!-- Linux -->
            <div class="rounded-xl border border-white/[0.15] overflow-hidden">
              <div class="px-4 py-3 bg-white/[0.02] border-b border-white/[0.15] flex items-center gap-2">
                <span class="text-sm font-medium">Linux (x86_64)</span>
                <span class="px-2 py-0.5 text-[10px] font-semibold rounded-full bg-green-500/10 text-green-400 border border-green-500/20">Available</span>
              </div>
              <div class="p-4">
                <p class="text-sm text-white/40 mb-3">Install with one command (no Node.js required):</p>
                <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                  <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">curl -fsSL https://raw.githubusercontent.com/xyz1o2/starcode-cli/main/install-npm.sh | bash</pre>
                </div>
                <p class="text-xs text-white/30 mt-3">
                  Requires <code class="px-1.5 py-0.5 rounded bg-white/[0.06] text-zinc-400">curl</code> and <code class="px-1.5 py-0.5 rounded bg-white/[0.06] text-zinc-400">npm</code>.
                </p>
              </div>
            </div>

            <!-- Build from source -->
            <div class="rounded-xl border border-white/[0.15] overflow-hidden">
              <div class="px-4 py-3 bg-white/[0.02] border-b border-white/[0.15] flex items-center gap-2">
                <span class="text-sm font-medium">Build from Source</span>
              </div>
              <div class="p-4">
                <p class="text-sm text-white/40 mb-3">Requires Rust toolchain (<code class="px-1.5 py-0.5 rounded bg-white/[0.06] text-zinc-400">rustup</code>):</p>
                <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                  <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">git clone https://github.com/xyz1o2/starcode-cli.git
cd starcode-cli/starcode-cli-main/starcode-cli
cargo build --release
cargo install --path .</pre>
                </div>
              </div>
            </div>
          </div>
        </section>

        <!-- Quick Start -->
        <section id="quick-start" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">Quick Start</h2>
          
          <div class="space-y-6">
            <div>
              <h3 class="text-lg font-semibold mb-3">1. Set up your API key</h3>
              <p class="text-sm text-white/40 mb-3">Configure your preferred LLM provider's API key:</p>
              <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">export ANTHROPIC_API_KEY="your-api-key-here"</pre>
              </div>
            </div>

            <div>
              <h3 class="text-lg font-semibold mb-3">2. Navigate to your project</h3>
              <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">cd /path/to/your/project</pre>
              </div>
            </div>

            <div>
              <h3 class="text-lg font-semibold mb-3">3. Start StarCode CLI</h3>
              <div class="relative bg-[#0a0a0a] border border-white/[0.15] rounded-lg p-4">
                <pre class="text-sm text-zinc-300 font-mono overflow-x-auto">starcode-cli</pre>
              </div>
              <p class="text-sm text-white/40 mt-3">You'll see an interactive prompt where you can describe what you want to do.</p>
            </div>
          </div>
        </section>

        <!-- Usage -->
        <section id="usage" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">Usage</h2>
          
          <div class="space-y-6">
            <div>
              <h3 class="text-lg font-semibold mb-3">Basic Commands</h3>
              <div class="space-y-3">
                <div class="rounded-lg border border-white/[0.15] overflow-hidden">
                  <div class="px-4 py-2 bg-white/[0.02] text-sm font-mono text-white">starcode-cli</div>
                  <div class="px-4 py-3 text-sm text-white/40">Start interactive mode in the current directory</div>
                </div>
                <div class="rounded-lg border border-white/[0.15] overflow-hidden">
                  <div class="px-4 py-2 bg-white/[0.02] text-sm font-mono text-white">starcode-cli --help</div>
                  <div class="px-4 py-3 text-sm text-white/40">Show all available options and commands</div>
                </div>
                <div class="rounded-lg border border-white/[0.15] overflow-hidden">
                  <div class="px-4 py-2 bg-white/[0.02] text-sm font-mono text-white">starcode-cli --version</div>
                  <div class="px-4 py-3 text-sm text-white/40">Display the current version</div>
                </div>
              </div>
            </div>

            <div>
              <h3 class="text-lg font-semibold mb-3">Example Prompts</h3>
              <div class="space-y-3">
                <div class="p-4 rounded-lg bg-white/[0.02] border border-white/[0.15]">
                  <p class="text-sm text-white/60 mb-2">Code editing:</p>
                  <code class="text-sm text-white">"Refactor the auth module to use JWT tokens"</code>
                </div>
                <div class="p-4 rounded-lg bg-white/[0.02] border border-white/[0.15]">
                  <p class="text-sm text-white/60 mb-2">Code explanation:</p>
                  <code class="text-sm text-white">"Explain the data flow in this repository"</code>
                </div>
                <div class="p-4 rounded-lg bg-white/[0.02] border border-white/[0.15]">
                  <p class="text-sm text-white/60 mb-2">Git operations:</p>
                  <code class="text-sm text-white">"Create a commit with a descriptive message"</code>
                </div>
              </div>
            </div>
          </div>
        </section>

        <!-- Configuration -->
        <section id="configuration" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">Configuration</h2>
          
          <p class="text-sm text-white/40 mb-6">
            StarCode CLI can be configured through environment variables.
          </p>

          <div class="space-y-4">
            <h3 class="text-lg font-semibold">Environment Variables</h3>
            <div class="rounded-xl border border-white/[0.15] overflow-hidden">
              <table class="w-full text-sm">
                <thead>
                  <tr class="bg-white/[0.02] border-b border-white/[0.15]">
                    <th class="px-4 py-3 text-left font-medium text-white/60">Variable</th>
                    <th class="px-4 py-3 text-left font-medium text-white/60">Description</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-white/[0.06]">
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">ANTHROPIC_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for Anthropic (Claude)</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">OPENAI_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for OpenAI (GPT-4)</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">DEEPSEEK_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for DeepSeek</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">MINIMAX_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for MiniMax</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">STEPFUN_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for StepFun (阶跃星辰)</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">XIAOMI_API_KEY</td>
                    <td class="px-4 py-3 text-white/40">API key for Xiaomi MiMo (all regions)</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">STARCODE_PROVIDER</td>
                    <td class="px-4 py-3 text-white/40">Default LLM provider to use</td>
                  </tr>
                  <tr>
                    <td class="px-4 py-3 font-mono text-white">STARCODE_MODEL</td>
                    <td class="px-4 py-3 text-white/40">Default model to use</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <!-- Providers -->
        <section id="providers" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">LLM Providers</h2>
          
          <p class="text-sm text-white/40 mb-6">
            StarCode CLI ships with the following LLM providers built in, precisely optimized for each. Configure the API key for your preferred provider to get started.
          </p>

          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">Claude</h4>
              <p class="text-sm text-white/40">Claude 4.5 Sonnet, Claude 4 Opus, Haiku — precisely optimized for code editing and reasoning.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">GPT</h4>
              <p class="text-sm text-white/40">GPT-4o, GPT-4 Turbo, GPT-3.5 Turbo — tuned for coding workflows.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">DeepSeek</h4>
              <p class="text-sm text-white/40">DeepSeek Coder, DeepSeek Chat — optimized for code generation and explanation.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">Xiaomi MiMo · Global</h4>
              <p class="text-sm text-white/40">Pay-as-you-go — mimo-v2-flash and more, tuned for coding tasks.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">Xiaomi MiMo · China (Token Plan)</h4>
              <p class="text-sm text-white/40">token-plan-cn endpoint — mimo-v2.5-pro and more.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">Xiaomi MiMo · Singapore (Token Plan)</h4>
              <p class="text-sm text-white/40">token-plan-sgp endpoint for Asia-Pacific.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">Xiaomi MiMo · Amsterdam (Token Plan)</h4>
              <p class="text-sm text-white/40">token-plan-ams endpoint for Europe.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">MiniMax</h4>
              <p class="text-sm text-white/40">MiniMax chat and completion models.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02]">
              <h4 class="font-semibold mb-2">StepFun</h4>
              <p class="text-sm text-white/40">StepFun — chat and reasoning models.</p>
            </div>
            <div class="p-4 rounded-xl border border-white/[0.15] bg-white/[0.02] md:col-span-2">
              <h4 class="font-semibold mb-2">OpenAI Compatible</h4>
              <p class="text-sm text-white/40">Point at any custom OpenAI-format endpoint — bring your own key, bring your own model.</p>
            </div>
          </div>
        </section>

        <!-- Support -->
        <section id="support" class="mb-16">
          <h2 class="text-3xl font-bold tracking-tight mb-6">Support</h2>
          
          <p class="text-sm text-white/40 mb-6">
            Need help or found a bug? We'd love to hear from you.
          </p>

          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <a href="https://github.com/xyz1o2/starcode-cli/issues" target="_blank" class="p-6 rounded-xl border border-white/[0.15] bg-white/[0.02] hover:bg-white/[0.04] transition-colors no-underline">
              <div class="text-2xl mb-3">🐛</div>
              <h4 class="font-semibold text-white mb-2">Report Issues</h4>
              <p class="text-sm text-white/40">Found a bug or have a feature request? Open an issue on GitHub.</p>
            </a>
            <a href="https://github.com/xyz1o2/starcode-cli/issues" target="_blank" class="p-6 rounded-xl border border-white/[0.15] bg-white/[0.02] hover:bg-white/[0.04] transition-colors no-underline">
              <div class="text-2xl mb-3">💬</div>
              <h4 class="font-semibold text-white mb-2">Discussions</h4>
              <p class="text-sm text-white/40">Have questions or want to share ideas? Start a discussion.</p>
            </a>
          </div>
        </section>

        <!-- Footer -->
        <footer class="mt-16 pt-8 border-t border-white/[0.15]">
          <div class="flex flex-col md:flex-row items-center justify-between gap-4">
            <div class="flex items-center gap-4">
              <a href="/" class="text-sm text-white/40 hover:text-white/60 no-underline transition-colors">Home</a>
              <a href="/docs" class="text-sm text-white/40 hover:text-white/60 no-underline transition-colors">Docs</a>
              <a href="https://github.com/xyz1o2/starcode-cli/issues" target="_blank" class="text-sm text-white/40 hover:text-white/60 no-underline transition-colors">Report Issue</a>
            </div>
            <p class="text-xs text-white/20">© 2026 StarCode CLI · Built with Rust</p>
          </div>
        </footer>
      </main>
    </div>
  </div>
</template>
