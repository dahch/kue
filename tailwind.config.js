/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: {
          950: "#08090B",
          900: "#0C0E11",
          850: "#111318",
          800: "#16181E",
          750: "#1C1F26",
          700: "#242832",
        },
        volt: {
          300: "#DCFF6B",
          400: "#C9F24B",
          500: "#B5E02F",
          600: "#93BC1F",
          900: "#2A330F",
        },
        signal: {
          red: "#FF5D5D",
          amber: "#FFB454",
          blue: "#6EC8FF",
          violet: "#C9A0FF",
        },
      },
      fontFamily: {
        sans: ['"Bricolage Grotesque Variable"', "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ['"JetBrains Mono Variable"', "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      boxShadow: {
        card: "0 1px 0 0 rgba(255,255,255,0.04) inset, 0 8px 24px -12px rgba(0,0,0,0.6)",
        pop: "0 12px 40px -12px rgba(0,0,0,0.75), 0 0 0 1px rgba(255,255,255,0.06)",
        volt: "0 0 0 1px rgba(201,242,75,0.35), 0 8px 32px -8px rgba(201,242,75,0.25)",
        danger: "0 0 0 1px rgba(255,93,93,0.4), 0 8px 32px -8px rgba(255,93,93,0.3)",
      },
      keyframes: {
        "fade-up": {
          from: { opacity: "0", transform: "translateY(8px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
        "fade-in": {
          from: { opacity: "0" },
          to: { opacity: "1" },
        },
        "scale-in": {
          from: { opacity: "0", transform: "scale(0.96) translateY(6px)" },
          to: { opacity: "1", transform: "scale(1) translateY(0)" },
        },
        "pulse-dot": {
          "0%, 100%": { opacity: "1", boxShadow: "0 0 0 0 rgba(255,93,93,0.55)" },
          "50%": { opacity: "0.75", boxShadow: "0 0 0 6px rgba(255,93,93,0)" },
        },
        "pulse-volt": {
          "0%, 100%": { opacity: "1", boxShadow: "0 0 0 0 rgba(201,242,75,0.5)" },
          "50%": { opacity: "0.7", boxShadow: "0 0 0 6px rgba(201,242,75,0)" },
        },
        shimmer: {
          from: { transform: "translateX(-100%)" },
          to: { transform: "translateX(250%)" },
        },
        "eq-bar": {
          "0%, 100%": { transform: "scaleY(0.35)" },
          "50%": { transform: "scaleY(1)" },
        },
      },
      animation: {
        "fade-up": "fade-up 0.35s cubic-bezier(0.22, 1, 0.36, 1) both",
        "fade-in": "fade-in 0.3s ease both",
        "scale-in": "scale-in 0.25s cubic-bezier(0.22, 1, 0.36, 1) both",
        "pulse-dot": "pulse-dot 1.6s ease-in-out infinite",
        "pulse-volt": "pulse-volt 1.6s ease-in-out infinite",
        "eq-bar": "eq-bar 1s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
