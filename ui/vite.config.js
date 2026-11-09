import { defineConfig } from 'vite';
export default defineConfig({
  base: './',
  build: { target: 'es2022', outDir: 'dist', chunkSizeWarningLimit: 4000 },
  define: { global: 'globalThis' },
});
