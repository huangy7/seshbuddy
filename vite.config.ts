import { resolve } from "path";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    // `scripts/*.test.mjs` 里几条用例跑的是**真实脚本**（遍历 src/、解析 .vue、读语言包），
    // 是 I/O 密集的：单独跑约 1–5s，全量并行争用下会超过 vitest 默认的 5s 而假红
    // （实测 5193 / 5301 / 5656ms）。CI runner 更慢，只会更容易复现。
    testTimeout: 30000,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
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
      // 3. tell Vite to ignore watching `src-tauri`
      // workspace 的 cargo 产物在仓库根部 target/（tauri dev 期间持续写入，
      // watcher 不忽略会跑爆文件句柄/内存导致 dev server 中途退出）
      ignored: ["**/src-tauri/**", "**/target/**", "**/dist/**", "**/graphify-out/**"],
    },
  },

  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        assistant: resolve(__dirname, "src/assistant.html"),
      },
    },
  },
}));
