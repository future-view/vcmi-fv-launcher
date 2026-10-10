import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}
interface State {
  error: Error | null;
}

/**
 * 全局错误边界：任何渲染期异常都会被捕获并显示到页面（而非白屏），
 * 便于调试时直接看到根因堆栈。调试构建下额外提示可按 F12 打开控制台。
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // 同时输出到控制台，方便 F12 查看完整堆栈
    console.error("应用渲染异常：", error, info);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="h-screen w-screen overflow-auto bg-slate-950 p-6 text-slate-200">
        <div className="mb-3 text-lg font-semibold text-red-400">
          应用渲染出错（白屏根因）：
        </div>
        <pre className="whitespace-pre-wrap break-words rounded-lg bg-slate-900 p-4 text-sm text-red-300">
          {error.message}
          {"\n\n"}
          {error.stack}
        </pre>
        <div className="mt-4 text-xs text-slate-500">
          调试提示：按 F12 打开开发者控制台可查看完整日志。
        </div>
      </div>
    );
  }
}
