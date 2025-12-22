import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "url";
import { dirname, resolve } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 3000,
    allowedHosts: ["localhost", "127.0.0.1", "dockyy.galeri.ee"],
    proxy: {
      "/api": {
        target: "http://localhost:8012",
        changeOrigin: true,
      },
    },
  },
});
