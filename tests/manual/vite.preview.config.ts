import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Separate root: preview fixtures never become production entrypoints.
export default defineConfig({
  root: path.resolve(__dirname, "../.."),
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(__dirname, "../../src") } },
  server: { host: "127.0.0.1", port: 4178, strictPort: true, open: false },
});
