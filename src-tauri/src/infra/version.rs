//! 版本比较：模组与领域**共用的唯一口径**（前端镜像见 `src/lib/modRepo.ts`，
//! 仓库索引脚本见 `scripts/build-realms-index.mjs`）。
//!
//! 规则：去 `v`/`V` 前缀 → 按 `.` 分段 → 每段取**前导数字**
//! （`"2b"`→2、`"12a"`→12、`"rc1"`→0、`"0-rc1"`→0）→ 逐段比较，缺段补 0。
//!
//! 设计取舍（宁缺勿错）：
//! - 后缀（`-rc1` / `-beta` 等）不参与比较：`1.0.0-rc1` 与 `1.0.0` 视为同版 ——
//!   预发布与正式版谁新谁旧在字符串层面无法可靠判定，宁可漏报也不误报降级 / 更新；
//! - 不可解析的段按 0 计、绝不 panic：版本串由用户手填，宽容优先。

use std::cmp::Ordering;

/// 点分版本逐段比较，返回 `a` 相对 `b` 的 `Ordering`。
pub fn compare(a: &str, b: &str) -> Ordering {
    let seg = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| {
                let digits: String = p
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                digits.parse::<u64>().unwrap_or(0)
            })
            .collect()
    };
    let (x, y) = (seg(a), seg(b));
    for i in 0..x.len().max(y.len()) {
        let l = *x.get(i).unwrap_or(&0);
        let r = *y.get(i).unwrap_or(&0);
        if l != r {
            return if l < r {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::compare;
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn compares_dotted_numeric_segments() {
        assert_eq!(compare("1.7.3", "1.7.4"), Less);
        assert_eq!(compare("1.7", "1.7.1"), Less);
        assert_eq!(compare("1.9", "2.0"), Less);
        assert_eq!(compare("1.8", "1.7"), Greater);
        assert_eq!(compare("1.7.4", "1.7.4"), Equal);
        // 缺段补 0：1.0 与 1.0.0 同版
        assert_eq!(compare("1.0", "1.0.0"), Equal);
    }

    #[test]
    fn ignores_v_prefix_and_reads_leading_digits() {
        assert_eq!(compare("v1.2.0", "1.2.0"), Equal);
        assert_eq!(compare("V2.0", "v1.9"), Greater);
        // 数字后紧贴字母的段仍取数字（旧口径会把整段当 0）
        assert_eq!(compare("1.2.3beta", "1.2.3"), Equal);
        assert_eq!(compare("1.2.3beta", "1.2.4"), Less);
        // 非数字开头的段按 0
        assert_eq!(compare("abc", "0"), Equal);
    }

    #[test]
    fn suffixes_do_not_take_part_in_comparison() {
        // 尾缀段数字为 0 / 无数字 → 与正式版视为同版（见模块注释：宁可漏报）
        assert_eq!(compare("1.0.0-rc1", "1.0.0"), Equal);
        assert_eq!(compare("1.0.0-rc1", "1.0.0-rc2"), Equal);
        // 但点分后缀中的纯数字段仍参与比较（beta.2 比 beta.1 新）
        assert_eq!(compare("1.0.0-beta.2", "1.0.0-beta.1"), Greater);
    }
}
