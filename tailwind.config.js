/** @type {import('tailwindcss').Config} */
export default {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        border: "hsl(220 20% 20%)",
        background: "#0F172A",
        foreground: "#E2E8F0",
        primary: "#3B82F6",
        secondary: "#6366F1",
        muted: "#94A3B8",
        destructive: "#EF4444",
        success: "#22C55E",
        warning: "#F59E0B",
      },
      fontFamily: {
        sans: ['"PingFang SC"', '"Microsoft YaHei"', "system-ui", "sans-serif"],
      },
      backdropBlur: {
        glass: "16px",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
};
