//! 会话源的登记表。
//!
//! 接入一个 CLI 要动三处：`cli_kinds!` 加变体、`cli.rs` 的 `id()` 加臂、本文件的
//! `source_for` 加臂。前两处是穷尽 `match`，第三处由 `source_for` 的穷尽 `match` 兜住，
//! 漏任何一处都编译失败 —— 因此这里不再需要「变体 ↔ 槽位」的编译期哨兵，
//! 也不再需要与源并列的第二张描述符表：描述符由源自己携带。

use crate::cli::CliKind;

use super::descriptor::CliDescriptor;
use super::source::CliSource;
use super::sources::{
    AntigravitySource, ClaudeSource, CodexSource, CursorSource, DshSource, GeminiSource,
    OpencodeSource, WorkBuddySource,
};

/// 按枚举取源。穷尽 `match`：新增变体而不在此登记，编译不过。
pub(crate) fn source_for(kind: CliKind) -> &'static dyn CliSource {
    match kind {
        CliKind::Claude => &ClaudeSource,
        CliKind::Codex => &CodexSource,
        CliKind::Gemini => &GeminiSource,
        CliKind::WorkBuddy => &WorkBuddySource,
        CliKind::Dsh => &DshSource,
        CliKind::Antigravity => &AntigravitySource,
        CliKind::Opencode => &OpencodeSource,
        CliKind::Cursor => &CursorSource,
    }
}

pub(crate) fn descriptor_for(kind: CliKind) -> &'static CliDescriptor {
    source_for(kind).descriptor()
}

/// 未命中任何源时落到这个 CLI：认不出数据目录的定位符此前一律按 Claude 处理，
/// 本兜底保持该判定不变，只是改为在「无人认领」时由注册表施加，
/// 而不是让某个源在自己的判据里也返回 true。
pub(crate) const FALLBACK: CliKind = CliKind::Claude;

/// 按序列化定位符反查归属。次序即声明次序；未命中落到 `FALLBACK`。
pub(crate) fn kind_for_path(key: &str) -> CliKind {
    CliKind::ALL
        .iter()
        .copied()
        .find(|kind| source_for(*kind).owns(key))
        .unwrap_or(FALLBACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 夹具路径覆盖全部 6 个 CLI 与两个平台的路径分隔符。
    fn fixture_paths() -> Vec<(&'static str, CliKind)> {
        vec![
            ("/Users/x/.claude/projects/a/b.jsonl", CliKind::Claude),
            ("/Users/x/.codex/sessions/a.jsonl", CliKind::Codex),
            ("/Users/x/.gemini/tmp/a.jsonl", CliKind::Gemini),
            ("/Users/x/.workbuddy/projects/p/a.jsonl", CliKind::WorkBuddy),
            ("/Users/x/.dsh/sessions/a/session.jsonl", CliKind::Dsh),
            ("/Users/x/.gemini/antigravity-cli/brain/s1/transcript.jsonl", CliKind::Antigravity),
            // 库型源：会话身份是 `cli://opencode/<id>` 虚拟键，不是文件路径。
            ("cli://opencode/ses_x", CliKind::Opencode),
            ("/Users/x/.cursor/projects/p/agent-transcripts/s/s.jsonl", CliKind::Cursor),
            // 边界案例：`antigravity-cli-tools` 只是名字里带了同一串字符，
            // 并不是 Antigravity 的数据目录。若排除方按裸子串否决，这条路径会
            // 「无人认领」而静默落到兜底项 —— 归属必须仍留在 Gemini。
            ("/Users/x/.gemini/tmp/antigravity-cli-tools/chats/s1.jsonl", CliKind::Gemini),
            (r"C:\Users\x\.codex\sessions\a.jsonl", CliKind::Codex),
            (r"C:\Users\x\.gemini\antigravity-cli\brain\s1\transcript.jsonl", CliKind::Antigravity),
            (r"C:\Users\x\.cursor\projects\p\agent-transcripts\s\s.jsonl", CliKind::Cursor),
        ]
    }

    /// 归属判定必须**两两互斥**：每个夹具路径恰好一个源认领。
    /// 零个认领说明有路径会静默落到兜底项；多于一个说明次序承载了语义。
    /// 这比「兜底项有且只有一个」强一个量级 —— 后者只能发现兜底项数量的错误。
    #[test]
    fn owns_is_mutually_exclusive() {
        for (path, _) in fixture_paths() {
            let claimants: Vec<&str> = CliKind::ALL
                .iter()
                .filter(|k| source_for(**k).owns(path))
                .map(|k| k.id())
                .collect();
            assert_eq!(claimants.len(), 1, "路径 {path} 被 {claimants:?} 认领，应当恰好一个");
        }
    }

    /// 路由结果必须与旧实现的逐条判定一致。
    #[test]
    fn kind_for_path_matches_legacy_routing() {
        for (path, expected) in fixture_paths() {
            assert_eq!(kind_for_path(path), expected, "路径 {path} 路由结果变化");
        }
    }

    /// 不认识任何标记的路径落到 `FALLBACK`。
    #[test]
    fn unknown_key_falls_back() {
        assert_eq!(kind_for_path("/tmp/whatever.jsonl"), FALLBACK);
    }

    /// 每个 CLI 都要在路径归属夹具里出现，否则新 CLI 的 `owns()` 写错时无人发现 ——
    /// 它的路径会被 `FALLBACK` 静默判给另一个 CLI。
    #[test]
    fn every_cli_appears_in_the_path_ownership_fixtures() {
        // `fixture_paths()` 的形态是 `Vec<(&'static str, CliKind)>`（路径, 归属的 CLI）。
        let covered: Vec<CliKind> = fixture_paths().iter().map(|(_, kind)| *kind).collect();
        for kind in CliKind::ALL {
            assert!(
                covered.contains(kind),
                "{kind:?} 不在路径归属夹具里：它的 owns() 写错时会被静默判给别的 CLI"
            );
        }
    }

    /// 描述符完整性：文件型源的数据根逐段非空、`sessions_subdir` 不含分隔符、
    /// `name` / `command` 非空。
    ///
    /// 空分段必须单独拦下：`[""]` 长度非零，只断言「分段非空」会放过它，
    /// 而空段 join 之后数据根等于用户主目录 —— 整个主目录被当成某个 CLI 的会话根。
    #[test]
    fn descriptors_are_well_formed() {
        for kind in CliKind::ALL {
            let descriptor = descriptor_for(*kind);
            assert!(!descriptor.name.is_empty(), "{kind:?} 的展示名为空");
            assert!(!descriptor.command.is_empty(), "{kind:?} 的命令名为空");
            let root = descriptor.file_root;
            assert!(!root.data_dir_segments.is_empty(), "{kind:?} 的数据根分段为空");
            for segment in root.data_dir_segments {
                assert!(!segment.is_empty(), "{kind:?} 的数据根含空分段: {:?}", root.data_dir_segments);
            }
            // 库型源没有会话子目录（`None`），此项对它不适用；有子目录的文件型源才校验。
            if let Some(subdir) = root.sessions_subdir {
                assert!(
                    !subdir.contains('/') && !subdir.contains('\\'),
                    "{kind:?} 的 sessions_subdir 含分隔符，应为单级目录名"
                );
            }
        }
    }
}
