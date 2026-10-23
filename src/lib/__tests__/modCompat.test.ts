import { describe, it, expect } from "vitest";
import {
  parseVcmiVersion,
  isModCompatible,
  compatRangeText,
} from "@/lib/modCompat";

/**
 * `modCompat` 复刻 VCMI 引擎的 `CModVersion::compatible` 语义，是最易误判、
 * 一旦错了就会让整批模组误报「不兼容」的纯逻辑，故重点回归。
 *
 * 关键规则（见 modCompat.ts 头注）：
 *  1. major 必须相等（`max:"1.7"` 的模组在 1.8 上判**不兼容**，而非向下兼容）；
 *  2. 只比较双方都声明了的段（任一侧缺该段 = Any = 跳过）；
 *  3. min 要求 引擎 ≥ min，max 要求 引擎 ≤ max。
 */
describe("parseVcmiVersion", () => {
  it("解析标准三段版本", () => {
    expect(parseVcmiVersion("1.7.4")).toEqual({ major: 1, minor: 7, patch: 4 });
  });

  it("超过 3 段截取前 3 段（PE FileVersion 常带第 4 段）", () => {
    expect(parseVcmiVersion("1.8.0.0")).toEqual({ major: 1, minor: 8, patch: 0 });
  });

  it("只写 major 时 minor/patch 为 null（Any）", () => {
    expect(parseVcmiVersion("1")).toEqual({ major: 1, minor: null, patch: null });
  });

  it("只写到 minor 时 patch 为 null", () => {
    expect(parseVcmiVersion("1.7")).toEqual({ major: 1, minor: 7, patch: null });
  });

  it("某段非数字 → 该段起视为未声明", () => {
    expect(parseVcmiVersion("1.x.3")).toEqual({ major: 1, minor: null, patch: null });
  });

  it("空串 / null / undefined → null", () => {
    expect(parseVcmiVersion("")).toBeNull();
    expect(parseVcmiVersion(null)).toBeNull();
    expect(parseVcmiVersion(undefined)).toBeNull();
  });

  it("首段非数字 → null", () => {
    expect(parseVcmiVersion("abc")).toBeNull();
  });

  it("容忍前后空白", () => {
    expect(parseVcmiVersion("  1.7.4  ")).toEqual({ major: 1, minor: 7, patch: 4 });
  });
});

describe("isModCompatible", () => {
  it("未声明 compatibility → 兼容", () => {
    expect(isModCompatible("1.7.4", null)).toBe(true);
    expect(isModCompatible("1.7.4", undefined)).toBe(true);
    expect(isModCompatible("1.7.4", {})).toBe(true);
  });

  it("引擎版本未知 → 宁可放过，判兼容", () => {
    expect(isModCompatible(null, { min: "1.7" })).toBe(true);
    expect(isModCompatible("", { min: "1.7" })).toBe(true);
    expect(isModCompatible("Unknown", { min: "1.7" })).toBe(true);
  });

  it("major 必须相等：max=1.7 的模组在 1.8 上不兼容", () => {
    expect(isModCompatible("1.8.0", { max: "1.7" })).toBe(false);
  });

  it("min 语义是「引擎 ≥ min」：同 major 下更高 minor 兼容", () => {
    expect(isModCompatible("1.8.0", { min: "1.7" })).toBe(true);
  });

  it("major 必须相等：min=1.7 的模组在 2.0 上不兼容（major 跃迁）", () => {
    expect(isModCompatible("2.0.0", { min: "1.7" })).toBe(false);
  });

  it("min 只写到 minor 时不比较 patch", () => {
    expect(isModCompatible("1.7.4", { min: "1.7" })).toBe(true);
    expect(isModCompatible("1.7.0", { min: "1.7" })).toBe(true);
  });

  it("min 写到 patch 时按 patch 比较", () => {
    expect(isModCompatible("1.7.4", { min: "1.7.5" })).toBe(false);
    expect(isModCompatible("1.7.5", { min: "1.7.5" })).toBe(true);
    expect(isModCompatible("1.7.9", { min: "1.7.5" })).toBe(true);
  });

  it("max 区间：引擎 ≤ max 才兼容", () => {
    expect(isModCompatible("1.7.4", { max: "1.7.9" })).toBe(true);
    expect(isModCompatible("1.7.9", { max: "1.7.4" })).toBe(false);
  });

  it("min + max 双区间同时成立才兼容", () => {
    expect(isModCompatible("1.7.5", { min: "1.7.0", max: "1.7.9" })).toBe(true);
    expect(isModCompatible("1.7.5", { min: "1.7.6", max: "1.7.9" })).toBe(false);
    expect(isModCompatible("1.7.5", { min: "1.7.0", max: "1.7.4" })).toBe(false);
  });

  it("min 只写 major（1）时同 major 全放行", () => {
    expect(isModCompatible("1.7.4", { min: "1" })).toBe(true);
    expect(isModCompatible("1.0.0", { min: "1" })).toBe(true);
  });
});

describe("compatRangeText", () => {
  it("无区间 → null（调用方不渲染该行）", () => {
    expect(compatRangeText(null)).toBeNull();
    expect(compatRangeText({})).toBeNull();
    expect(compatRangeText({ min: "", max: "" })).toBeNull();
  });

  it("仅 min → ≥ x", () => {
    expect(compatRangeText({ min: "1.7.0" })).toBe("≥ 1.7.0");
  });

  it("仅 max → ≤ y", () => {
    expect(compatRangeText({ max: "1.8" })).toBe("≤ 1.8");
  });

  it("min + max → 区间", () => {
    expect(compatRangeText({ min: "1.6.0", max: "1.8" })).toBe("1.6.0 – 1.8");
  });
});
