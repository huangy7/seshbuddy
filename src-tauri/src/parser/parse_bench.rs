//! 大会话解析性能探针。
//!
//! 默认 `#[ignore]`，手动指定样本文件后运行：
//!
//! ```sh
//! SESHBUDDY_BENCH_SESSION=/path/to/big.jsonl \
//!   cargo test --lib parse_bench -- --ignored --nocapture
//! ```
//!
//! 用途：把"打开一个大会话到底慢在哪"拆成 **I/O+解码 / JSON 解析 / 业务装配** 三段，
//! 避免凭直觉优化。样本通常是真实会话 JSONL（单行可达数 MB）。
//!
//! 注意：本探针只读文件、不写任何用户数据，因此可以安全地在任何样本上跑。

use std::io::{BufRead, BufReader};
use std::time::Instant;

fn sample_path() -> Option<String> {
    match std::env::var("SESHBUDDY_BENCH_SESSION") {
        Ok(p) if !p.trim().is_empty() => Some(p),
        _ => None,
    }
}

/// 只做 I/O + UTF-8 解码 + 行切分，不解析 JSON。用于给出"解析之外"的地板耗时。
fn read_only(path: &str) -> (usize, usize, u128) {
    let file = std::fs::File::open(path).expect("open sample");
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = String::new();
    let mut lines = 0usize;
    let mut bytes = 0usize;
    let started = Instant::now();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                lines += 1;
                bytes += n;
            }
            Err(e) => panic!("read_line failed: {e}"),
        }
    }
    (lines, bytes, started.elapsed().as_millis())
}

/// 反序列化成通用 `serde_json::Value` 树但不装配。
/// 与 [2] 的差值即为"装配成 ChatMessage"的净开销 —— 用来判断该优化
/// JSON 反序列化本身，还是优化 `entry.get(...)` 那一串字段提取。
fn value_parse_only(path: &str) -> (usize, u128) {
    let file = std::fs::File::open(path).expect("open sample");
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = String::new();
    let mut ok = 0usize;
    let started = Instant::now();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => panic!("read_line failed: {e}"),
        }
        if serde_json::from_str::<serde_json::Value>(&buf).is_ok() {
            ok += 1;
        }
    }
    (ok, started.elapsed().as_millis())
}

/// 全量解析（`serde_json::Value` 树 + `parse_content`），与 [2] 同口径。
/// 仅存在于测试构建中，用来给「类型化 + 边解析边截断」提供同批次对照。
fn legacy_full_parse(path: &str) -> (usize, u128) {
    let file = std::fs::File::open(path).expect("open sample");
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let session_path = std::path::Path::new(path);
    let mut subagent_map = std::collections::HashMap::new();
    let mut buf = String::new();
    let mut messages = Vec::new();

    let started = Instant::now();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !super::claude::chat_line_candidate(line) {
            continue;
        }
        if let Some(msg) = super::claude::parse_jsonl_line_legacy(
            line,
            false,
            Some(&mut subagent_map),
            Some(session_path),
        ) {
            messages.push(msg);
        }
    }
    (messages.len(), started.elapsed().as_millis())
}

/// 只做「类型化捕获」：解析条目但不碰 content 正文。
/// 用来把开销拆成「RawValue 捕获」与「正文解析」两段，便于定位嵌套 `RawValue`
/// 重复扫描子树的代价。
fn typed_capture_only(path: &str) -> (usize, u128) {
    let file = std::fs::File::open(path).expect("open sample");
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = String::new();
    let mut captured = 0usize;
    let started = Instant::now();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() || !super::claude::chat_line_candidate(line) {
            continue;
        }
        if let Some(entry) = super::claude_entry::ClaudeEntry::parse(line) {
            if entry.message().and_then(|m| m.content()).is_some() {
                captured += 1;
            }
        }
    }
    (captured, started.elapsed().as_millis())
}

/// 类型化捕获 + 完整正文解析（不构造 ChatMessage 的其余字段）。与 [2] 的差值即 ChatMessage 装配。
fn typed_capture_and_content(path: &str) -> (usize, usize, u128) {
    let file = std::fs::File::open(path).expect("open sample");
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = String::new();
    let mut captured = 0usize;
    let mut parts = 0usize;
    let started = Instant::now();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() || !super::claude::chat_line_candidate(line) {
            continue;
        }
        if let Some(entry) = super::claude_entry::ClaudeEntry::parse(line) {
            if let Some(content) = entry.message().and_then(|m| m.content()) {
                captured += 1;
                parts += super::claude_entry::content_parts(content).len();
            }
        }
    }
    (captured, parts, started.elapsed().as_millis())
}

#[test]
#[ignore = "手动运行：需设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>"]
fn parse_bench() {
    let Some(path) = sample_path() else {
        eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
        return;
    };

    let size = std::fs::metadata(&path).expect("stat sample").len();
    eprintln!("样本: {path}");
    eprintln!("大小: {:.1} MB", size as f64 / 1e6);
    eprintln!();

    let (lines, bytes, read_ms) = read_only(&path);
    eprintln!(
        "[1] 顺序读 + UTF-8 解码（复用缓冲）: {read_ms:>6} ms   {lines} 行 / {:.1} MB",
        bytes as f64 / 1e6
    );

    let started = Instant::now();
    let full = super::claude::parse_session_file_with_offset(&path, false)
        .expect("full parse");
    let full_ms = started.elapsed().as_millis();
    eprintln!(
        "[2] 全量解析（含 [1] + JSON + 装配）: {full_ms:>6} ms   {} 条消息",
        full.messages.len()
    );

    let (value_lines, value_ms) = value_parse_only(&path);
    eprintln!(
        "[3] 仅 Value 反序列化（不装配）:      {value_ms:>6} ms   {value_lines} 行"
    );

    let started = Instant::now();
    let mut batches = 0usize;
    let mut streamed = 0usize;
    super::parse_session_file_streaming(&path, false, |batch| {
        batches += 1;
        streamed += batch.len();
        true
    })
    .expect("streaming parse");
    let stream_ms = started.elapsed().as_millis();
    eprintln!(
        "[4] 流式解析（分批 emit）:           {stream_ms:>6} ms   {streamed} 条 / {batches} 批"
    );

    let (legacy_count, legacy_ms) = legacy_full_parse(&path);
    eprintln!(
        "[5] 改造前全量解析（Value 树，对照）: {legacy_ms:>6} ms   {legacy_count} 条"
    );

    let (captured, capture_ms) = typed_capture_only(&path);
    eprintln!(
        "[6] 类型化捕获（不解析正文）:        {capture_ms:>6} ms   {captured} 条"
    );

    let (captured2, parts, content_ms) = typed_capture_and_content(&path);
    eprintln!(
        "[7] 类型化捕获 + 正文解析:           {content_ms:>6} ms   {captured2} 条 / {parts} 个 part"
    );
    eprintln!();

    // 注意：[2]/[4] 在 >=16MB 时走并行内核，而 [6]/[7] 是单线程类型化阶段，
    // 「装配 = [2]-[7]」在并行后会失真（可为负）。仅在 [2] 未并行的场景下输出拆分。
    if capture_ms > 0 && content_ms > 0 && full_ms >= content_ms {
        eprintln!(
            "新路径拆分（单线程口径）: 捕获 {capture_ms} ms | 正文解析 {} ms | 装配 {} ms",
            content_ms as i128 - capture_ms as i128,
            full_ms as i128 - content_ms as i128,
        );
    } else if full_ms < content_ms {
        eprintln!(
            "新路径拆分: 不适用 —— [2] 已走 P1 并行（{full_ms} ms < 单线程捕获+正文 {content_ms} ms），I/O 与解析跨线程重叠"
        );
    }
    eprintln!();

    if full_ms > 0 && full_ms >= value_ms {
        let io = read_ms as f64;
        let json = value_ms as f64 - io;
        let assemble = full_ms as f64 - value_ms as f64;
        eprintln!(
            "拆分: I/O+解码 {io:.0} ms ({:.0}%) | Value 反序列化 {json:.0} ms ({:.0}%) | 装配 {assemble:.0} ms ({:.0}%)",
            io / full_ms as f64 * 100.0,
            json / full_ms as f64 * 100.0,
            assemble / full_ms as f64 * 100.0,
        );
    } else if full_ms > 0 {
        eprintln!(
            "拆分: 不适用 —— [2] 已走 P1 并行（{full_ms} ms < 单线程 Value 全程 {value_ms} ms），单线程各阶段占比对并行路径无意义"
        );
    }
    if full_ms > 0 && legacy_ms > 0 {
        eprintln!(
            "P0+P1 提速: {legacy_ms} ms → {full_ms} ms = {:.2}x（同口径：全量解析 + 装配）",
            legacy_ms as f64 / full_ms as f64
        );
    }
}
