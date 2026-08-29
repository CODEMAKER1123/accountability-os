import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Subdued desktop palette, Linear-like. Single committed dark look.
        ink: {
          950: "#0b0d10",
          900: "#101317",
          850: "#14181d",
          800: "#191e24",
          700: "#232a32",
          600: "#2f3843",
          500: "#4a5561",
          400: "#6b7683",
          300: "#8f99a5",
          200: "#b6bec8",
          100: "#dde2e8",
          50: "#f2f4f6",
        },
        accent: {
          DEFAULT: "#5b8def",
          dim: "#3f66b8",
        },
        focus: "#4ea87c",
        supporting: "#7fa860",
        neutralcat: "#8f99a5",
        distracted: "#c96f5e",
        idlecat: "#5a636d",
        warn: "#d9a052",
      },
      fontFamily: {
        sans: [
          "Inter",
          "Segoe UI Variable",
          "Segoe UI",
          "system-ui",
          "-apple-system",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "Cascadia Code",
          "Consolas",
          "ui-monospace",
          "monospace",
        ],
      },
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1rem" }],
      },
    },
  },
  plugins: [],
} satisfies Config;
