import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa';
// ...existing code...

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    react(),
  // ...existing code...
    VitePWA({
      registerType: 'autoUpdate',
      manifest: {
        name: 'NovaPOS',
        short_name: 'NovaPOS',
        start_url: '/',
        display: 'standalone',
        background_color: '#ffffff',
        theme_color: '#317EFB'
        // (icons would be specified here in a real app)
      },
      workbox: {
        globPatterns: ['**/*.{js,css,html,png,svg,json}']
      }
    })
  ],
})
