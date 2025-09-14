/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'media',  // Use OS preference for light/dark
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}"
  ],
  theme: {
    extend: {
      colors: {
        // Example brand colors (replace with actual palette from light_master.png)
        primary: "#1e40af",
        secondary: "#64748b"
      }
    }
  },
  plugins: []
};
