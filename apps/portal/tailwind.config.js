/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        gaia: {
          50: '#f4f6fb',
          100: '#e8ecf7',
          500: '#3b82f6',
          600: '#2563eb',
          700: '#1d4ed8',
          800: '#0f172a',
          900: '#0b0f19',
          950: '#06090e',
        }
      }
    },
  },
  plugins: [],
}
