import { defineConfig } from "astro/config";

export default defineConfig({
  output: "static",
  vite: {
    server: {
      proxy: {
        "/api": "http://127.0.0.1:3100",
      },
    },
  },
});
