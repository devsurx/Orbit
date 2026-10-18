import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri injects TAURI_DEV_HOST when developing on a device/VM; everything else
// is the plain Vite story.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // The Rust side is rebuilt by cargo, never by Vite.
      ignored: ["**/crates/**", "**/registry/**", "**/target/**"],
    },
  },
  build: {
    target: "esnext",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
    emptyOutDir: true,
  },
});
