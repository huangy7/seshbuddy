pub mod agent;
pub mod backfill;
pub mod commands;
pub mod conversations;
pub mod quick_phrases;

#[cfg(test)]
mod tests {
    use crate::cli::CliKind;

    /// 每个 CLI 要么有提取格式，要么**显式**列在「暂无提取」名单里。
    ///
    /// 断言集合的**完备性**而不是与一个字面量相等：后者只在「加了格式却没更新字面量」时变红，
    /// 而真正要拦的是「加了 CLI 却没决定它有没有格式」—— 那种情况下集合不变，测试照样绿。
    #[test]
    fn every_cli_declares_whether_it_has_an_extraction_format() {
        let without: Vec<&str> = CliKind::ALL
            .iter()
            .filter(|k| crate::cli_registry::source_for(**k).transcript_format().is_none())
            .map(|k| k.id())
            .collect();
        assert_eq!(
            without,
            vec!["dsh", "antigravity", "opencode", "grok"],
            "「暂无提取格式」的 CLI 集合变了：新 CLI 必须在此显式表态"
        );
    }

    /// 逐个钉住**取值**，不只是「有没有」。
    ///
    /// 上一条断言的是集合完备性，它证明不了「Claude 拿到了 Claude 的格式」——
    /// 把两个源的格式对调，上一条与 `transcript-store` 侧的映射测试**都照样绿**
    /// （各测映射的一侧），而后果是拿错解析器去抽、产出结构对不上的内容。
    #[test]
    fn each_cli_maps_to_its_own_transcript_format() {
        use transcript_store::extract::CliFormat;
        let expected = [
            (CliKind::Claude, Some(CliFormat::Claude)),
            (CliKind::Codex, Some(CliFormat::Codex)),
            (CliKind::Gemini, Some(CliFormat::Gemini)),
            (CliKind::WorkBuddy, Some(CliFormat::WorkBuddy)),
            (CliKind::Dsh, None),
            (CliKind::Antigravity, None),
            (CliKind::Opencode, None),
            (CliKind::Grok, None),
        ];
        for (kind, format) in expected {
            assert_eq!(
                crate::cli_registry::source_for(kind).transcript_format(),
                format,
                "{kind:?} 的提取格式与它自己的解析器对不上"
            );
        }
    }
}
