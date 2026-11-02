/** @type {import('tailwindcss').Config} */
module.exports = {
  mode: "all",
  content: [
    "./apps/legacy/kopuz/**/*.{rs,html,css}",
    "./apps/legacy/components/**/*.{rs,html,css}",
    "./apps/legacy/pages/**/*.{rs,html,css}",
    "./apps/legacy/hooks/**/*.{rs,html,css}",
    "./daemon/player/**/*.{rs,html,css}",
    "./daemon/reader/**/*.{rs,html,css}",
    "./daemon/server/**/*.{rs,html,css}",
    "./shared/utils/**/*.{rs,html,css}",
    "./shared/config/**/*.{rs,html,css}",
    "./apps/legacy/kopuz_route/**/*.{rs,html,css}",
  ],
  theme: {
    extend: {
      colors: {
        black: "var(--color-black)",
        white: "var(--color-white)",
        slate: {
          400: "var(--color-slate-400)",
          500: "var(--color-slate-500)",
        },
        green: {
          400: "var(--color-green-400)",
          500: "var(--color-green-500)",
        },
        indigo: {
          400: "var(--color-indigo-400)",
          500: "var(--color-indigo-500)",
          600: "var(--color-indigo-600)",
          900: "var(--color-indigo-900)",
        },
        purple: {
          400: "var(--color-purple-400)",
          500: "var(--color-purple-500)",
          600: "var(--color-purple-600)",
          700: "var(--color-purple-700)",
        },

        red: {
          400: "var(--color-red-400)",
          500: "var(--color-red-500)",
        },
        orange: {
          500: "var(--color-orange-500)",
        },
        neutral: {
          900: "var(--color-neutral-900)",
        },
      },
    },
  },

  plugins: [],
};
