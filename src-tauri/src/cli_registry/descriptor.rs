//! CLI 的静态元数据。
//!
//! 刻意是**纯数据而非 trait**：每个 CLI 一个单元结构体加一份 `impl` 会写出成段
//! 只返回常量的方法体，trait 在此不提供抽象收益，只提供 6 份样板。
//!
//! 本模块只声明类型，不含数据表：每个描述符实例与它所属 CLI 的归属判定、启动能力
//! 写在同一个 `sources/<cli>.rs` 里，元数据不再集中到一张与源并列、需要人工同步的表。

/// 该 CLI 在文件系统上的数据根。
///
/// **库型源同样提供它**：数据根是「CLI 把数据放在哪」，与会话是不是文件无关 ——
/// 库型源的库文件也在自己的数据根下。两者的差别只在**会话子目录**那一半：
/// 文件型源有（会话是子目录下的文件），库型源没有（会话是库里的行），故它是 `Option`。
#[derive(Debug, Clone, Copy)]
pub struct FileRoot {
    /// 相对用户主目录的分段，如 `[".claude"]`、`[".gemini", "antigravity-cli"]`。
    pub data_dir_segments: &'static [&'static str],
    /// 数据根下存放会话的子目录，如 `"projects"`、`"brain"`。单级目录名，不含分隔符。
    /// **库型源没有这个概念**，给 `None`。
    pub sessions_subdir: Option<&'static str>,
}

/// 没有代理能力的 CLI 上报什么端口。
///
/// 端口是**每个 CLI 都要上报的值**（`ProxyStatus.port` 是必填字段，`default_proxy_status`
/// 对任何 CLI 都会读它并返回给前端），所以它不由「有没有代理能力」决定，而是逐 CLI 数据。
/// 这个值本身不指向任何真实监听。
pub(crate) const NO_PROXY_PORT: u16 = 18088;

/// CLI 的静态元数据。不含自身 `id`：取描述符的入口 `descriptor_for(kind)` 已经带着 `kind`，
/// 再存一份 `id` 只会多出一个必须与键保持同步、却没有任何读方的字段。
#[derive(Debug, Clone, Copy)]
pub struct CliDescriptor {
    pub name: &'static str,
    /// 托盘菜单用的短名。与 `name` **不同**：`name` 是完整品牌名（如 "Claude Code"），
    /// 托盘菜单项空间有限，用短名（如 "Claude"）。两者不是重复，别合并。
    pub tray_label: &'static str,
    /// 可执行文件名。可能仅作展示与回退，**不保证可直接执行** ——
    /// 桌面应用型 CLI 的恢复走深链，从不 exec 该字符串。
    pub command: &'static str,
    /// 代理端口。**每个 CLI 都要有**：`ProxyStatus.port` 是必填字段，前端按 CLI 取状态时
    /// 无论该 CLI 支不支持代理都会读到它。没有代理能力的 CLI 用 [`NO_PROXY_PORT`]。
    pub proxy_port: u16,
    pub file_root: FileRoot,
}
