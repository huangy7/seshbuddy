//! 扫描的索引机器。
//!
//! 与 `scan.rs` 的分工：`scan.rs` 里的 `build_projects_scan` 是**这一套机器的一种用法**，
//! 服务「一个定位符 = 一个文件 = 一份元数据」的文件型源；形态不吻合的源（会话是目录、
//! 一个会话对应多个候选文件或库中多行）直接驱动本模块，不必去适配那层封装。

use crate::session::SessionListMetadata;

use super::*;

/// 扫描的索引机器。源在自己的 `scan` 里驱动它：遍历自己的定位符，逐条把裁决交给它。
///
/// **这是库，不是框架** —— 它不调用源，源调用它。库里缺能力时往这里加方法，
/// 已有的源一行不动；这正是把「骨架读取闭包产出」换成「源写入累加器」的全部意义。
pub(crate) struct ScanAccumulator<'a> {
    kind: CliKind,
    force: bool,
    custom_names: &'a HashMap<String, String>,
    /// 库里已知的索引行，**恒不受 `force` 影响**。陈旧判定基准、展示状态回落、
    /// 换代搬迁的「已入库路径」判据都以它为据。
    known: HashMap<String, app_db::SessionListIndexRecord>,
    tombstones: app_db::TombstoneFilter,
    index_updates: Vec<app_db::SessionListIndexRecord>,
    seen: HashSet<String>,
    invalid: HashSet<String>,
    sessions: Vec<RawSession>,
}

/// 复用视图为空时的替身，避免 `force` 下为了返回一个空表而复制整份已知索引。
static EMPTY_INDEX: LazyLock<HashMap<String, app_db::SessionListIndexRecord>> =
    LazyLock::new(HashMap::new);

impl<'a> ScanAccumulator<'a> {
    /// 读库备好索引机器。
    ///
    /// `force` 只影响「是否复用缓存元数据」，不影响陈旧判定基准 —— 否则强制刷新时
    /// 陈旧行会永久滞留在列表里（数据源切换、会话删除后都清不掉）。
    pub(crate) fn new(
        kind: CliKind,
        custom_names: &'a HashMap<String, String>,
        force: bool,
    ) -> AppResult<Self> {
        Ok(Self {
            kind,
            force,
            custom_names,
            known: load_session_list_index(kind),
            tombstones: app_db::load_tombstone_filter(kind).unwrap_or_default(),
            index_updates: Vec::new(),
            seen: HashSet::new(),
            invalid: HashSet::new(),
            sessions: Vec::new(),
        })
    }

    /// 供 `build_projects_scan` 构造逐条解析的上下文。
    pub(crate) fn context(&self) -> ScanContext<'_> {
        ScanContext {
            force: self.force,
            cached_index: &self.known,
            tombstone_filter: &self.tombstones,
        }
    }

    /// 复用视图里的某一行：`force` 时恒为 `None`。
    fn cached_for_reuse(&self, key: &str) -> Option<&app_db::SessionListIndexRecord> {
        if self.force {
            None
        } else {
            self.known.get(key)
        }
    }

    /// **已知视图**整表。换代搬迁要按「库里已入库的路径」判断哪些旧代记录需要改写，
    /// 那是整表遍历而非单键查询。
    pub(crate) fn known_index(&self) -> &HashMap<String, app_db::SessionListIndexRecord> {
        &self.known
    }

    /// **复用视图**整表：`force` 时为空，否则等于 `known_index()`。
    /// 与 `push_session` 内部的复用判定同源。
    pub(crate) fn reusable_index(&self) -> &HashMap<String, app_db::SessionListIndexRecord> {
        if self.force {
            &EMPTY_INDEX
        } else {
            &self.known
        }
    }

    /// 墓碑判定：逻辑键或物理路径任一命中即算。
    pub(crate) fn is_tombstoned(&self, logical_key: &str, path: &str) -> bool {
        self.tombstones.is_tombstoned(logical_key, path)
    }

    /// 标记「本轮见过」：其索引行不按陈旧处理。
    pub(crate) fn mark_seen(&mut self, key: &str) {
        self.seen.insert(key.to_string());
    }

    /// 标记「在、但用不了」（解析失败、候选链全落空）：收尾时其索引行**与陈旧行一并清除**。
    ///
    /// 失效集合在 `collect_stale_paths` 里无条件并入陈旧集合，不受存活比例守卫拦截 ——
    /// 守卫只保护「压根没枚举到」的行（瞬时故障时不误清），而源逐条判定的失效是明确裁决，
    /// 留着只是一行没有内容的半截元数据；下次扫描恢复可解析时会重新入库。
    ///
    /// **与 `mark_seen` 分开** —— 只记失效的源在存活守卫里不计入存活数，大面积部分失败时
    /// 不该被当成「扫到了这么多会话」，以免掩盖真正的批量删除。
    pub(crate) fn mark_invalid(&mut self, key: &str) {
        self.invalid.insert(key.to_string());
    }

    /// 解析失败的通用形态：失效与已见都记。文件型源用它即可。
    pub(crate) fn mark_unparsed(&mut self, key: &str) {
        self.mark_invalid(key);
        self.mark_seen(key);
    }

    /// 收一条会话：记已见、必要时写索引行、装配列表项。
    ///
    /// `key` 是该会话的**持久化键**（索引行键、`file_path`）—— **由源决定**。
    /// 定位符与持久化键不总是同一个串：DSH 的定位符是会话目录，持久化键是目录下
    /// 按候选顺位选中的那个文件。
    pub(crate) fn push_session(
        &mut self,
        key: &str,
        metadata: SessionListMetadata,
        projection: ScanProjection,
    ) {
        let cached = self.cached_for_reuse(key).cloned();
        self.push_session_inner(key, metadata, projection, cached);
    }

    /// 同 `push_session`，但「库中已有行」由源指定而非从复用视图按键取。
    ///
    /// 既用于展示状态（归档快照、已归档）的回落，也用于「内容未变则不重写索引行」的判定。
    /// 换代搬迁后旧记录不在新键下时用它。
    pub(crate) fn push_session_with_cached(
        &mut self,
        key: &str,
        metadata: SessionListMetadata,
        projection: ScanProjection,
        cached: Option<app_db::SessionListIndexRecord>,
    ) {
        self.push_session_inner(key, metadata, projection, cached);
    }

    /// 装配一条会话。`cached` 是「该键上库中已有的行」，既用于展示状态回落，
    /// 也用于「内容未变则不重写索引行」的判定。
    ///
    /// 收成私有实现：Task A 只有 `push_session` 一个入口（库中行从复用视图按键取）；
    /// 形态不吻合的源需要自己指定这一行（换代搬迁后旧记录不在新键下），届时再加一个
    /// 公开入口把 `cached` 透出来。
    fn push_session_inner(
        &mut self,
        key: &str,
        mut metadata: SessionListMetadata,
        projection: ScanProjection,
        cached: Option<app_db::SessionListIndexRecord>,
    ) {
        // 先记已见：改动前 6 个文件型源与 DSH 都是「先记已见、再写索引」，无一例外。
        self.mark_seen(key);

        // 项目路径纠正：投影给出的「最终采用」值常与元数据里的不同（目录结构反解、
        // 外部映射）。差异要写回索引，否则快照（读索引）会与实时扫描的项目分组分叉。
        if metadata.project_path != projection.project_path {
            metadata.project_path = projection.project_path.clone();
        }

        // 索引行按「解析结果 vs 库中已有行」比对后落库：内容与 mtime 都一致就不重复写
        // （写会刷新 indexed_at，而 upsert 不触碰归档标记，内容本就不变）。
        // `key` 是持久化字符串键：非 UTF-8 路径 lossy 后已非真实路径，stat 失败、`modified_ms` 取 0，
        // 该会话每轮重解析；可接受 —— 改动前的逐 CLI 用 `to_str().unwrap_or("")` 本就退化成空键。
        let modified_ms = fs::metadata(key)
            .ok()
            .map(|file_metadata| file_modified_ms(&file_metadata))
            .unwrap_or_default();
        let up_to_date = cached
            .as_ref()
            .map(|record| index_record_is_current(record, &metadata, modified_ms))
            .unwrap_or(false);
        if !up_to_date {
            self.index_updates.push(build_session_list_index_record(
                key,
                modified_ms,
                &metadata,
            ));
        }

        let session_id = metadata.session_id;
        let display_name = crate::title_resolver::resolve_display_name(
            crate::commands::session::custom_session_name(self.custom_names, self.kind, key),
            projection.title.as_deref(),
            metadata.first_user_message.as_deref(),
            projection.history_display.as_deref(),
            &session_id,
        );
        let timestamp = metadata
            .last_timestamp
            .or(metadata.first_timestamp)
            .unwrap_or_default();
        let has_archive_snapshot = cached
            .as_ref()
            .map(|record| record.has_archive_snapshot)
            .unwrap_or(false);
        let is_archived = cached
            .as_ref()
            .map(|record| record.is_archived)
            .unwrap_or(false);

        self.sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path: key.to_string(),
                display_name,
                timestamp,
                file_size: metadata.file_size,
                git_branch: projection.git_branch,
                has_archive_snapshot,
                is_archived,
                cli_id: self.kind.id().to_string(),
            },
            encoded_dir: projection.encoded_dir,
            original_path: projection.original_path,
        });
    }

    /// 收尾：清陈旧行、落库索引更新、返回列表。
    pub(crate) fn finish(self, unverified_prefixes: &[PathBuf]) -> AppResult<Vec<RawSession>> {
        persist_session_list_index_changes(
            self.kind,
            &self.known,
            &self.seen,
            self.index_updates,
            &self.invalid,
            unverified_prefixes,
        );
        Ok(self.sessions)
    }
}

/// 库中已有行是否已经等于本轮解析结果 —— 相等就不必再写一次。
///
/// 比对含 `modified_ms`：只改了 mtime 而内容未变的文件若被判为「无需写」，下一轮缓存仍会
/// 因 mtime 不符而落空、反复重解析，故时间戳也参与判等。
fn index_record_is_current(
    record: &app_db::SessionListIndexRecord,
    metadata: &SessionListMetadata,
    modified_ms: i64,
) -> bool {
    record.session_id == metadata.session_id
        && record.project_path == metadata.project_path
        && record.title == metadata.title
        && record.first_user_message == metadata.first_user_message
        && record.first_timestamp == metadata.first_timestamp
        && record.last_timestamp == metadata.last_timestamp
        && record.git_branch == metadata.git_branch
        && record.file_size == metadata.file_size
        && record.modified_ms == modified_ms
}
