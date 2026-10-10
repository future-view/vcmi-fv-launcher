import yauzl from "yauzl";

/**
 * 从 zip 文件中按路径精准读取单个条目，不解压整包。
 * @param zipPath zip 文件绝对路径
 * @param entryPath 目标条目路径（大小写不敏感匹配）
 * @returns Buffer，找不到或读取失败返回 null
 */
export function readZipEntry(zipPath: string, entryPath: string): Promise<Buffer | null> {
  return new Promise((resolve) => {
    yauzl.open(zipPath, { lazyEntries: true }, (err, zipfile) => {
      if (err || !zipfile) {
        console.warn(`[zip] 打开失败 ${zipPath}:`, err?.message ?? "no zipfile");
        resolve(null);
        return;
      }
      const target = normalizePath(entryPath);
      let done = false;

      const finish = (buf: Buffer | null) => {
        if (done) return;
        done = true;
        zipfile.close();
        resolve(buf);
      };

      zipfile.readEntry();
      zipfile.on("entry", (entry: yauzl.Entry) => {
        if (normalizePath(entry.fileName) === target) {
          zipfile.openReadStream(entry, (rerr, stream) => {
            if (rerr || !stream) {
              // 加密/损坏：读取失败
              console.warn(`[zip] 读取条目失败 ${entryPath}:`, rerr?.message ?? "no stream");
              finish(null);
              return;
            }
            const chunks: Buffer[] = [];
            stream.on("data", (c: Buffer) => chunks.push(c));
            stream.on("end", () => finish(Buffer.concat(chunks)));
            stream.on("error", () => finish(null));
          });
        } else {
          zipfile.readEntry();
        }
      });
      zipfile.on("error", () => finish(null));
      zipfile.on("end", () => finish(null));
    });
  });
}

/** 判断 zip 文件是否加密（读取失败也返回 false，调用方自行容错） */
export function isZipEncrypted(zipPath: string): Promise<boolean> {
  return new Promise((resolve) => {
    yauzl.open(zipPath, { lazyEntries: true }, (err, zipfile) => {
      if (err || !zipfile) {
        resolve(false);
        return;
      }
      let encrypted = false;
      zipfile.on("entry", (entry: yauzl.Entry) => {
        // bit 0 = 加密
        if (entry.generalPurposeBitFlag & 0x1) {
          encrypted = true;
          zipfile.close();
          resolve(true);
          return;
        }
        zipfile.readEntry();
      });
      zipfile.on("end", () => {
        zipfile.close();
        resolve(encrypted);
      });
      zipfile.on("error", () => {
        zipfile.close();
        resolve(false);
      });
      zipfile.readEntry();
    });
  });
}

/** 归一化路径：统一分隔符、转小写，用于大小写不敏感匹配 */
function normalizePath(p: string): string {
  return p.replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
}

/** 列出 zip 内所有条目名（用于调试/定位） */
export function listZipEntries(zipPath: string): Promise<string[]> {
  return new Promise((resolve) => {
    yauzl.open(zipPath, { lazyEntries: true }, (err, zipfile) => {
      if (err || !zipfile) {
        resolve([]);
        return;
      }
      const names: string[] = [];
      zipfile.readEntry();
      zipfile.on("entry", (entry: yauzl.Entry) => {
        names.push(entry.fileName);
        zipfile.readEntry();
      });
      zipfile.on("end", () => {
        zipfile.close();
        resolve(names);
      });
      zipfile.on("error", () => {
        zipfile.close();
        resolve(names);
      });
    });
  });
}
