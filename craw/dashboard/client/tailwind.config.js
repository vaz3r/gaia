/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,jsx}'],
  theme: {
    extend: {
      colors: {
        ink: {
          950: '#0a0e1a',
          900: '#101626',
          800: '#1a2237',
          700: '#26304d',
        },
        accent: {
          DEFAULT: '#38bdf8',
          dark: '#0ea5e9',
        },
      },
    },
  },
  plugins: [],
}