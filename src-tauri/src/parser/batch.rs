//! 流式分批策略 —— 所有解析器共用。
//!
//! 收敛原因：此前 6 个解析器各自实现了一遍"攒够 `batch_size` 就回调"，策略散落各处，
//! 结果是只有 claude 拿到了"首批小、其后大"的自适应批量，其余 provider 仍按固定条数
//! 发事件，大会话因此多出数倍的跨进程事件。分批属于**通道策略**，不该由各 provider 各自决定。
//!
//! 策略取舍：
//! - 首批刻意取小值：首屏只要一小批就能画出来，越早 emit 越早结束骨架屏；
//! - 其后放大批量：开销主要由**事件数量**（每次都要序列化 + 跨进程投递 + 前端反序列化）
//!   而非总字节数决定，批量越大事件越少；
//! - 调用方只负责喂消息与消费回调，不再自己维护计数器与 Vec 容量。
//!
//! 注意：本模块只改变**何时**发车，不改变发什么、也不改变消息顺序。

use crate::session::ChatMessage;

/// 首批条数 —— 首屏尽快出画面
pub(crate) const FIRST_BATCH: usize = 50;
/// 后续批次条数 —— 压低事件数量
pub(crate) const STEADY_BATCH: usize = 200;

/// 分批发射器。典型用法：
///
/// ```ignore
/// let mut emitter = BatchEmitter::new();
/// // 逐条：
/// emitter.push(msg);
/// if !emitter.maybe_flush(&mut on_batch) { return Ok(..); }
/// // 收尾：
/// emitter.flush_remaining(&mut on_batch);
/// ```
pub(crate) struct BatchEmitter {
    steady: usize,
    limit: usize,
    buf: Vec<ChatMessage>,
}

impl BatchEmitter {
    pub(crate) fn new() -> Self {
        Self::with_limits(FIRST_BATCH, STEADY_BATCH)
    }

    /// 自定义阈值（测试用）。`steady` 不小于 `first`，且两者至少为 1，
    /// 否则"攒够即发"的判定会退化成逐条发车或永不发车。
    pub(crate) fn with_limits(first: usize, steady: usize) -> Self {
        let first = first.max(1);
        let steady = steady.max(first);
        Self {
            steady,
            limit: first,
            buf: Vec::with_capacity(first),
        }
    }

    /// 供解析器直接写入的缓冲区。
    ///
    /// 个别解析器（如 Codex）一次 `process_line` 可能产出 0..n 条消息并自行写入 Vec，
    /// 这里暴露缓冲区而不是绕开它们改写生产逻辑。
    pub(crate) fn buffer_mut(&mut self) -> &mut Vec<ChatMessage> {
        &mut self.buf
    }

    pub(crate) fn push(&mut self, msg: ChatMessage) {
        self.buf.push(msg);
    }

    /// 立即强制发车（如首块完成时）。若缓冲区非空，发出后将阈值提升到 `steady`。
    /// 返回 `false` 表示调用方应中止解析。
    pub(crate) fn flush_now<F>(&mut self, on_batch: &mut F) -> bool
    where
        F: FnMut(Vec<ChatMessage>) -> bool,
    {
        if self.buf.is_empty() {
            return true;
        }
        self.limit = self.steady;
        let take = std::mem::replace(&mut self.buf, Vec::with_capacity(self.limit));
        on_batch(take)
    }

    /// 达到当前阈值则发车并把阈值提升到 `steady`。返回 `false` 表示调用方应中止解析。
    pub(crate) fn maybe_flush<F>(&mut self, on_batch: &mut F) -> bool
    where
        F: FnMut(Vec<ChatMessage>) -> bool,
    {
        if self.buf.len() < self.limit {
            return true;
        }
        self.limit = self.steady;
        let take = std::mem::replace(&mut self.buf, Vec::with_capacity(self.limit));
        on_batch(take)
    }

    /// 发出不足一整批的尾批。必须在解析结束后调用，否则末尾消息会丢。
    pub(crate) fn flush_remaining<F>(&mut self, on_batch: &mut F)
    where
        F: FnMut(Vec<ChatMessage>) -> bool,
    {
        if self.buf.is_empty() {
            return;
        }
        let take = std::mem::take(&mut self.buf);
        on_batch(take);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(i: usize) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            timestamp: String::new(),
            model: None,
            token_usage: None,
            content_parts: Vec::new(),
            is_meta: false,
            uuid: Some(format!("u{i}")),
        }
    }

    fn collect(emitter: &mut BatchEmitter, n: usize) -> Vec<Vec<ChatMessage>> {
        let mut out: Vec<Vec<ChatMessage>> = Vec::new();
        let mut on_batch = |b: Vec<ChatMessage>| {
            out.push(b);
            true
        };
        for i in 0..n {
            emitter.push(msg(i));
            if !emitter.maybe_flush(&mut on_batch) {
                break;
            }
        }
        emitter.flush_remaining(&mut on_batch);
        out
    }

    #[test]
    fn first_batch_is_small_then_steady() {
        let mut e = BatchEmitter::with_limits(2, 5);
        let batches = collect(&mut e, 20);
        let sizes: Vec<usize> = batches.iter().map(|b| b.len()).collect();
        assert_eq!(sizes, vec![2, 5, 5, 5, 3]);
    }

    #[test]
    fn tail_is_flushed() {
        let mut e = BatchEmitter::with_limits(10, 10);
        let batches = collect(&mut e, 4);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 4, "不足一批的尾批必须发出");
    }

    #[test]
    fn order_is_preserved() {
        let mut e = BatchEmitter::with_limits(3, 3);
        let flat: Vec<String> = collect(&mut e, 8)
            .into_iter()
            .flatten()
            .map(|m| m.uuid.unwrap())
            .collect();
        let expected: Vec<String> = (0..8).map(|i| format!("u{i}")).collect();
        assert_eq!(flat, expected);
    }

    #[test]
    fn zero_limits_do_not_degenerate() {
        // 阈值为 0 会让"攒够即发"永不成立（或在 len>=0 时逐条发车），必须被夹到至少 1
        let mut e = BatchEmitter::with_limits(0, 0);
        let batches = collect(&mut e, 3);
        assert_eq!(batches.iter().map(|b| b.len()).sum::<usize>(), 3);
    }

    #[test]
    fn abort_stops_further_batches() {
        let mut e = BatchEmitter::with_limits(1, 1);
        let mut seen = 0;
        let mut on_batch = |b: Vec<ChatMessage>| {
            seen += b.len();
            false
        };
        e.push(msg(0));
        assert!(!e.maybe_flush(&mut on_batch), "回调返回 false 时必须能中止");
        e.flush_remaining(&mut on_batch);
        assert_eq!(seen, 1, "中止后不应再收到新批次");
    }
}
