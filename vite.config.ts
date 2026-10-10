import { defineConfig, loadEnv, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";
import fs from "fs";

/**
 * Mock 缓存根目录（浏览器自测模式数据源）。
 * 不硬编码：优先读环境变量 VCMI_MOCK_CACHE_DIR（.env.local 或启动参数），
 * 默认回退到项目内符号链接的测试缓存目录。
 * 约定目录结构： <root>/mods/<modId>/mod.json、<root>/mods/<modId>/images/...、<root>/mods/index.json
 *
 * 注意：vite 不会把 .env 注入 `process.env`，必须用 loadEnv 读取。
 */
function resolveMockCacheRoot(mode: string): string {
  const env = loadEnv(mode, process.cwd(), "");
  const fromEnv = env.VCMI_MOCK_CACHE_DIR || process.env.VCMI_MOCK_CACHE_DIR;
  const root = fromEnv
    ? path.resolve(fromEnv)
    : path.resolve(__dirname, "references/Heroes3-VCMI/.vcmi-fv-launcher");
  if (process.env.VCMI_MOCK_DEBUG) {
    console.log("[vite] MOCK_CACHE_ROOT =", root);
  }
  return root;
}

/** dev server 中间件：/mock-cache/** → 直接读取缓存目录文件（实时，无需同步拷贝） */
function mockCachePlugin(mockCacheRoot: string): Plugin {
  return {
    name: "vcmi-mock-cache",
    configureServer(server) {
      server.middlewares.use("/mock-cache", (req, res) => {
        try {
          const url = (req.url || "").split("?")[0];
          const rel = decodeURIComponent(url).replace(/^\/+/, "");
          const base = path.resolve(mockCacheRoot);
          const fp = path.resolve(base, rel);
          if (process.env.VCMI_MOCK_DEBUG) {
            console.log("[mock-cache]", req.url, "->", fp, "exists=", fs.existsSync(fp));
          }
          if (!fp.startsWith(base)) {
            res.statusCode = 403;
            res.end();
            return;
          }
          if (!fs.existsSync(fp) || !fs.statSync(fp).isFile()) {
            res.statusCode = 404;
            res.end(`mock-cache miss: ${rel}`);
            return;
          }
          const ext = path.extname(fp).toLowerCase();
          const types: Record<string, string> = {
            ".json": "application/json",
            ".webp": "image/webp",
            ".png": "image/png",
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".bmp": "image/bmp",
          };
          res.setHeader("Content-Type", types[ext] ?? "application/octet-stream");
          res.setHeader("Cache-Control", "no-cache");
          fs.createReadStream(fp).pipe(res);
        } catch {
          res.statusCode = 500;
          res.end();
        }
      });
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => ({
  plugins: [react(), mockCachePlugin(resolveMockCacheRoot(mode))],
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    host: "0.0.0.0",
    allowedHosts: true,
    port: 5173,
    strictPort: false,
    // references/ 是本地符号链接（VCMI 源码 / 游戏安装目录）。当系统拒绝访问链接时
    // chokidar 遍历目录会 stat 失败并终止 dev server，故排除监听。
    watch: {
      ignored: ["**/references/**"],
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
}));
