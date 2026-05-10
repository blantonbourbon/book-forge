import { defineConfig } from "astro/config";

export default defineConfig({
  output: "static",
  server: {
    host: "127.0.0.1",
    port: 3101,
  },
  vite: {
    server: {
      proxy: {
        "/api": "http://127.0.0.1:3100",
      },
    },
  },
});
