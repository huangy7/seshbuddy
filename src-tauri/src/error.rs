use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::Serialize;

/// Unified application error type.
///
/// Replaces the scattered `Result<T, String>` + `.map_err(|e| format!(...))` pattern.
/// Two shapes cross IPC (see the `Serialize` impl below): `Business` renders as a bare
/// string, `Coded` as `{ code, params }` for the frontend to localize.
///
/// `Clone`：流式事件的 payload 类型要满足 `streaming::emit_done` 的 `T: Clone`，
/// 而 fan-out 的分项错误要原样（不压平）装进 payload，故 `AppError` 需可克隆。
/// 变体字段全是 `String` / `&'static str` / `BTreeMap`，克隆是浅拷贝。
#[derive(Debug, Clone, thiserror::Error)]
pub enum AppError {
    /// 仍是字符串的错误。**今天没有生产站点构造它**（实测 0，计划 11 的 T4；计划 13 的 T2b
    /// 删掉最后一个产出它的 `From` 工厂后仍是 0）——产出它的只剩 `AppError::business`
    /// 与测试夹具（三个 `From` 工厂 `String` / `&str` / `tauri::Error` 已全部删除，
    /// 见下方「已删除」块）。
    ///
    /// **T2b 的独立判据**（实测，在本行所在提交上跑，与下方 `Serialize` 段那条互为旁证）：
    /// `command grep -rn 'AppError::Business\|Self::Business' src-tauri/src src-tauri-proxy/src transcript-store/src | command grep -v ':[0-9]*:\s*//'`
    /// → **2 行**，即代码只剩 **2 处**：`error.rs` 里 `business()` 构造器自身的
    /// `Self::Business(msg.into())`，与 `streaming.rs` 的 `#[cfg(test)]` 夹具。
    /// 两处都**不构造生产站点**；`business()` 的调用点则全在测试夹具里
    /// （见下方 `Serialize` 段的 7 处实测），故**生产构造站点为 0**。
    ///
    /// ⚠️ **末尾那个 `| command grep -v ':[0-9]*:\s*//'` 不是装饰**：`AppError::Business`
    /// 这个模式也会命中**注释里提到它**的行——**本段自己就写了好几行**（上面那条命令、
    /// 上面那句 `Self::Business(msg.into())` 的说明，以及**本警告行自己**：它为说明这件事
    /// 又把这个模式写了一遍）。这与下方「本族至此清空」段要求 `^` 锚定是同一类修法：
    /// **判据的文本不能包含它自己在数的东西**。
    #[error("{0}")]
    Business(String),

    /// 以错误码跨 IPC 返回的结构化错误：前端按 `errors.<code>` 取当前语言的文案，
    /// `params` 是插值实参。
    ///
    /// `code` 取 `&'static str` 而非枚举：码表由前端语言包定义，Rust 侧穷举不出来，
    /// 枚举只会带来上百个变体的样板；静态生命周期还顺带让「码在运行时拼出来」变成编译错误。
    ///
    /// **码在语言包里存不存在由闸门的规则 10 校验**（`scripts/check-i18n.mjs`：
    /// 扫全部 `coded("…")` 字面量，与 `errors.*` 双向闭合），故这里不再只是注释里的约定。
    ///
    /// `#[error]` 显式写成 `{code}`：thiserror 在没有该属性时会回落到**首字段的 `Display`**，
    /// 结果相同，但那只在 `code` 恰好仍是首字段时成立——字段一换序，`Display` 就静默变了意思。
    #[error("{code}")]
    Coded {
        code: &'static str,
        params: BTreeMap<String, String>,
    },
}

impl AppError {
    /// Create a business-logic error from a string message.
    /// Use this for validation errors like "模型名称不能为空".
    pub fn business(msg: impl Into<String>) -> Self {
        Self::Business(msg.into())
    }

    /// 构造一个以错误码跨 IPC 返回的错误。
    ///
    /// `code` 要能在前端语言包的 `errors` 命名空间里查到——Rust 编译器校验不了这一点，
    /// 由闸门的规则 10 校验（扫全部 `coded("…")` 字面量与 `errors.*` 双向闭合）。
    /// 缺条目时前端 `renderAppError` 会回退成裸 code（不崩，但用户看到的是码）。
    pub fn coded(code: &'static str) -> Self {
        Self::Coded {
            code,
            params: BTreeMap::new(),
        }
    }

    /// 追加一个插值实参。
    ///
    /// **非 `Coded` 变体上无操作**：`with` 的调用点未必静态知道变体（错误可能来自连接池、
    /// IO 或仍是字符串的 `Business`），它只负责补上下文，不该让一次「补充」把已经构造好的
    /// 错误变成崩溃；字符串形状里也没有 params 位可放。
    pub fn with<K: Into<String>, V: Into<String>>(self, key: K, value: V) -> Self {
        match self {
            Self::Coded { code, mut params } => {
                params.insert(key.into(), value.into());
                Self::Coded { code, params }
            }
            other => other,
        }
    }

    /// **只给日志用**：`Coded` 渲染成 `code` + 逐个 `k=v`，其余变体沿用 `Display` 的现有输出
    /// （它们本来就带可读文本）。
    ///
    /// **为什么不改 `Display`**：`Display` 是**裸 code**，喂给前端的 `String(err)` 兜底与
    /// `Serialize` 的字符串分支，**两者都面向用户**——让它带上 `detail` 会把原始 OS 文本推上屏，
    /// 正是 R3 要防的。诊断信息量的问题在**调用点**，不在 `Display`。
    ///
    /// **不截断、不转义 `detail`**：日志要的就是那段原始 OS 文本。参数值里的空格与换行原样输出，
    /// 故本方法的输出**不是**可解析的格式，只供人读。
    ///
    /// 无 params 的 `Coded` 输出与 `Display` 逐字节相同（只有 code），故站点上的信息量只增不减。
    pub fn diagnostic(&self) -> String {
        match self {
            Self::Coded { code, params } => {
                let mut out = String::from(*code);
                for (key, value) in params {
                    out.push(' ');
                    out.push_str(key);
                    out.push('=');
                    out.push_str(value);
                }
                out
            }
            other => other.to_string(),
        }
    }
}

/// 基础设施错误的构造口：底层库 / OS 的错误统一转成 `Coded`，包装文案由语言包提供。
///
/// **底层文本是英文 / OS 文案，不翻译**（R4）——它进 `params.detail` 原样透出，不进语言包；
/// 面向用户的那层包装文案在 `errors.internal.*`，四语各一份。
///
/// 形态是手写的 `From` 而非 `#[from]`：`#[from]` 只能生成「变体持有源错误」的转换，
/// 而这里要的是「转成 `Coded` 并带上 `detail`」。`?` 与 `map_err(AppError::from)` 两种写法
/// 都照旧可用，调用点一处都不用改。
///
/// 代价一（刻意接受，本仓无一处读它）：thiserror 的 `#[from]` 会把源错误自动接成
/// `Error::source()`，手写 impl 没有这条链。
///
/// 代价二（已消除）：曾有的 `impl From<AppError> for String` 会让 `Coded` 经那条路径退化成
/// 裸 code（`Display` 就是 code，插值实参全丢）。**该 impl 已由计划 11 的 T5 删除**，
/// 口径见下面「实测射程」末尾。
///
/// **实测射程（逐值追过，不是按类型推的）**：只有把 `AppResult` 显式压成 `String`
/// 的那几处会把裸 code 推给用户。曾逐条点名的三处**现已全部关闭**，逐条记下落点——
/// - `src-tauri/src/cli.rs` 的 `write_cli_path_overrides(...).map_err(|e| e.to_string())`
///   （`AppResult` 来自 `conn()?` 与 rusqlite），曾经 `commands/settings.rs` 的
///   `set_cli_data_dir`（已注册的命令）跨 IPC。**该站点已随签名加宽删掉**：实测 `cli.rs` 里
///   `map_err(|e| e.to_string())` 命中 0 处，且 `write_cli_path_overrides` 的签名由
///   `cli.rs` 的类型守卫钉在 `AppResult<()>` 上，改回去编译不过。
/// - `src-tauri/src/updater.rs` 的 `read_updater_settings` / `write_updater_settings`
///   （`db/settings.rs` 的 `conn()?`、`query_row(...).optional()?`、
///   `serde_json::from_str(...).map_err(AppError::from)`），曾经 `get_updater_state`
///   与 updater 面板上屏。**该站点已由 B8 删掉**（实测：四个
///   `.map_err(|e| e.to_string())` 命中 0 处，`build_updater_state` 等六个函数的
///   签名加宽成 `AppResult`，由 `updater.rs` 的类型守卫钉住）。
/// - `src-tauri/src/commands/session_index/mod.rs` 的
///   `run_cli_fanout` 里曾有的 `operation(kind).map_err(|error| error.to_string())`，
///   落进 `SearchStreamCliError.message` 与 `ScanProjectsCliResult.error` 两条通道推给前端。
///   （第一个字段当时名为 `message`，B4 已改名为 `error`；两处**当前**的名字都是 `error`。）
///   **该站点已由 B4 删掉**（实测：`run_cli_fanout` 现在直接透传 `operation(kind)`，
///   两个 payload 字段的类型已是 `AppError`），压平不再发生。
///
/// 这张清单**不是**「压平已绝迹」的证明，而是「追过的都追完了」的记录：新写
/// `.map_err(|e| e.to_string())` 消费 `AppResult` 时，本清单要跟着补一条。
///
/// **`src-tauri/src/proxy.rs` 那三处 `map_err(String::from)` 曾误记入，已两次订正，今天不在
/// 其中，也不能再按原来的理由排除**：写下那条时的理由是它们调的 `cli_config.rs` helper 把
/// **每一个**底层错误都包成 `AppError::Business`（`Business` 的 `Display` 是 `{0}`，压成
/// `String` 逐字节不变）——**该前提自 T2 起不成立**：`cli_config.rs` 已迁成 `coded()`
/// （实测：该文件现有 3 个 `coded()` 站点），那三处若还在，`Coded` 当时就会经
/// `From<AppError> for String` 退化成裸 code（**那条 impl 已由 T5 删除**，同一形状今天编译不过）。
/// 而**三处站点本身也已被 T2 删掉**（`6aaa4e16`，实测：`proxy.rs` 里 `map_err(String::from)`
/// 命中 0 处），改成让 `AppResult` 一路传到 IPC。B3 把 `proxy.rs` 其余的错误站点一并迁成
/// `coded()`——**本句的「其余」按下面这个口径读**：闸门自己的 `findCodedCodeLiterals` 在
/// `319f5622` 上数得该文件 **52** 个生产 `coded()` 站点（`6b5b97ce` 的 diff 新增 51 处，
/// 另 1 处迁前已有）。侦察阶段记过「54 个错误站点」，那是**另一种口径**（迁前该文件
/// 62 条生产 CJK 减去 8 条数据类/标签，在 `6b5b97ce^` 上量得）——两数不是同一个量
/// （迁移里同时有合并与拆分：8 个 `unsupported_kind` 站点并成 2 个 `match` 臂，
/// `ensure_port_available`/`require_query_kind` 两个 `format!` 中间函数被删而码落到
/// 5 个调用点），**引用时以 52 为准**。并连带把 17 个 `Result<_, String>` 函数加宽成
/// `AppResult`、删掉那 2 个只做 `format!` 的中间函数（`ensure_port_available`、
/// `require_query_kind`）——不加宽就没有一处装得下 `Coded`，`?` 会在链上换一处继续压平。
///
/// 上面这几处属「`Result<_, String>` 完全绕过 `AppError` 序列化」这一类；修法是逐站点停止
/// 把 `AppResult` 压成 `String`。
///
/// **该类已于计划 11 的 T4 收口（实测口径）**：三个 crate（`src-tauri` / `src-tauri-proxy` /
/// `transcript-store`）里 **35 个 `fn` 返回 `Result<_, String>`**（33 生产 + 2 `#[cfg(test)]`；
/// 尖括号 / 圆括号配平扫描器（`{` 只当返回类型的终止符，不参与配平）在基线 `a60d2715` 上量得，T8 复测）全部加宽成 `AppResult`，
/// 实测剩 0；唯一残留的 `Result<_, String>` 是 `tray.rs` 里一个
/// **局部闭包**的类型标注（`rebuild_tray_menu_with_update` 内的 channel 载荷，转成 `String`
/// 只为满足 `Send`），不是签名，也不跨 IPC。**边界**：枚举用尖括号/圆括号配平扫描器做，
/// 编译器只能做旁证（`#[cfg(windows)]` 门控的代码在主机构建里不可见，本机无法交叉检查）。
///
/// **本清单今天的口径（计划 11 的 T5 之后）**：`impl From<AppError> for String` **已删**
/// （T5 实测：删除后 `cargo check --workspace --all-targets` 全绿，即删除当时已无使用者）。
/// 所以**「在 `Result<_, String>` 函数里对 `AppResult` 用 `?`」现在是一条编译错误**
/// （`E0277: ? couldn't convert the error to String`），**不再是静默压平**。
/// **它没有覆盖的形状**（清单本身仍然要人工维护）：① 显式的 `.to_string()` /
/// `format!("{err}")` 仍然合法（站点看得见，但照样丢 `params`）；② **连签名带函数体一起**
/// 回退成 `String` 的协同回退不碰任何 `AppError`，编译器看不见——那由各文件的类型守卫钉住
/// （如 `updater.rs` 的 `app_error_signature_guard`）。
impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::coded("internal.database").with("detail", err.to_string())
    }
}

impl From<r2d2::Error> for AppError {
    fn from(err: r2d2::Error) -> Self {
        AppError::coded("internal.pool").with("detail", err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::coded("internal.io").with("detail", err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::coded("internal.json").with("detail", err.to_string())
    }
}

impl From<tantivy::TantivyError> for AppError {
    fn from(err: tantivy::TantivyError) -> Self {
        AppError::coded("internal.tantivy").with("detail", err.to_string())
    }
}

/// Serialize for Tauri IPC, dispatched by variant.
///
/// `Coded` 发 `{"code": "...", "params": {...}}`：前端 `invokeApp.ts` 的 `isCodedError`
/// 按 `code` 是字符串来认这个形状，缺条目时用 `err.params ?? {}`，故两个字段名与
/// 「无 params 也发空对象」都是跨进程契约的一部分。
///
/// **其余变体（含 `Business`）必须保持裸字符串形状（R1）**：前端
/// `AppErrorPayload = string | CodedError` 与三处 `isCodedError` 判据都按这个线格式认人，
/// `Business` 变体本身也还活着（7 处 `AppError::business` 测试夹具 + `streaming.rs` 那 1 处
/// 直接构造 `Business` 的夹具；三个 `From` 工厂已全部删除，生产构造站点为 0），
/// 改形状要等那个变体被删掉之后。
/// **实测（终审后重测，`dabf45d1`）**：`AppError::business` 的调用点在三个 crate 里
/// 只剩 **7** 处，**全部是测试夹具**（`error.rs` ×3、`commands/pty.rs` ×2、
/// `assistant/commands.rs` ×1、`commands/session_index/tests.rs` ×1）——生产站点为 0。
/// 量它的命令（三 crate、只认调用点）：
/// `command grep -rnE 'AppError::business\("|business\(format!\("' src-tauri/src/ src-tauri-proxy/src/ transcript-store/src/ | wc -l` → 7。
/// T4 首测时是 6 处（`error.rs` ×2），第 3 处是 T6 在 `error.rs` 新增的夹具
/// ——两个数都对，是时间差。
/// 本段原写「后端仍有 188 处未改造」，那是 T2/T3 迁完之后就陈旧的数（历史：计划 11 的
/// T2 关掉 36 处手写英文、T3 关掉 55 处透传），**引用时以 0 为准**。
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            // `params` 是 `BTreeMap`，键序天然稳定，同一份错误的序列化结果唯一。
            Self::Coded { code, params } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("code", code)?;
                map.serialize_entry("params", params)?;
                map.end()
            }
            other => serializer.serialize_str(&other.to_string()),
        }
    }
}

/// Convenience alias used across the crate.
pub type AppResult<T> = Result<T, AppError>;

// **已删除（计划 13 的 T2）：`impl From<String> for AppError` 与 `impl From<&str> for AppError`。**
//
// 两条都是**静默造 `Business`** 的工厂：任何 `Result<_, String>` 函数里的 `?` /
// `.map_err(AppError::from)` / `impl Into<AppError>` 实参位传入 `String` 或 `&str` 都会走它们，
// 造出的错误里没有归属层——**对每一种 `business(...)` 形状的 grep 都不可见**。
// 删掉它们之后，「`String` / `&str` → `AppError`」的桥不存在了，任何这类回退立刻变成
// `E0277: ? couldn't convert the error to AppError`（**是编译错误，不是静默压平**）。
//
// **历史：这一族被连续四条计划延后，四次都是同一条理由**——**删除无法在本机验证**：
// `#[cfg(windows)]` / `#[cfg(target_os = "windows")]` 门控的代码在主机构建里对编译器**不可见**
// （本机跑不起 `x86_64-pc-windows-msvc` 交叉检查，`ring` 的构建脚本需要 Windows SDK，
// 仓里已在 `context_menu.rs` 记过），删 impl 有可能只在 Windows 上编译不过，而当时没有任何
// 命令能发现。**计划 13 的 T1 造了那把尺子**（`scripts/audit-windows-cfg.mjs`：逐条读门控区域内
// 每个 `?` 的错误来源表达式），本步才第一次有判据可依。
//
// **删除的判据（实测，不是按类型推的）**：给两条 impl 各加 `#[cfg(any())]` 后跑
// `cargo check --workspace --all-targets`，读到——
// - `From<String>`：**恰好 1 处** `E0277`，`src-tauri/src/streaming.rs:125` 的 `#[cfg(test)]`
//   夹具（`"…".to_string().into()`）。该夹具已改成显式 `AppError::Business(...)`，
//   它钉的**序列化形状**（裸字符串而非 `{code, params}`）原样保留。
// - `From<&str>`：**0 处**（与计划 11 的 T4 在同一形状上量到的一致）。
// 两条 impl 均已删除；删除后 `cargo check --workspace --all-targets` 全绿。
//
// ⚠️ **上面那两个数是在【改夹具之前】的提交上量的，在今天的 HEAD 上不再复现**：
// 夹具已不再走 `From<String>`，故今天把这两条 impl 按原样加回、各加 `#[cfg(any())]` 再跑
// `cargo check --workspace --all-targets`，读到的是 **0 处错误（EXIT=0，实测）**。
// **两个数都对，是时间差**——判据必须连同**测量时点**一起记，否则它会在下一个提交上变成假话。
//
// **T1 那把尺子的边界（不要读成「Windows 已编译验证」）**：T1 是**形状审计 + 手读来源表达式**，
// **不是 Windows 编译证明**——本机没有 Windows 编译，CI 的 `windows` job 至今**一次都没执行过**
// （待首次推送）。
//
// **T1 的结果（判据同上：形状审计 + 手读，不是编译证明）**：`scripts/audit-windows-cfg.mjs` 在
// Windows 门控区域（65 个）内报 **32** 条形状候选，**32/32 逐条读**后 **0 条**的 `?` 来源可能是
// `String` / `&str` / `tauri::Error`——**这就是本步删除的前置条件**。
//
// **T1 为什么没拦住本步那 2 处 macOS 站点（两个独立原因）**：
// ① **射程**：T1 只扫 **Windows 门控区域**（扫描根 `git ls-files src-tauri/src`，
//    `src-tauri-proxy` 也在射程外），而 `commands/settings.rs` 那两处是
//    `#[cfg(target_os = "macos")]`——**从来不在候选清单里**。这是主因。
// ② **无类型推断**：尺子**会**列出 `?` 行（`SHAPES` 的第一项就是 ``[`?`]``；实测 32 条候选
//    **全部**带 ``[`?`]`` 标记），但它**解析不出 `?` 的错误来源类型**，给出的只是候选清单，
//    要人逐条读。**别把「列得出 `?` 行」读成「判得出这个 `?` 需不需要那条 impl」。**
//
// **变异证明（计划 13 的 T2，阳性对照，实测）**：在 `error.rs` 末尾注入
// `fn probe() -> AppResult<()> { let r: Result<(), String> = Err("x".to_string()); r?; Ok(()) }`
// （`String` 来源经 `?` 进 `AppResult` 槽——正是这两条 impl 提供的桥）——
// **删除后编译失败**：`E0277: ? couldn't convert the error to error::AppError`（1 处）；
// **把 `From<String>` 加回同一份文件后，同一段代码编译通过**（0 错）。故失败由删除引起，
// 而不是注入本身。
//
// ⚠️ **别拿「`AppResult` 塞进 `Result<(), String>`」那个方向的注入来证明本步**：它测的是
// 计划 11 的 T5 删掉的 `From<AppError> for String`，与本步删的这两条无关。**实测**：
// `fn f() -> Result<(), String> { let r: AppResult<()> = Err("x".to_string()); r?; Ok(()) }`
// 在 `From<String>` **在**与**不在**两种情况下都失败，且报错完全相同（`E0308`: `String` 填不进
// `AppError` 槽 + `E0277: ? couldn't convert the error to std::string::String`）——
// **它对这两条 impl 的存亡不敏感，用它会得到假绿/假红**。
//
// ⚠️ **计划 11 的 T5 删的是相反方向的那条**（`From<AppError> for String`，见本文件末尾），
// **不是这两条**。

// **已删除（计划 13 的 T2b）：`impl From<tauri::Error> for AppError`。**
//
// 本族的第三条、也是最后一条。与上面两条一样是**静默造 `Business`** 的工厂
// （`tauri::Error` 的文本直接当消息，没有归属层），但它不是死代码。
//
// **删除的判据（实测，不是按类型推的）**：**删除前**给本 impl 加 `#[cfg(any())]` 后跑
// `cargo check --workspace --all-targets`，读到 **3 条** `E0277` 诊断，落在
// `src-tauri/src/commands/settings.rs` 的 **2 个站点**上（同一个站点会报两条：整个调用表达式
// 一条、触发转换的那个 token 一条）——
// - **站点 1**（`refresh_macos_app_icon`，`commands/settings.rs`）：显式
//   `.map_err(AppError::from)` 接 `app.run_on_main_thread(...)` 的返回值。形状是
//   **显式 `AppError::from`**。
// - **站点 2**（`apply_dock_visibility`，同一文件）：`app.set_dock_visibility(visible)?`。
//   形状是 **`?` 的隐式转换**——**3 条诊断里只有这 1 条是 `?` 形状**。
//
// ⚠️ **定位这两个站点请用函数名，不要用行号**：行号会被任何一次改动顶偏（本块已为此吃过
// 一轮修复），而函数名在重命名之外是稳定的。探针当时的行号读数**仅作追溯、不是现值**：
// 站点 1 报在 `73:5`（整个表达式）与 `86:14`（`AppError::from`），站点 2 报在 `97:37`。
//
// 删除后 `cargo check --workspace --all-targets` 全绿（本步实测）。
//
// 两处函数都是 `#[cfg(target_os = "macos")]` 门控——**这正是它此前四次都没被探到的原因**：
// 计划 11 的 T4 探的是 `From<String>` / `From<&str>`，本 impl 只被「具名登记给下一轮」而从未跑过
// 探针；计划 13 的 T1 的射程是 **Windows 门控区域**（扫描根 `git ls-files src-tauri/src`，
// `src-tauri-proxy` 亦在射程外），macOS 门控的这两处**从来不在它的候选清单里**。
// （T1 那把尺子的能力边界，见上方「已删除」块。）
//
// **删除必须与那 2 处站点作为一件事一起做**：改成 `coded()` 需要一个新错误码，而 T2 当时
// 实测 `errors.internal.*` 只有 6 条（`database` / `pool` / `io` / `json` / `tantivy` /
// `lock_poisoned`），没有一条是「tauri 运行时调用失败」；T2 的约束又是不改任何语言包、
// 文件范围不含 `commands/settings.rs`。故 T2 只删了前两条，本步（T2b）补齐后两件事：
// **新码 `internal.tauri`（四语各一条）+ 那 2 处站点**，两处都写成
// `AppError::coded("internal.tauri").with("detail", e.to_string())`。
//
// 形态与既有家族一致：`internal.*` 是**子系统形状**的家族，故这里也是**一个码**，
// 不为每个失败点各造一个。
//
// ⚠️ **上面这句是设计取舍，不是测量**——「`internal.io` 用一个码覆盖许多不同的 io 失败」
// 里的「许多」静态数不出来（io 失败点的个数不可数），故不写成实测。
// **可测量的那一半**（实测，在本段所在提交上跑）：
// `command grep -rn 'coded("internal.io")' src-tauri/src src-tauri-proxy/src transcript-store/src | command grep -v ':[0-9]*:\s*//'`
// → **3 行**，其中 2 行落在 `error.rs` 的 `#[cfg(test)] mod tests` 内，故**生产 `coded()` 站点
// 只有 1 处**（`From<std::io::Error>` 那条 impl）——全仓每一处把 `io::Error` 交给 `AppError`
// 的 `?` / `.map_err` 都从这一个码出去，而不是按失败点各造一个码。
// ⚠️ 过滤段同上：本段自己那行命令含 `coded("internal.io")`，不过滤会把它数成站点。
//
// **本族至此清空**：`From<String>` / `From<&str>`（T2）与 `From<tauri::Error>`（T2b）全删，
// 三条都不在了。本文件里剩下的 `impl From<…> for AppError` 只有 **5** 条，全是把底层库错误
// 转成 `Coded` 的那一类（`rusqlite` / `r2d2` / `io` / `serde_json` / `tantivy`），不产出 `Business`。
//
// 量它的命令**必须行首锚定 `^`**，实测输出（写下本行时的读数）：
//     command grep -rn '^impl From<.*> for AppError' src-tauri/src
//     src-tauri/src/error.rs:193:impl From<rusqlite::Error> for AppError {
//     src-tauri/src/error.rs:199:impl From<r2d2::Error> for AppError {
//     src-tauri/src/error.rs:205:impl From<std::io::Error> for AppError {
//     src-tauri/src/error.rs:211:impl From<serde_json::Error> for AppError {
//     src-tauri/src/error.rs:217:impl From<tantivy::TantivyError> for AppError {
//     → 5 行 = 5 条 impl
//
// ⚠️ **不锚定会连注释一起数进来**，这个坑本块已经踩过一次：首版把不锚定的同一命令当判据写进
// 注释、报「→ 5 条」，而在 `ce98e1cd`（首版所在提交）上它的**实测输出是 10 行**——5 条 impl
// **加 5 行注释**（`grep -c '^\s*//.*impl From<.*> for AppError' src-tauri/src/error.rs` → 5），
// 其中 3 行就是本块自己写下的（「已删除」标题、上面那句「只有 5 条」、以及命令那一行）。
// 锚定 `^` 之后这 5 行全部落选。
// **判据是「输出 5 行」，不是上面那 5 个行号**——行号会被上游改动顶偏，输出行数不会。

// **已删除（计划 11 的 T5）：`impl From<AppError> for String`。**
//
// 它曾是静默压平的机制：让 `AppResult` 在 `Result<_, String>` 函数里用 `?` 时
// **悄悄退化成裸 code**（`Display` 就是 code，插值实参全丢）。类型守卫钉在**签名**上，
// 覆盖不到**函数体内部的 `?`**；删掉 impl 才是判据——**编译器知道类型，正则不知道**。
//
// **实测（T5，删除当时）**：删掉后 `cargo check --workspace --all-targets` **全绿**，
// 即该 impl 已无使用者（T4 已把三个 crate 的 35 个 `Result<_, String>` 签名加宽成
// `AppResult`，唯一残留的是 `tray.rs` 里一个局部闭包的类型标注）。**所以删除是纯收益**：
// 它不修任何现存站点，只把「静默压平」变成一条编译错误。
//
// **变异证明（T5）**：注入 `fn f() -> Result<(), String> { let r: AppResult<()> = …; r?; Ok(()) }`
// → 编译失败（`E0277: ? couldn't convert the error to String`）；把本 impl 临时加回 →
// 同一段代码编译通过（阳性对照，证明失败由删除引起而非注入本身）。
//
// **它没有关掉的形状**见上方 `impl From<rusqlite::Error> for AppError` 的「实测射程」末尾。
//
// **同族的另两条 impl 不在此列**：`From<String> for AppError` 与 `From<&str>`
// 已在计划 13 的 T2 删除（见上），`From<tauri::Error>` 已在计划 13 的 T2b 删除（见上）。
// 删掉的是这一条：
//
//     impl From<AppError> for String {
//         fn from(err: AppError) -> Self {
//             err.to_string()
//         }
//     }

/// 测试用的日志捕获，**只被诊断站点的行为测试使用**（`#[cfg(test)]`）。
///
/// 它存在的理由：`diagnostic()` 的效果**只体现在日志文本上**，没有它就没法把「这个站点
/// 打印的是 code 还是 code+params」钉住 —— 把某站点的 `e.diagnostic()` 改回 `e`
/// 会静默编译、闸门绿、全套测试绿（实测）。
///
/// 用 `tracing::subscriber::with_default`（**线程局部**）而不是全局 `init`：
/// `cargo test` 并行跑测试，全局订阅者会被先跑的那个测试抢注、后面的一律拿不到。
#[cfg(test)]
pub(crate) mod log_capture {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// 在 WARN 级、无 ANSI 的订阅者下跑 `body`，返回捕获到的日志文本。
    pub(crate) fn capture(body: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).expect("日志必须是合法 UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ① 结构化错误：`code` 与 `params` 以对象形状跨 IPC。
    #[test]
    fn coded_serializes_as_code_and_params() {
        let err = AppError::coded("a.b").with("p", "v");
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"code":"a.b","params":{"p":"v"}}"#
        );
    }

    /// ② R1 的回归守卫：`Business` 必须仍是裸字符串，否则前端的
    /// `AppErrorPayload = string | CodedError` 联合即刻作废（`Business` 变体本身仍在，
    /// 生产构造站点已归零，只剩 7 处 `AppError::business` 夹具与 `streaming.rs` 那 1 处
    /// 直接构造 `Business` 的夹具产出它）。
    #[test]
    fn business_still_serializes_as_bare_string() {
        assert_eq!(
            serde_json::to_string(&AppError::business("中文")).unwrap(),
            r#""中文""#
        );
    }

    /// ③ 非 `Coded` 变体上 `with()` 无操作：既不 panic，也不改变原值。
    #[test]
    fn with_on_non_coded_variant_is_a_noop() {
        let err = AppError::business("原样").with("p", "v");
        assert_eq!(serde_json::to_string(&err).unwrap(), r#""原样""#);
    }

    /// ④ 无 params 时字段是空对象而非缺失（前端 `err.params ?? {}` 两种都吃，此处定死一种）。
    #[test]
    fn coded_without_params_keeps_empty_params_object() {
        assert_eq!(
            serde_json::to_string(&AppError::coded("a.b")).unwrap(),
            r#"{"code":"a.b","params":{}}"#
        );
    }

    /// ★ `diagnostic()` 的判据：`Coded` 带出 code **与** 全部 params，`Display` 一个都不动。
    ///
    /// 这一条同时钉住两件事：① 站点改用 `diagnostic()` 后 `detail` 回到了日志里；
    /// ② `Display` 仍然是裸 code（若谁把 `diagnostic()` 的实现在 `Display` 上「顺手统一」，
    /// 本测试的 `Display` 断言会红）。
    #[test]
    fn diagnostic_carries_code_and_params_without_touching_display() {
        let err = AppError::coded("internal.io").with("detail", "No such file or directory (os error 2)");
        assert_eq!(
            err.diagnostic(),
            "internal.io detail=No such file or directory (os error 2)"
        );
        // 键序按 `BTreeMap` 的字典序，与序列化口径一致。
        let multi = AppError::coded("pty.open_failed")
            .with("session", "s-1")
            .with("detail", "boom");
        assert_eq!(multi.diagnostic(), "pty.open_failed detail=boom session=s-1");
        assert_eq!(multi.to_string(), "pty.open_failed");
    }

    /// `detail` 原样透出：不截断、不转义（空格 / 引号 / 换行都在），无 params 时与 `Display` 相同。
    #[test]
    fn diagnostic_does_not_escape_detail() {
        let raw = "line1\n\"quoted\"  spaced";
        let err = AppError::coded("internal.io").with("detail", raw);
        assert!(err.diagnostic().ends_with(raw), "detail 必须原样出现在 diagnostic 里");

        assert_eq!(AppError::coded("a.b").diagnostic(), "a.b");
        assert_eq!(
            AppError::coded("a.b").diagnostic(),
            AppError::coded("a.b").to_string()
        );
    }

    /// 非 `Coded` 变体沿用 `Display`（`Business` 本来就带可读文本，不是裸 code）。
    #[test]
    fn diagnostic_of_non_coded_variant_is_display() {
        let err = AppError::business("中文原文");
        assert_eq!(err.diagnostic(), err.to_string());
        assert_eq!(err.diagnostic(), "中文原文");
    }

    /// `params` 的键序稳定（`BTreeMap` 按字典序），否则同一份错误会有多种序列化结果。
    #[test]
    fn coded_params_serialize_in_stable_key_order() {
        let err = AppError::coded("a.b").with("z", "1").with("a", "2");
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"code":"a.b","params":{"a":"2","z":"1"}}"#
        );
    }

    /// 连接必然失败的 manager：`r2d2::Error` 的构造器是私有的，只能经由一个建不起来的
    /// `Pool` 拿到真实实例。`connection_timeout` 压到 50ms，否则 `build` 会等满默认的 30 秒。
    #[derive(Debug)]
    struct FailingManager;

    impl r2d2::ManageConnection for FailingManager {
        type Connection = ();
        type Error = std::io::Error;

        fn connect(&self) -> Result<(), std::io::Error> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "连接被拒绝"))
        }

        fn is_valid(&self, _: &mut ()) -> Result<(), std::io::Error> {
            Ok(())
        }

        fn has_broken(&self, _: &mut ()) -> bool {
            false
        }
    }

    /// ★ R4：五类基础设施错误**全部**走 `{ code, params }`，底层文本原样进 `detail`。
    ///
    /// 这五条原本是 `#[error("…")]` 的中文 `Display`，经字符串序列化与每条 `to_string()`
    /// 路径对**每个**变体生效——用户看到的就是那五句中文。改完之后：
    /// - 线格式是 `{ code, params }`，前端按 `errors.internal.*` 取四语文案；
    /// - `Display` 是 code 本身（经 `Coded` 的 `#[error("{code}")]`），不再产出中文；
    /// - 底层英文 / OS 文本不翻译，只作为 `detail` 透出。
    #[test]
    fn infrastructure_errors_become_coded_with_detail() {
        let pool_err = r2d2::Pool::builder()
            .connection_timeout(std::time::Duration::from_millis(50))
            .max_size(1)
            .build(FailingManager)
            .expect_err("连接必然失败，build 必须返回 Err");
        let json_err = serde_json::from_str::<i32>("not json").expect_err("必须解析失败");

        let cases: Vec<(&str, AppError)> = vec![
            ("internal.database", rusqlite::Error::InvalidQuery.into()),
            ("internal.pool", pool_err.into()),
            (
                "internal.io",
                std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into(),
            ),
            ("internal.json", json_err.into()),
            ("internal.tantivy", tantivy::TantivyError::IndexAlreadyExists.into()),
        ];

        for (code, err) in cases {
            let value = serde_json::to_value(&err).unwrap();
            assert_eq!(value["code"], code, "{code} 的线格式 code 不对");
            assert!(
                value["params"]["detail"].as_str().is_some_and(|d| !d.is_empty()),
                "{code} 的 params.detail 必须是非空的底层文本"
            );
            // `Display` 是 code，不再是中文——字符串序列化与 `to_string()` 路径拿到的就是它。
            assert_eq!(err.to_string(), code, "{code} 的 Display 必须是 code 本身");
        }
    }
}
