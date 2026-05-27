import typography from "@tailwindcss/typography";

/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // JupiterOS GUI palette (VS Code-inspired)
        vsc: {
          bg: "#1e1e1e",
          sidebar: "#252526",
          panel: "#1e1e1e",
          border: "#3c3c3c",
          title: "#323233",
          text: "#cccccc",
          muted: "#858585",
          accent: "#007acc",
          green: "#4ec9b0",
          red: "#f14c4c",
          yellow: "#cca700",
          blue: "#569cd6",
          orange: "#ce9178",
          selection: "#264f78",
          hover: "#2a2d2e",
          active: "#37373d",
          tab: "#1e1e1e",
          tabActive: "#1e1e1e",
          tabBorder: "#007acc",
          statusbar: "#007acc",
        },
        jupiter: {
          // ── Base (Brand Identity: Deep Space Black) ──
          bg: "#050508",
          surface: "#0a0a10",
          elevated: "#141420",
          border: "#1e1e2e",
          // ── Text ──
          text: "#e2e8f0",      // Soft White
          muted: "#94a3b8",     // Slate Gray
          dim: "#64748b",       // Muted Gray
          // ── Primary: Jupiter Orange ──
          orange: {
            DEFAULT: "#e8923a",
            dark: "#d97706",
            light: "#f59e0b",   // Amber Gold
            glow: "#fde49e",
          },
          // ── Secondary: Ice Blue ──
          blue: {
            DEFAULT: "#38bdf8",
            900: "#0c1a2e",
            700: "#1e3a5f",
            500: "#2d7bbf",
            400: "#38bdf8",
            200: "#7dd3fc",
          },
          // ── Amber Gold ──
          amber: {
            DEFAULT: "#f59e0b",
            900: "#3e331a",
            700: "#b45309",
            500: "#d97706",
            400: "#f59e0b",
            200: "#fcd34d",
          },
          // ── Status & Accent ──
          green: "#4ade80",     // Moon Green
          red: "#f85149",       // Error / Danger
          purple: "#a78bfa",    // Cosmic Purple
        },
      },
      fontFamily: {
        display: ['"Space Grotesk"', "sans-serif"],
        body: ['"Inter"', "sans-serif"],
      },
    },
  },
  plugins: [typography],
};
