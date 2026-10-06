//! 会话源契约。
//!
//! 归属判定刻意收 `&str`：反查（给一个路径串问「这是谁的」）发生在拿到 `cli_id` 之前，
//! 若收带 `cli_id` 的定位符，「这个会话归我吗」会退化成 `loc.cli_id() == self.id()` 而恒真。
//!
//! **已知且暂不消解的依赖方向**：本文件所属的 `cli_registry` 依赖 `parser::<cli>` 与
//! `parser::shared`（各源的解析实现落在 `sources/<cli>.rs`），而 `parser/mod.rs` 又依赖
//! `cli_registry`（分派层靠 `kind_for_path` / `source_for` 找源）。于是两模块互相依赖。
//! 这是重构中间态的取舍，不是终局：读族搬完后，解析模块整体应挪到各源之下，依赖收成单向。
//! 这一步是独立且更大的改动（要同时挪动解析实现与它们的调用方），故本期不在此处顺手做，
//! 只把方向记下来，避免后人在读到这个环时误以为是疏漏。Rust 允许模块成环，编译不受影响。
//!
//! **同一取向的第二处**：`snapshot` 让各源依赖 `commands::session_index` 的装配骨架。
//! 快照骨架与源同属「这个 CLI 怎么装配列表」的知识，收在源侧才不必回分派层补臂；
//! 它与解析依赖一样是本期刻意接受的中间态，不是终局。

use super::descriptor::CliDescriptor;
use super::features::LaunchFeature;
use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::AggregatedSessionSearch;
use crate::commands::session_index::RawSession;
use crate::error::AppResult;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 虚拟定位符的 scheme。文件路径永远不以它开头（Windows 盘符是 `X:` 而非 `cli://`），
/// 故用它区分「库型会话」与「文件型会话」不会与任何真实路径冲突。
const VIRTUAL_SCHEME: &str = "cli://";

/// 会话身份。取代「文件路径」，全链路（列表索引 / 检索 / 自定义名 / 归档 / 墓碑）统一使用。
///
/// 存在的意义是让「这个会话没有文件路径」不再退化成一个裸路径：库型会话（数据在
/// 数据库里，以 `cli://<id>/<key>` 为身份）若被当成普通路径去做 `Path::exists()`，
/// 会恒为 `false`，而启动期孤儿清理把 `false` 读作「会话已消失」并**物理删除**其
/// 索引行与检索文档 —— 用户的会话在下次启动时消失。把「有没有文件路径」收进枚举，
/// 「对虚拟会话做文件系统判定」就不再写得出来。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SessionLocator {
    /// 文件型源：一个会话 = 一个文件。
    File { cli_id: CliKind, path: PathBuf },
    /// 库型源：`cli://<id>/<key>`，非文件系统身份。`key` 是剥掉 scheme 与 id 后的裸键。
    Virtual { cli_id: CliKind, key: String },
}

impl SessionLocator {
    /// 唯一的文件系统出口。虚拟定位符一律返回 `None`，
    /// 使「对虚拟会话做文件系统判定」在类型层面无从表达。
    pub(crate) fn as_file_path(&self) -> Option<&Path> {
        match self {
            Self::File { path, .. } => Some(path),
            Self::Virtual { .. } => None,
        }
    }

    /// 持久化层使用的字符串。`File` 逐字节等于原路径串，存量索引无需重建；
    /// `Virtual` 为 `cli://<id>/<key>`。
    pub(crate) fn to_key(&self) -> String {
        match self {
            Self::File { path, .. } => path.to_string_lossy().into_owned(),
            Self::Virtual { cli_id, key } => format!("{VIRTUAL_SCHEME}{}/{}", cli_id.id(), key),
        }
    }

    /// 从持久化字符串解码。`cli_id` 无法从文件路径反推，由调用方给出 ——
    /// 持久化层的主键本就带 `cli_id`（列表索引、检索状态、墓碑都是复合键）。
    pub(crate) fn decode(cli_id: CliKind, key: &str) -> Self {
        match key.strip_prefix(VIRTUAL_SCHEME) {
            Some(rest) => {
                // 串里自带 id，但 id 的权威来源是持久化层主键（调用方传入的 cli_id）。
                // 剥掉 `cli://<id>/` 前缀只留裸键，to_key 与 cli_id 才永远自洽。
                let bare = rest
                    .strip_prefix(cli_id.id())
                    .and_then(|r| r.strip_prefix('/'))
                    .unwrap_or(rest);
                Self::Virtual { cli_id, key: bare.to_string() }
            }
            None => Self::File { cli_id, path: PathBuf::from(key) },
        }
    }

    pub(crate) fn cli_id(&self) -> CliKind {
        match self {
            Self::File { cli_id, .. } | Self::Virtual { cli_id, .. } => *cli_id,
        }
    }
}

/// 文件型源共用的存在性判定。
///
/// 虚拟定位符不属于文件型源 —— 那属于库型源。走到 `None` 说明有调用方把虚拟会话
/// 交给了文件型源，是契约违例；此时取「存在」并在 debug 下断言。
///
/// **失败方向是承重的，不可反转。** `false` 会被启动期孤儿清理读作「会话已消失」，
/// 据此**物理删除**索引行与检索文档；一旦把仍有数据的会话判成不存在，删除不可恢复。
/// 反过来漏删一行陈旧索引，只是留下一条下次启动还能再清的垃圾。两者代价不对称，
/// 所以宁可漏删，不可误删。
pub(super) fn file_backed_exists(loc: &SessionLocator) -> bool {
    match loc.as_file_path() {
        Some(path) => path.exists(),
        None => {
            debug_assert!(false, "file-backed source received a virtual locator: {loc:?}");
            true
        }
    }
}

/// 一次枚举的产出。
pub(crate) struct Discovered {
    /// 本 CLI 的全部会话。文件型源遍历数据目录，库型源查询数据库。
    pub(crate) locators: Vec<SessionLocator>,
    /// 读取失败的子树。逐层遍历的源据此保留这些子树下的既有索引行（不按「已删除」处理）；
    /// 递归收集的源读取失败会冒泡中止整轮扫描，故给空。
    pub(crate) unverified_prefixes: Vec<PathBuf>,
}

/// 读取会话子目录；失败时把该目录记为「本轮未验证」并跳过，而不是当成"里面什么都没有"。
///
/// 读不到和空目录对陈旧行判定意味着完全相反的事：空目录说明会话确实没了、该清理；读不到
/// 只是这一轮看不见，按"已删除"推断会把该子树下的记录整批清掉（权限变化、外置盘未挂载、
/// I/O 压力都会命中）。根目录读取失败仍然中止整轮扫描（数据源整体不可用），子目录级失败
/// 只跳过该子树，其余项目照常更新。
///
/// 收在此处而非扫描层：它服务的正是 `discover` 的「未验证子树」产出，
/// 逐层遍历的文件型源共用同一份记录口径，才不会各写一份而分叉。
pub(super) fn read_dir_or_record_unverified(
    dir: &Path,
    unverified: &mut Vec<PathBuf>,
) -> Option<fs::ReadDir> {
    match fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(err) => {
            tracing::warn!(
                "会话子目录读取失败，本轮跳过该子树（其下已有记录保留、不按已删除处理）: dir={:?}, err={}",
                dir,
                err
            );
            unverified.push(dir.to_path_buf());
            None
        }
    }
}

/// 递归收集目录树下的全部 `.jsonl` 文件，保持 `read_dir` 的遍历次序。
///
/// 读取失败会冒泡中止整轮扫描：递归收集的源（codex / antigravity）无法像逐层遍历那样
/// 把失败子树精确记入「未验证」，宁可整轮失败让调用方重试，也不静默丢一个子树。
pub(super) fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文件定位符的 `to_key()` 必须与今天的 `session_path` 串逐字节相同 ——
    /// 差一个字节，存量索引就会与新写入的行错位。
    #[test]
    fn file_locator_key_is_byte_identical_to_path_string() {
        let raw = "/Users/x/.claude/projects/a/b.jsonl";
        let loc = SessionLocator::decode(CliKind::Claude, raw);
        assert_eq!(loc, SessionLocator::File { cli_id: CliKind::Claude, path: PathBuf::from(raw) });
        assert_eq!(loc.to_key(), raw);
        assert_eq!(loc.as_file_path(), Some(Path::new(raw)));
        assert_eq!(loc.cli_id(), CliKind::Claude);
    }

    /// 虚拟定位符：`as_file_path()` 为 `None`（「对虚拟会话做文件系统判定」无从表达），
    /// 且 `to_key()` 能还原成 `cli://<id>/<key>`。
    #[test]
    fn virtual_locator_has_no_file_path_and_round_trips() {
        let raw = "cli://claude/ses_abc";
        let loc = SessionLocator::decode(CliKind::Claude, raw);
        assert_eq!(
            loc,
            SessionLocator::Virtual { cli_id: CliKind::Claude, key: "ses_abc".to_string() }
        );
        assert_eq!(loc.as_file_path(), None);
        assert_eq!(loc.to_key(), raw);
        assert_eq!(loc.cli_id(), CliKind::Claude);
    }
}

/// 输入是否**可能被截断**。
///
/// 完整读取时内层帧不完整说明数据损坏，原样返回上一层结果更安全；
/// 按上限截断的头部读取时帧**必然**不完整，能解多少要多少。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Truncation {
    /// 调用方读到了完整内容。
    Complete,
    /// 调用方按上限截断了流（例如只读前若干 KiB 做身份识别）。
    Truncated,
}

pub(crate) trait CliSource: Send + Sync {
    /// 本 CLI 的静态元数据。描述符与归属判定、启动计划同处一个源文件，
    /// 元数据不再集中到与源并列、需要人工同步的第二张表。
    fn descriptor(&self) -> &'static CliDescriptor;

    /// `key` 是定位符的序列化形式。互斥性由各源自己保证，不依赖注册次序。
    ///
    /// 只声明本 CLI 真正拥有的路径，兜底由注册表在「无人认领」时施加：兜底项若在此返回 true，
    /// 会吞掉别人已认领的路径。要关闭的风险也不止「两个源都认领」：**零个源认领**同样破坏
    /// 互斥性，而且更隐蔽 —— 某源按自己的判据否决一条路径、真正该认领它的源又没认领时，
    /// 注册表只能把它落到兜底项，表现为会话静默换了 CLI，全程不报错。因此「否决」的范围
    /// 必须与对方「认领」的范围逐字对应，文件型源统一经 `under_data_root` 判定即可满足这一点。
    fn owns(&self, key: &str) -> bool;

    /// 会话是否仍存在，供启动期孤儿清理使用。
    ///
    /// 刻意不给默认实现：默认体要么复制文件型语义（库型源会被误判为「不存在」而遭删除），
    /// 要么给个 `true`（文件型源的孤儿永远清不掉）。「我按什么判存在」必须每个源写下来。
    /// 文件型源统一经 [`file_backed_exists`] 实现。
    fn exists(&self, loc: &SessionLocator) -> bool;

    /// 本 CLI 的会话能否删除。
    ///
    /// 文件型源的会话是一个文件，删除即把文件移入回收站，故为 `true`；库型源的会话是
    /// 数据库里的行，删除要改库（本源尚未提供），故为 `false` —— 它没有可移入回收站的
    /// 文件，按文件路径删除只会恒假跳过、静默报成功。
    ///
    /// **刻意不给默认实现**：默认体要么把「库型源也能删」的假象写死（UI 报成功、库里
    /// 的行仍在且被墓碑隐藏），要么反过来让文件型源不可删。「我按什么删」必须每个源写下来。
    fn can_delete(&self) -> bool;

    /// 枚举本 CLI 的全部会话。
    ///
    /// 返回定位符而非路径：库型源没有文件路径，用路径会让它在类型层面接不进来。
    /// 刻意不给默认实现：遍历形态（逐层下降还是递归收集、读取失败如何处置）正是
    /// 「这个 CLI 的会话散落在哪里」的知识，默认体要么漏掉某个 CLI 的差异而不自知，
    /// 要么把某个 CLI 的形态强加给所有人。
    fn discover(&self) -> AppResult<Discovered>;

    /// 装配会话列表的分页项目快照：读索引、逐记录投影、分组分页。
    ///
    /// 骨架（读页、跳过子代理、分组、分页、展示名解析链）由
    /// [`crate::commands::session_index::build_projects_snapshot`] 统一提供，本方法只负责
    /// 本 CLI 的预循环准备与该 CLI 的逐记录投影 —— 新增一个 CLI 时，扫描/快照层不必补臂。
    /// 刻意不给默认实现：投影正是「这个 CLI 的会话长什么样」的知识，默认体要么漏掉某个
    /// CLI 的差异而不自知，要么把某个 CLI 的差异强加给所有人。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>>;

    /// 扫描并装配本 CLI 的会话列表。
    ///
    /// 与 [`Self::snapshot`] 的分工：`snapshot` 只读索引、不碰盘，索引缺席时给 `None`；
    /// 本方法从数据源重建列表，是索引缺席时的兜底。`force` 表示忽略缓存、强制全量重解析。
    ///
    /// 收在源上而非留在分派层：分派层若按变体 `match`，每接一个新 CLI 都要回到那里补臂，
    /// 而「这个 CLI 从哪里扫、怎么装配」正是源自己的知识。
    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>>;

    // ===== 会话读取能力：入参同样是序列化定位符 `key`（当前恒为文件路径） =====
    //
    // 读取能力收在源上，分派层只做 `kind_for_path` → `source_for` 两步转发。
    // 每个 CLI 的解析细节留在各自的源文件里，接入一个新 CLI 不必回到分派层补臂 ——
    // 那些臂承载的正是「这个 CLI 怎么读」的知识，与源同处一文件才不会两处分叉。
    //
    // 一律不给默认实现：默认体让「我忘了实现」与「这个 CLI 没有这种读法」
    // 在类型上无法区分，而后者本就该被显式写出来。
    /// 解析整份会话为消息列表。
    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>>;

    /// 首条用户消息，供列表标题兜底；取不到时 `None`，由调用方决定回退。
    fn first_user_message(&self, key: &str) -> Option<String>;

    /// 会话标识；取不到时 `None`。调用方据此判空并决定回退，本方法不代做兜底。
    fn session_id(&self, key: &str) -> Option<String>;

    /// 未经空白过滤的原始项目路径；出口的「空白不是路径」收口在转发层。
    fn project_path_raw(&self, key: &str) -> Option<String>;

    /// 只读元数据扫描：不解析整份消息体，列表页据此避免全量读盘。
    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata>;

    /// 是否子代理会话。父目录名为 `subagents` 的通用判据在转发层先行，
    /// 本方法只负责各 CLI 自己的形态判定。
    fn is_subagent(&self, key: &str) -> bool;

    /// 流式解析：每凑够一批消息就回调一次，返回末位偏移与子代理表。
    ///
    /// 回调收 `&mut dyn FnMut` 而非泛型：trait 方法要能被 `dyn CliSource` 调用，
    /// 泛型参数会破坏对象安全。`false` 表示调用方要求中止（当前无使用者，留作取消支持）。
    fn parse_streaming(
        &self,
        key: &str,
        skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)>;

    /// 从 `offset` 起增量解析；各 CLI 的「水位」坐标系不同（字节或行号），
    /// 由返回值原样带回，调用方下次照传即可，不必知道单位。
    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult>;

    /// 从内存内容解析（归档兜底用）。内容本身已确定格式，故不需要 key。
    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage>;

    /// 扫描检索文档并报告读取进度。进度回调同样收 `&mut dyn FnMut`：理由同 `parse_streaming`。
    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>>;

    /// 从内存字节扫描检索文档。内容已确定格式，故不需要 key。
    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>>;

    // ===== 可选能力：`None` = 对外声明「本 CLI 没有这个能力」 =====
    //
    // 这两个访问器**刻意不给默认实现**：默认返回 `None` 会让「我忘了实现」与
    // 「我不支持」在类型上无法区分，而能力位正是靠「有无实现」来对账的。
    /// 新建会话能力。`None` 时该 CLI 不能开新会话。
    fn new_session(&self) -> Option<&'static dyn LaunchFeature>;
    /// 恢复会话能力。`None` 时该 CLI 不能恢复会话 —— 执行侧据此报错而不是拼出空参数，
    /// 空参数会让终端里启动一个不带 `--resume` 的裸 CLI，"恢复"静默变成"新开"。
    fn resume_session(&self) -> Option<&'static dyn LaunchFeature>;

    /// 该 CLI 是否有可托管的配置文件。
    ///
    /// **不给默认实现**：默认 `None` 会让「忘了实现」变成合法的「不支持」。
    fn config_file(&self) -> Option<&'static dyn crate::cli_registry::features::ConfigFileFeature>;

    /// 该 CLI 是否支持 API 代理。
    ///
    /// **不给默认实现**：默认 `None` 会让「忘了实现」变成合法的「不支持」。
    fn api_proxy(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProxyFeature>;

    /// 该 CLI 是否支持配置档托管。
    ///
    /// **不给默认实现**：默认 `None` 会让「忘了实现」变成合法的「不支持」。
    fn api_profile(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProfileFeature>;

    /// 该 CLI 是否支持分叉会话。
    ///
    /// **不给默认实现**：默认 `None` 会让「忘了实现」变成合法的「不支持」。
    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature>;

    /// 该 CLI 是否有用量统计能力。
    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature>;

    /// 监听：这个路径的变化算不算本 CLI 的会话变化。
    ///
    /// 归源的理由：判据是「本 CLI 的会话文件长什么样」—— 一个按格式代名判定、
    /// 一个只认固定正本名、一个要排除子代理目录。这是读族的知识，不该留在监听器里。
    fn is_session_event_path(&self, path: &Path, sessions_dir: &Path) -> bool;

    /// 定位该 CLI 的可执行文件。找不到给 `None`。
    ///
    /// 归源的理由：**大多数 CLI 是 PATH 上的二进制，但有的不是** —— 桌面应用型 CLI 要按
    /// 应用包的存在性判断。这个差异属于「这个 CLI 怎么被找到」，不是共用逻辑。
    fn locate_executable(&self) -> Option<String>;

    /// 墓碑写入时的兜底逻辑 id（索引里没有 id 时）。**允许读文件内容。**
    ///
    /// 多数 CLI 从路径推导，但有的 CLI 的 id 藏在文件内容里，路径推不出来。
    fn tombstone_fallback_id(&self, session_path: &Path) -> String;

    /// **仅凭路径**就能得到的逻辑 id；该 CLI 的 id 需要读文件内容时给 `None`。
    ///
    /// 与 `tombstone_fallback_id` 是**两个不同的契约，不要合并**：本方法的调用方在索引全表的
    /// 过滤里逐个调用它，读文件会让它变成 O(全表 × 读盘)。
    fn path_only_session_id(&self, session_path: &str) -> Option<String>;

    /// 该 CLI 的**自定义标题表**（`session_id → 标题`），没有则给空表。
    ///
    /// 归源的理由：多数 CLI 的标题就在会话文件里，索引已存；但有的 CLI 把用户重命名放在
    /// **自己的另一个库里**，与索引不同源。命令层与检索整形都要用它，两处各写一遍
    /// 「哪个 CLI 要去哪儿读」迟早分叉 —— 那正是本方法要消灭的形态。
    fn custom_titles(&self) -> HashMap<String, String>;

    /// 该 CLI 的**索引标题表**（`会话名 → 标题`）；没有则 `None`。
    ///
    /// 与 `custom_titles` 不是一回事：那是**用户在应用里改的名字**，
    /// 这是 CLI 自己维护的索引文件里的标题。有的 CLI 有，多数没有。
    ///
    /// 刻意不给默认实现：默认 `None` 会让「我忘了实现」与「本 CLI 没有索引」在类型上
    /// 无法区分，而能力位正是靠「有无实现」来对账的。
    fn index_titles(&self) -> Option<HashMap<String, String>>;

    /// 检索结果整形。各源决定用哪条装配路径与喂哪些 history 映射。
    ///
    /// 归源的理由与读族一致：用哪份 history 映射、按什么键回填展示名与项目路径，
    /// 正是「这个 CLI 的会话怎么整形」的知识；留在分派层按变体 `match`，每接一个新
    /// CLI 都要回那里补臂。
    ///
    /// **本方法引入一处刻意的反向依赖**：`cli_registry` 由此依赖 `commands::session` 的
    /// `build_claude_search_results` / `build_codex_search_results` / `AggregatedSessionSearch`。
    /// 这与本模块顶部记的 `parser` 反向依赖同源，是重构中间态的取舍、不是终局：终态是把
    /// 检索层也纳入源，那是更大的搬迁（要同时挪动聚合、索引读取与墓碑过滤的调用方），
    /// 本期不顺手做，只把方向记下来。
    fn build_search_results(
        &self,
        aggregated: Vec<AggregatedSessionSearch>,
        records: &HashMap<String, app_db::SessionListIndexRecord>,
        custom_names: &HashMap<String, String>,
    ) -> AppResult<Vec<SearchResult>>;

    /// 把归档内容解到会话源文件的**原始形态**。
    ///
    /// 多数 CLI 的源文件是明文，归档就是 `gzip(jsonl)`，本方法原样返回；
    /// 但有的 CLI 的源文件**本身还压了一层**，归档是 `gzip(zstd(jsonl))`，要多解一层。
    /// 归源的理由：这是「这个 CLI 的会话文件是什么格式」的知识。
    ///
    /// **必须容错**：内层解不出来时原样返回，不得报错 —— 取舍见 [`Truncation`]。
    fn decode_archived_source(&self, bytes: Vec<u8>, truncation: Truncation) -> Vec<u8>;

    /// 把归档内容压回该 CLI 源文件应有的形态，供恢复写回。
    ///
    /// 不压回去会让明文写到 `.zstd` 路径上，而该 CLI 的解析器按后缀走 zstd 解码 —— 读不了。
    fn encode_for_restore(&self, content: Vec<u8>, session_path: &str) -> AppResult<Vec<u8>>;

    /// 启动 PTY 前要写的**临时配置文件**内容/路径；不需要则 `None`。
    ///
    /// 归源的理由：要不要临时配置、写什么，是「这个 CLI 怎么启动」的知识。
    fn build_temp_settings(&self) -> AppResult<Option<String>>;

    /// 该 CLI 是否需要**钩子转发**（把它的钩子事件转给本应用）。
    ///
    /// 归源的理由：要不要中继钩子事件，是「这个 CLI 怎么启动、它的钩子长什么样」的知识，
    /// 与 `build_temp_settings` 同属 PTY 启动这一族。
    fn needs_hook_relay(&self) -> bool;

    /// 该 CLI 的**转录提取格式**；没有则 `None`。
    ///
    /// 归源的理由：这是「这个 CLI 的转录怎么抽成消息」的知识，与读族同源。
    /// 搬到源上之后，新 CLI 忘了声明格式会**编译不过** —— 此前它会被静默排除出助手回填与缓存统计。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat>;
}

/// 文件型源的归属判据：`key` 中是否出现描述符数据根的完整目录链（两侧都带分隔符边界）。
///
/// 目录名只在描述符里写一次，归属判定与数据目录解析因此不会各写一份而分叉；
/// 按目录段而非裸子串比对，`antigravity-cli-tools` 这类只是名字含同一串字符的目录不会被误认。
/// 逐段比对而不拼接针串：本函数在会话索引与解析分派的每个文件上都会被调用，不做堆分配。
pub(super) fn under_data_root(key: &str, kind: CliKind) -> bool {
    let segments = super::descriptor_for(kind).file_root.data_dir_segments;
    ['/', '\\'].into_iter().any(|sep| {
        key.match_indices(sep).any(|(i, _)| {
            let mut rest = &key[i + 1..];
            segments.iter().all(|seg| {
                match rest.strip_prefix(seg).and_then(|r| r.strip_prefix(sep)) {
                    Some(r) => {
                        rest = r;
                        true
                    }
                    None => false,
                }
            })
        })
    })
}
