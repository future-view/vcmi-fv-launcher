import { describe, it, expect } from "vitest";
import iconv from "iconv-lite";
import { decodeText, parseJsonLenient } from "@/lib/enc";

/**
 * `enc` 是项目「容错铁律」的一环：VCMI 中文 mod.json 常为 GBK、且常带尾逗号。
 * 解码回退与宽松解析一旦失效，模组会静默丢名或解析失败，故重点回归。
 */
describe("decodeText", () => {
  it("合法 UTF-8 原样返回", () => {
    expect(decodeText(Buffer.from("hello world", "utf8"))).toBe("hello world");
  });

  it("剥离 UTF-8 BOM", () => {
    const bom = Buffer.from([0xef, 0xbb, 0xbf]);
    const body = Buffer.from("名称", "utf8");
    expect(decodeText(Buffer.concat([bom, body]))).toBe("名称");
  });

  it("非法 UTF-8 字节回退 GBK（VCMI 中文 mod.json 常见）", () => {
    const gbk = iconv.encode("中文模组名称", "gbk");
    // 先确认这些字节按 utf-8 解会产生替换符（触发回退的前提）
    expect(gbk.toString("utf8")).toContain("\uFFFD");
    expect(decodeText(gbk)).toBe("中文模组名称");
  });

  it("接受普通 Uint8Array（非 Buffer）入参", () => {
    const bytes = new Uint8Array(Buffer.from("ascii", "utf8"));
    expect(decodeText(bytes)).toBe("ascii");
  });
});

describe("parseJsonLenient", () => {
  it("标准 JSON 直接解析", () => {
    expect(parseJsonLenient('{"a":1,"b":[2,3]}')).toEqual({ a: 1, b: [2, 3] });
  });

  it("容忍对象尾逗号", () => {
    expect(parseJsonLenient('{"a":1,"b":2,}')).toEqual({ a: 1, b: 2 });
  });

  it("容忍数组尾逗号", () => {
    expect(parseJsonLenient('{"list":[1,2,3,]}')).toEqual({ list: [1, 2, 3] });
  });

  it("无法解析时返回 null（不抛异常）", () => {
    expect(parseJsonLenient("{ this is not json")).toBeNull();
  });

  it("空串返回 null", () => {
    expect(parseJsonLenient("")).toBeNull();
  });
});
