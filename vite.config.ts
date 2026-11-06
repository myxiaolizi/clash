import { defineConfig } from "vite";

// Tauri 需要固定端口且不能自动切换（否则 devUrl 对不上）
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: "src",
  publicDir: false,
  build: {
    // 输出到 Tauri 的 frontendDist
    outDir: "../dist",
    emptyOutDir: true,
    target: "chrome105", // Win11 WebView2 基线
    minify: "esbuild",
    sourcemap: false,
    // 单文件优先：减少小 chunk 请求，冷启动更快
    rollupOptions: {
      output: {
        manualChunks: undefined,
      },
    },
    assetsInlineLimit: 8192,
  },
  clearScreen: false,
  server: {
    port: 5273,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 5274,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
