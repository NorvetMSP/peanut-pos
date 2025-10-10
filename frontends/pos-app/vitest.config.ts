import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config'

export default mergeConfig(viteConfig, defineConfig({
  test: {
    environment: 'jsdom',
    clearMocks: true,
    exclude: [
      'e2e/**',
      'node_modules/**',
      'dist/**'
    ]
  },
}))
