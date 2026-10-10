import iconv from "iconv-lite";

/**
 * 解码 Buffer → string，容忍多种编码。
 * 优先 utf-8，若含非法字节则回退 gbk/cp1252（VCMI 中文 mod.json 可能为 GBK）。
 */
export function decodeText(buf: Uint8Array): string {
  const b = Buffer.from(buf);
  // 去掉 UTF-8 BOM
  let bytes = b;
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    bytes = bytes.subarray(3);
  }
  const utf8 = bytes.toString("utf8");
  if (!utf8.includes("\uFFFD")) return utf8;
  try {
    return iconv.decode(bytes, "gbk");
  } catch {
    return iconv.decode(bytes, "cp1252");
  }
}

/**
 * 宽松 JSON 解析：容忍尾逗号与注释（VCMI mod.json 常见尾逗号）。
 * 先标准解析，失败则剥离尾逗号后重试。
 */
export function parseJsonLenient(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    // 剥离尾逗号：将 ",}" 或 ",]" 替换为 "}" 或 "]"
    const cleaned = text
      .replace(/,\s*}/g, "}")
      .replace(/,\s*\]/g, "]");
    try {
      return JSON.parse(cleaned);
    } catch (err) {
      console.error("[parseJsonLenient] 解析失败:", err);
      return null;
    }
  }
}
