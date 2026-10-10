import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "@/shell/ErrorBoundary";
import { installUiScale } from "@/lib/tauri";
import "./index.css";

/**
 * 先应用界面缩放（WebView 整页缩放）再渲染。
 *
 * 顺序很关键：缩放由「窗口逻辑尺寸 ÷ 设计稿 1280×720」得出，
 * 若先渲染再缩放，首帧会以错误尺寸闪一下。缩放失败不阻断启动。
 * 安装后还会订阅窗口尺寸变化，最大化 / 还原 / 拖动时自动重算（见 `installUiScale`）。
 */
async function bootstrap() {
  try {
    await installUiScale();
  } catch (e) {
    console.warn("界面缩放初始化失败", e);
  }
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

void bootstrap();
