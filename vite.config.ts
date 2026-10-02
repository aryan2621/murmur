import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// The app window is a React page (app.html); the floating widget is a static page in public/.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    // Local desktop app: one bundle is fine.
    chunkSizeWarningLimit: 1500,
    rollupOptions: { input: { app: path.resolve(__dirname, "app.html") } },
  },
});
