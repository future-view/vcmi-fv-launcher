//! 通用容量受限缓存：**按字节上界**淘汰（不是按条目数），O(1) 均摊。
//!
//! 背景：构建期各缓存的条目体积差异极大（一个 DEF 数 MB，一个配置条目几十 KB），
//! 按条目数设限无法约束真实占用；而"满了就整体 `clear()`"会造成雪崩
//! （下一批全部重新解析 / 重新读盘）。故统一改为按字节上界淘汰。
//!
//! ⚠️ 淘汰策略是 **FIFO**（淘汰最早插入者）而非严格 LRU：严格 LRU 每次命中都要移动
//! 链表节点，在"每次资源解析都查一次"的热路径上（实测数十万次）代价过大；
//! 而本项目的访问模式是**按模组成簇**（同一个 mod 的资源在短时间内连续被访问），
//! FIFO 的命中率与 LRU 基本等价，但插入/查询都是 O(1) 且无锁内重排。
//!
//! 依赖方向：本层属 `infra`，不依赖 `vcmi` / `modcache` / `tauri`。

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

/// 清空任意一个 `SizedCache`（构建结束释放内存用）。
///
/// 独立成函数的原因：各调用点的 `SizedCache` **类型参数不同**，无法放进同一个数组里循环；
/// 逐个写 `if let Some(c) = X.get()` 又会复制四遍样板。
pub(crate) fn clear_sized<K: Eq + Hash + Clone, V>(cache: Option<&Mutex<SizedCache<K, V>>>) {
    if let Some(c) = cache {
        if let Ok(mut g) = c.lock() {
            g.clear();
        }
    }
}

/// 缓存条目：带插入序号，用于识别"已被覆盖的陈旧队列记录"。
struct Entry<V> {
    size: usize,
    seq: u64,
    v: V,
}

/// 按字节上界的 FIFO 缓存。
///
/// ⚠️ `cap_bytes` 为**软上界**：单条体积可以超过它（此时该条插入后立即成为唯一条目，
/// 下一次插入才会把它淘汰），保证"永远能放下至少一条"，不会因大对象而彻底失效。
pub(crate) struct SizedCache<K, V> {
    map: HashMap<K, Entry<V>>,
    /// (插入序号, 键)：淘汰从队首开始
    order: VecDeque<(u64, K)>,
    bytes: usize,
    cap: usize,
    next_seq: u64,
}

impl<K: Eq + Hash + Clone, V> SizedCache<K, V> {
    pub(crate) fn new(cap_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            cap: cap_bytes,
            next_seq: 0,
        }
    }

    /// 命中返回 `Some(v)`（需要 `V: Clone`；热路径通常存 `Arc`，浅拷贝）。
    pub(crate) fn get(&self, k: &K) -> Option<V>
    where
        V: Clone,
    {
        self.map.get(k).map(|e| e.v.clone())
    }

    /// 插入；超出字节上界时从最早插入者开始淘汰。
    pub(crate) fn insert(&mut self, k: K, size: usize, v: V) {
        if let Some(old) = self.map.remove(&k) {
            self.bytes = self.bytes.saturating_sub(old.size);
            // 队列里的旧记录**不移除**：淘汰时按 seq 判定为陈旧并跳过（保持 O(1)）
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.map.insert(k.clone(), Entry { size, seq, v });
        self.order.push_back((seq, k));
        self.bytes = self.bytes.saturating_add(size);
        self.evict_if_needed();
    }

    fn evict_if_needed(&mut self) {
        while self.bytes > self.cap {
            match self.order.pop_front() {
                Some((seq, k)) => {
                    // 陈旧记录（该键已被重新插入）：跳过，不重复扣减
                    let stale = self.map.get(&k).map(|e| e.seq != seq).unwrap_or(true);
                    if stale {
                        continue;
                    }
                    if let Some(e) = self.map.remove(&k) {
                        self.bytes = self.bytes.saturating_sub(e.size);
                    }
                }
                None => break,
            }
        }
    }

    /// 清空（模组集合变化 / 构建结束释放内存）。
    ///
    /// ⚠️ 清空后**同时收缩底层容器**：只 `clear()` 时 `HashMap` / `VecDeque` 的已分配
    /// 桶数组与环形缓冲仍挂在缓存对象上（进程级 `OnceLock` 永不被 drop），内存看不出下降
    /// —— 这正是"构建完内存不释放"的一部分。收缩后再次使用会重新分配，属于可接受的一次性代价。
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.map.shrink_to_fit();
        self.order.clear();
        self.order.shrink_to_fit();
        self.bytes = 0;
    }

    #[allow(dead_code)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
