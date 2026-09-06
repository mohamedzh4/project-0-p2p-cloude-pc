/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
  },
  test: {
    environment: "node",
    env: {
      VITE_MATCHMAKER_URL: "http://matchmaker.test",
    },
  },
});
