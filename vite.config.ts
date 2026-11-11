import { defineConfig } from "vite";

// The same build serves the Tauri webview (desktop, Android) and, later, the read-only LAN view.
export default defineConfig({
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true, sourcemap: false },
});
