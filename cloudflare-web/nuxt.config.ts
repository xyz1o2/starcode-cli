import tailwindcss from "@tailwindcss/vite";

export default defineNuxtConfig({
  compatibilityDate: "2026-05-28",
  devtools: { enabled: true },

  css: ["~/assets/css/main.css"],

  vite: {
    plugins: [tailwindcss()],
  },

  nitro: {
    preset: "cloudflare-pages",
  },

  app: {
    head: {
      charset: "utf-8",
      viewport: "width=device-width, initial-scale=1.0",
      title: "StarCode CLI – AI Coding Assistant in Your Terminal",
      meta: [
        {
          name: "description",
          content:
            "StarCode CLI — Rust-powered AI coding assistant. Chat with Claude, GPT, DeepSeek, MiniMax, StepFun and Xiaomi MiMo in your terminal.",
        },
        {
          name: "keywords",
          content:
            "AI coding assistant, terminal AI, CLI AI, StarCode, Rust CLI, LLM terminal, GPT terminal, Claude CLI, DeepSeek, code assistant",
        },
        { name: "author", content: "StarCode CLI" },
        { name: "robots", content: "index, follow" },
        { name: "theme-color", content: "#000000" },
        {
          property: "og:title",
          content: "StarCode CLI – AI Coding Assistant in Your Terminal",
        },
        {
          property: "og:description",
          content: "Rust-powered AI coding assistant. Claude, GPT, DeepSeek, MiniMax, StepFun, Xiaomi MiMo. Native speed.",
        },
        { property: "og:url", content: "https://starcode.help/" },
        { property: "og:type", content: "website" },
        { name: "twitter:card", content: "summary_large_image" },
        {
          name: "twitter:title",
          content: "StarCode CLI – AI Coding Assistant",
        },
        {
          name: "twitter:description",
          content: "Rust-powered. Claude, GPT, DeepSeek, MiniMax, StepFun, Xiaomi MiMo. Native terminal speed.",
        },
      ],
      link: [
        { rel: "canonical", href: "https://starcode.help/" },
        { rel: "icon", type: "image/svg+xml", href: "/favicon.svg" },
        { rel: "preconnect", href: "https://fonts.googleapis.com" },
        { rel: "preconnect", href: "https://fonts.gstatic.com", crossorigin: "" },
        { rel: "stylesheet", href: "https://fonts.googleapis.com/css2?family=Press+Start+2P&family=VT323&display=swap" },
      ],
    },
  },
});
