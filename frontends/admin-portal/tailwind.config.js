/* eslint-env node */
/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'media',  // Use OS preference for light/dark mode
  content: [
    "./index.html",
    "./src/**/*.{js,jsx,ts,tsx}"
  ],
  theme: {
    extend: {
      colors: {
        // Brand colors (same placeholders as POS app)
        primary: "#1e40af",
        secondary: "#64748b"
      }
    }
  },
  plugins: []
};

