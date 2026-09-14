import { defineConfig } from "vite";
import { resolve } from "node:path";

// Tauri drives this dev server; the fixed port and strict mode keep devUrl honest.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    target: "safari15",
    rollupOptions: {
      input: {
        hud: resolve(import.meta.dirname, "index.html"),
        settings: resolve(import.meta.dirname, "settings.html"),
      },
    },
  },
});
