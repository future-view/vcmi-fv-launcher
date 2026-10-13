//! 弹窗操作区容器（共享层）。
//!
//! 统一处理「底部按钮在长 locale / 窄窗口下被挤压或溢出」的根因：
//! - `flex-wrap`：空间不足时整行换行而非溢出；
//! - `justify-end + gap`：按钮组整体靠右、间距一致；
//! - 子元素用 `me-auto` 把左侧项（如「跳过此版本」）推到左端，放不下时自动换到上一行。
//!
//! 按钮本身需自行加 `whitespace-nowrap`，文本才不会被折行撑高。

import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

interface Props {
  children: ReactNode;
  className?: string;
}

export function DialogFooter({ children, className }: Props) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-end gap-x-3 gap-y-2 border-t border-slate-800 px-5 py-3.5",
        className,
      )}
    >
      {children}
    </div>
  );
}
