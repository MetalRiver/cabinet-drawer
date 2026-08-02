import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue()],
  // 关键修复：Vite 打包必须配置 @ 别名（tsconfig.json 的 paths 只给 tsc 类型检查用，Rollup 不认）
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  base: "./",  // 关键修复：相对路径 base，tauri webview2 才能找到 assets/*
  build: {
    emptyOutDir: true,
    chunkSizeWarningLimit: 1500,
    // 关键修复：禁用 modulePreload，tauri webview2 对 modulepreload link 支持差
    // 12 个 webview2 反复重启的根因 —— Vite 生成的 modulepreload link 在 webview2 加载失败
    // 导致 JS 入口永远 hang，整个 Vue App 不 mount
    modulePreload: false,
    // 把所有小 chunk 合并到一个文件，避免 modulepreload 依赖
    rollupOptions: {
      output: {
        manualChunks: undefined,
      },
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));