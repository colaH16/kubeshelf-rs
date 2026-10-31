/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
// Tests read the shared header CSS (?raw) to check its account contract; other CSS stays stubbed.
export default defineConfig({ plugins: [react()], build: { outDir: 'dist', emptyOutDir: true }, server: { proxy: { '/api': 'http://localhost:8080', '/auth': 'http://localhost:8080' } }, test: { css: { include: [/cola-header\.css/] } } });
