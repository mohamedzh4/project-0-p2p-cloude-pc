import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri drives the dev server on a fixed port and needs failures to be loud
// rather than silently falling back to another port.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
