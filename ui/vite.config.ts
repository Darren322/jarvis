/// <reference types="vitest/config" />
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// base './' keeps asset paths relative so dist/index.html loads from file://.
export default defineConfig({
  base: './',
  plugins: [react()],
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.{ts,tsx}'],
  },
});
