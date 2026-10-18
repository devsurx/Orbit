/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // One accent, per the design brief; everything else is near-black glass.
        accent: {
          DEFAULT: "#6e4aff",
          soft: "#8f77ff",
          deep: "#4526c9",
        },
        ink: {
          900: "#0b0b12",
          800: "#12121c",
          700: "#1a1a27",
          600: "#262635",
        },
      },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["ui-monospace", "SFMono-Regular", "monospace"],
      },
      keyframes: {
        bloom: {
          "0%": { opacity: "0", transform: "scale(.94)" },
          "100%": { opacity: "1", transform: "scale(1)" },
        },
        petal: {
          // The centring translate must live inside the keyframes. An animation
          // that fills forwards replaces the `translate-*` utilities with its own
          // transform, so leaving it out pins every petal by its top-left corner
          // — which pushes wide chips off the edge of the overlay.
          "0%": { opacity: "0", transform: "translate(-50%, -50%) scale(.7)" },
          "100%": { opacity: "1", transform: "translate(-50%, -50%) scale(1)" },
        },
      },
      animation: {
        bloom: "bloom 110ms ease-out both",
        // No fill mode: once it ends the utilities take the transform back, so
        // the hover scale keeps working.
        petal: "petal 130ms cubic-bezier(.2,.8,.3,1)",
      },
    },
  },
  plugins: [],
};
