import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并 Tailwind class 工具 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
