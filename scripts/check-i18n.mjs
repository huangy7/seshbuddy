#!/usr/bin/env node
// i18n 闸门：阻止新增硬编码文案、阻止语言包缺口。
// 后端错误码与 errors 命名空间的条目比对是规则 10（见 main()）：`coded()` 的 code 与
// `errors.*` 双向闭合——码表在前端语言包里，Rust 侧穷举不出来，只有这条规则能守住它。
// 待迁移白名单只减不增——每批迁移完成后划掉对应路径，杜绝「迁了一半又写新的硬编码」。
import { existsSync, readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
// 规则 7 的 AST 遍历用它。typescript 是本仓的直接依赖（vue-tsc 也用它）。
// 裸标识符按**本文件所在位置**解析，故闸门被复制到别处运行时必须能就地解析到它——
// 测试夹具因此把 node_modules 一并供给（见 check-i18n.test.mjs 的 buildFixture）。
import ts from "typescript";
// 规则 8 的模板 AST 遍历用它。与 find-english-blind.mjs 用的是同一个解析器、同一个节点口径，
// 而规则 8 的允许清单正是从那个脚本产出的候选裁定出来的——两边口径必须一致，
// 否则清单里的条目会与闸门实际取到的字符串对不上，逐条落空。裸标识符的解析同上。
import { parse as parseSfc } from "@vue/compiler-sfc";

// 被 vitest 转译后 import.meta.url 不再是 file: URL，模块顶层解析会直接抛错，
// 而测试只导入纯函数。根目录只在 main() 里用得到，解析推迟到那里。
let ROOT;
/**
 * 「中文文案」的字符范围：汉字 + 中日韩标点 + 全角形式。
 *
 * 只认汉字会漏掉整类硬编码：`join("、")` 里一个汉字都没有，闸门对它一声不响，
 * 而它在英文界面上就是 `Claude、Gemini` 这种可见缺陷。U+3000–U+303F 是、。「」『』，
 * U+FF00–U+FFEF 是全角形式：（）等。汉字的 U+4E00–U+9FFF 行为不变，
 * 变的只是「什么算中文文本」，规则拿它做什么没变。
 */
const CJK_RANGES = "\\u3000-\\u303F\\u4E00-\\u9FFF\\uFF00-\\uFFEF";
const CJK = new RegExp(`[${CJK_RANGES}]`);
/** 模板文本与 HTML 正文按空白切出的整块 token，与 CJK 共用同一字符范围。 */
const CJK_TOKEN = new RegExp(`[^\\s]*[${CJK_RANGES}][^\\s]*`, "g");

/**
 * 尚未迁移的路径。每批完成后删除对应项，只减不增。
 *
 * **计划 7 把它清空了**：最后一条 `src/composables/usePricingCatalog.ts` 随「厂商身份与显示
 * 拆分」一起移除——该文件里那六个含中文的厂商名不是待翻译文案，而是同时被当作去重键与过滤键的
 * 身份值，故它们搬进了语言包（`dialogs.pricing.provider.<slug>`，品牌名四语同值、刻意不翻），
 * 代码里只留 id。规则 1 与规则 7 自此对 src/ 下全部前端文件生效，没有跳过的文件。
 *
 * **本数组从此刻起应保持为空**：往里加条目就是给新写的硬编码文案开口子，
 * 而闸门对它的全部抓力正是「没有豁免」。
 */
const FRONTEND_PENDING = [
  // 批次 2（API）：api-profile/ 与 api-log/ 已全部迁完，两条目录不再有白名单条目。
  // 批次 3（助手与对话）起按文件列举而非整目录——目录条目会连已无中文的文件一起遮住，
  // 那些文件现在就该受闸门约束。
  // 批次 5（逻辑层）把 src/composables/ 与 src/utils/ 这两条目录条目也换成了逐文件列举：
  // 它们曾把两个目录下的文件全部遮住，其中一批已无 CJK 的文件因此从未被规则 1 检查过；
  // 换成逐文件列举后，那些文件立即受约束。这也是后续任务能逐条移除的前提——
  // 目录条目下没有可移除的逐文件条目。
  // 全表只列文件，不得再新增目录条目：删掉某行即对该文件重新启用规则 1，那是它迁移完成的标志。
];
/**
 * 尚未迁移的后端文件。每批完成后删除对应项，只减不增。
 *
 * **计划 8 的 T1 把 `"src-tauri/src/"` 这条目录条目换成了逐文件条目**，理由与计划 5 对
 * 前端做过的那次相同（见 FRONTEND_PENDING 的说明）：目录条目会连已无中文的文件一起遮住，
 * 那些文件现在就该受规则 2 约束。**全表只列文件，不得再新增目录条目**——
 * 删掉某行即对该文件重新启用规则 2，那是它迁移完成的标志。
 *
 * 条目 = **实测含生产 CJK 的文件**（用修好切分与取串之后的闸门重跑得出）。
 * **当前条数以 `node scripts/check-i18n.mjs` 打印的后端读数为准**（B1 起每批都在下方追加
 * 「本表因此从 N 条降到 M 条」的小结，下面那段 35 / 33 是 B1 之前的快照，**不要拿它当现值**）。
 * **文件一旦零 CJK，条目即刻删除**——留着它等于把已迁完的文件重新遮住，规则 2 看不见其中
 * 新写的中文，正是 T1 修掉的那类缺陷的镜像。计划 8 就是这样删掉 `src-tauri/src/error.rs`
 * （T4 迁走其中的 CJK `#[error]` 分支）与 `src-tauri/src/streaming.rs`（T3 替换了那处
 * panic 文案）两条的：两份文件的生产 CJK 实测都是 0，只剩 `#[cfg(test)]` 里的。
 * 目录条目下的零 CJK 文件**不列**：它们从此刻起受规则 2 约束。
 * （**B1 当时实测**：`src-tauri/src` 下 67 个 .rs，其中 35 个含生产 CJK、32 个零 CJK；
 * 35 条里 `commands/profile.rs`、`cli.rs`、`context_menu.rs` 走具名例外表（见下），
 * 故当时本表 = 32 + `src-tauri-proxy/src/proxy.rs` 一条 = 33 条。
 * 同一个测法在零的那一侧给出 32、在非零的那一侧给出 35，故不是「扫描器没扫到」的假零。
 * 计划 8 的最终评审发现这里写的 27 是**删两条之前的数**：error.rs 与 streaming.rs 从表里删掉后
 * 变成零 CJK 文件，零的个数随之 27 → 29，本行在 T1 里订正。
 * **批次 6 的 B1 又删掉三条**（`cli_config.rs`、`commands/mod.rs`、`db/profiles.rs`，
 * 迁完共享字面量后实测零生产 CJK），非零个数 38 → 35、零的个数 29 → 32；
 * 同批把 `commands/profile.rs`（还有 4 条生产 CJK，都不是本计划的迁移目标）移进
 * `RUST_PENDING_EXCEPTIONS`，本表因此从 39 条降到 35 条。）
 * **批次 6 的 B3 又删掉两条**（`commands/proxy.rs` 迁完实测零生产 CJK；`proxy.rs` 迁完只剩
 * 3 条数据类展示名 / 分组标签，移进 `RUST_PENDING_EXCEPTIONS`），本表因此从 33 条降到 31 条。
 * **批次 6 的 B4 再删掉六条**，本表因此从 31 条降到 25 条：
 * - `session_index_watcher.rs`：3 个错误站点迁完，**生产 CJK 实测 0**，条目直接删除
 *   （它没有任何遗留义务，不进例外表）；
 * - `commands/session.rs`（剩 10 条）、`commands/session_index/mod.rs`（剩 2 条）、
 *   `commands/session_index/scan.rs`（剩 3 条）、`session.rs`（剩 4 条）、
 *   `commands/session_index/tests.rs`（剩 31 条，整份是测试代码）：残留都不是本计划的
 *   迁移目标，移进 `RUST_PENDING_EXCEPTIONS`，理由逐条写在各自条目上方。
 * 其中两个另有 phase 3 的义务但规则 2 看不见（只认 CJK 码点），在计划 8 的 T1 报告里具名：
 * `commands/pty.rs:89` 与 `commands/updater.rs:57` 都是
 * `.map_err(|e| AppError::business(e.to_string()))`——把底层错误的字符串原样透传给用户，
 * 而底层错误是英文/OS 文案，规则 2 对它一声不响。
 * 反过来，`commands/session_index/tests.rs`（31 条）与 `parser/parse_bench.rs`（1 条）**整份是
 * 测试代码**（由别处的 `#[cfg(test)] mod X;` 引入，文件自身没有标记），却被算成生产 CJK；
 * 它们留在表里是保守取值，但**不是 phase 3 的迁移目标**。
 * （**收口订正**：这两条**留在 `RUST_PENDING_EXCEPTIONS` 里**，并被表头明确标注为**假阳性**
 * ——它们没有义务，但那是它们唯一的具名去处。收口时试过让闸门自动认出这类文件、把表降到 18 条，
 * **已回退**：判据被五个可编译的反例打穿，理由是拿人写的错标换闸门里静默的洞不划算。
 * 详见 `RUST_PENDING_EXCEPTIONS` 表头那段。）
 *
 * **批次 6 的 B5 再减五条**，本表因此从 25 条降到 **20** 条（25 − 4 删条目 − 1 移例外表）：
 * `git.rs`（28 条）、`commands/workspace_fs.rs`（7 条）、`db/mod.rs`（1 条）、
 * `db/tantivy_search.rs`（2 条）四条迁完错误站点后**生产 CJK 实测 0**，条目直接删除（−4）；
 * 同批的 `db/session_archive.rs`（4 条：3 个错误站点 + `:525-526` 的 SQL 正文）迁完剩 1 条
 * 数据类，**移进 `RUST_PENDING_EXCEPTIONS`**（−1），理由写在该条目上方。
 * **两个数都要数**：删条目与移例外表都让本表少一条，只减前 4 条会得到 21，而闸门打印的是 20。
 *
 * **批次 6 的 B6 再减五条**，本表因此从 20 条降到 **15** 条（20 − 4 删条目 − 1 移例外表）：
 * `assistant/agent.rs`（4 条）、`assistant/conversations.rs`（2 条）、
 * `assistant/quick_phrases.rs`（3 条）、`assistant/backfill.rs`（1 条）四条迁完错误站点后
 * **生产 CJK 实测 0**，条目直接删除（−4）；同批的 `assistant/commands.rs`
 * （13 条：12 个错误站点 + `:113` 的会话标题兜底值）迁完剩 1 条数据类，
 * **移进 `RUST_PENDING_EXCEPTIONS`**（−1），理由写在该条目上方。
 * 本批的 `conversations.rs` 只报 2 条而不是侦察时的 3 条：第 3 条 `"无法获取用户目录"`
 * 已由 B2 迁成 `cli.home_dir_missing`（复用 T3 的码，未新造），本次是**只读复核**。
 * 故本批实际迁 **22** 个错误站点，不是派发里的 23——差的那一条就是它。
 *
 * **批次 6 的 B7 再减六条**，本表因此从 15 条降到 **9** 条，六条**全部移进
 * `RUST_PENDING_EXCEPTIONS`**（0 删条目 − 6 移例外表）：`parser/claude.rs`（5 个错误站点迁完
 * 剩 1 条 `expect` 文案）、`parser/antigravity.rs`（4 条：1 处截断标记 + 3 条比较哨兵）、
 * `parser/claude_entry.rs`（1 条截断标记）、`parser/gemini.rs`（1 条截断标记）、
 * `parser/workbuddy.rs`（1 条截断标记）、`parser/parse_bench.rs`（1 条 `#[ignore]` 属性，
 * 整份是测试代码）。理由逐条写在各自条目上方。
 *
 * **批次 6 的 B8（最后一批）把本表清空**：9 → **0**（6 删条目 + 3 移例外表）。
 * 6 条迁完错误站点后**生产 CJK 实测 0**，条目直接删除。迁掉的量按**唯一文案**计
 * （合计 19 条唯一 / 21 处出现）：`pty_manager.rs` 9 条唯一 / 11 处（`"会话不存在: {}"`
 * 出现 3 次）、`pricing.rs` 5、`commands/settings.rs` 2、`claude_hooks.rs` 1、
 * `commands/model_list.rs` 1、`commands/system_ops.rs` 1；
 * 3 条只剩**非 CJK 义务**，移进 `RUST_PENDING_EXCEPTIONS` 并逐条写明理由
 * （`updater.rs` 2 条数据类、`native_text.rs` 1 条 `panic!` 文案、
 * `src-tauri-proxy/src/proxy.rs` 2 次 / 1 条唯一的数据类正文标记）。
 * `commands/mod.rs` 不在本批的迁移目标里：它实测 0 条生产 CJK，那条
 * `cli_config.app_data_dir_missing` 已由 T2 迁完，**不要**为它写条目。
 *
 * **本数组从此刻起应保持为空**（与 `FRONTEND_PENDING` 的说明同）：规则 2 自 B8 起对三个扫描根
 * 下的全部 .rs 生效，没有跳过的文件；往里加条目就是给新写的硬编码文案开口子，
 * 而闸门对它的全部抓力正是「没有豁免」。**空数组是预期状态，不是待办。**
 */
const RUST_PENDING = [];

/**
 * 具名例外表：**有非 CJK 义务**的文件，规则 2 与棘轮都放行。形态同 RUST_UNTRANSLATED
 * （只列文件 + 逐项写明理由），条目另带 `residual` 供残量棘轮校验，不是另一种机制。
 *
 * **为什么需要它**：棘轮（见 main()）要求 RUST_PENDING 的每个条目的文件仍有生产 CJK，
 * 否则报出——它守的是「文件已迁完、条目却留着，规则 2 从此看不见其中新写的中文」。
 * 但有一类文件没有可迁的 CJK 也**不该**被逼着删条目：
 * - **数据类文案仍占着的文件**（R4）：返回值不是错误的那些文案，修法不同，归计划 10。
 *   （表里还有几条不属于 R4 的同类——`panic!`/`expect` 文案、比较哨兵、协议标记、SQL 注释，
 *   各自条目上方写明是哪一类。）
 * 这类文件需要一个**具名**的去处：写在注释里闸门读不到，混在白名单里则与「待迁移的界面文案」
 * 分不开，白名单就再也说不清自己为什么还有条目。
 *
 * **本表的构成（计划 10 收口实测，基线 `6d0fb1b0`）**：**10 条 = 8 条真实义务 + 2 条「仅测试
 * 文件」假阳性**，读者不必自己数。真实义务的残生产 CJK 合计 **13 条**，按**不能改的理由**
 * 分三类（一个文件只落一类，故各类之和即总数）：
 * - **E 非 UI 载体 4 条 / 3 个文件**：`cli.rs`（1，Windows 批处理脚本正文）、
 *   `db/session_archive.rs`（1，SQL 注释）、`commands/session.rs`（2，写进 WorkBuddy 自己那个
 *   库的 SQL 字面量）。三条都**没有一条路径到 SeshBuddy 渲染的界面**，各自条目上方写明
 *   「谁执行它、输出去了哪里」。
 * - **F 文本被解析承重 5 条 / 2 个文件**：`session.rs`（2，`[图表]` / `[图表: {}]`，被**前端**
 *   `src/composables/useMessageTurns.ts:198/334` 的正则解析）、`parser/antigravity.rs`
 *   （3，`你正在…` 哨兵，被**后端**的 `starts_with` 解析）。
 * - **G `panic!`/`expect` 4 条 / 3 个文件**：`commands/profile.rs`（2）、`parser/claude.rs`（1）、
 *   `native_text.rs`（1）。都是内部不变量或构建期告警，**不面向用户**。
 *
 * **2 条假阳性**：`commands/session_index/tests.rs`（**42 条**）与 `parser/parse_bench.rs`（1 条）。
 * 两者**整份是测试代码**（由别处的 `#[cfg(test)] mod X;` 引入、文件自身一个 `cfg(test)` 标记都
 * 没有），其中的中文是夹具，**没有义务可履行**。它们不是无名豁免——表头在这里替它们说清身份，
 * 免得读者拿「每条都有义务」去套它们。**终态残留的生产 CJK 因此有两个口径**：逐文件跑探测器
 * 得 **56 条**（含这两条的 42 + 1），**诚实数字是 13 条 / 8 个文件**——报 56 会把剩余债务
 * 夸大约 4 倍。差的那 43 条全部来自这两条假阳性。
 *
 * **⚠️ 这两条为什么是「人写进表里」而不是「让闸门自动认出来」——试过，已回退，别再试一遍。**
 * 收口时给规则 2 加过一套 `findTestOnlyModules()`（**已回退，函数本身也已删除**——下面提到它
 * 只是为了记下为什么不能这么做，不要再去 grep 它）：扫一遍 `#[cfg(test)]\s*mod\s+(\w+)\s*;`，
 * 按 `<dir>/<stem>.rs` 与 `<dir>/<stem>/mod.rs` 解析路径，把这些文件从规则 2 与棘轮里跳过。
 * 它当场就把本表从 20 降到 18，随后被评审用可编译的反例打穿。**反例要分两批读，别混成
 * 一句「判据不成立」**——两批的根因不同，只有第二批是回退的理由：
 * - **①② 被 fix round 1 修好了**（判据从「存在一条测试声明」收成「**全体声明一致**」），
 *   **它们不构成回退的理由**：
 *   ① `#[cfg(test)] mod x;` 与 `#[cfg(not(test))] mod x;` 并列：该文件在生产构建里也是模块；
 *   ② 同一路径被两个 crate 根分别声明：`main.rs` 说它是测试模块，`lib.rs` 说它是生产模块。
 * - **③④⑤ 打穿了修好之后的判据**（**这三条才是回退的理由**，它们与下面那条根因同源）：
 *   ③ `#[path = "…"]` 的声明把文件编译进生产，而另一条 `#[cfg(test)]` 声明指向**同一路径**；
 *   ④ 内联模块（`lib.rs` 里 `mod a { mod x; }`）加一个**无人声明的孤儿 `a.rs`**（内含
 *      `#[cfg(test)] mod x;`），使 `a/x.rs` 看起来是测试模块——它其实被生产构建编进去；
 *   ⑤ 函数体里的 `#[path]` 声明（`fn f() { #[path = "x.rs"] mod x; }`）同理。
 * ③④⑤ 的根因只有一条：**闸门解析「声明 → 路径」用的是启发式而不是 rustc 的规则，且从不检查
 * 声明方文件自己可不可达**；解析不出的声明被静默丢掉，于是另一条指向同一路径的 `#[cfg(test)]`
 * 声明就足以让一个**生产**文件被跳过、而闸门照常报「通过」。要让它真的成立就得建**模块图**
 * （内联模块、`#[path]`、从 crate 根出发的可达性）——那不是收口任务该做的事。
 * ⚠️ **两批要一起读**：只验 ①② 会发现它们**已被修好**，从而误以为 ③④⑤ 也站不住——
 * **真正支撑回退的是 ③④⑤**。它们今天在三个扫描根里都是 **0 处**，但「今天 0 处」不是判据：
 * 判据是「这条判据可不可证伪」，而它在 ③④⑤ 这三种**合法输入**上已经可证伪。
 *
 * **取舍是刻意的，不是妥协**：本表里的一条错标是**人写的、可评审的记录**——有人写下路径与
 * 理由，复核者读得到；自动判据是**无人复核的派生结果**，判错时除非有人专门跑探针，否则看不见。
 * 拿一条看得见的错标去换一个**在验证工具里静默的洞**，对「少两条假阳性」这点收益来说不划算。
 * （**只报不跳过**的版本是安全的——把派生集合当提示打印、由人去写条目。没做：那是一条没有
 * 强制力的新机制，而本表「具名条目」这条正路已经够用。）
 *
 * **两处放行**：① 棘轮跳过这里的文件（零 CJK 也允许留在 RUST_PENDING）；
 * ② 规则 2 跳过这里的文件（与 RUST_UNTRANSLATED 同一处放行），故文件可以**移出白名单**而它的
 * 中文不会被报成硬编码——白名单因此能诚实地清零，而不是靠一条永远删不掉的条目撑着。
 * 代价与 RUST_UNTRANSLATED 相同：写进这里的文件，其新增中文对规则 2 不可见。
 * 故**只收有具名理由的文件**，理由要写明属哪一类；没有理由的条目就是无名豁免。
 *
 * **本表有棘轮，判据是「该文件的生产 CJK 与条目里的 `residual` 相符」**：`residual` 是机器读得到
 * 的字段（原先只是散文里的一个数），main() 逐条拿 `findCjkRustProductionLiterals` 实测该文件并
 * 比对，不符即失败。写进表的文件对规则 2 仍然不可见（那是 ② 那处放行的代价），但残量不再是
 * 无人校验的声明：文件里再长出**生产**中文会让实测值偏离 `residual`，棘轮因此会响——除非有人
 * 把 `residual` 同步改大，而那是一次显式的、可评审的改表动作。
 * **这条棘轮是补上的，起因是计划 10 暴露的两个具体实例**：`src-tauri/src/context_menu.rs`（T2 归零）与
 * `src-tauri/src/proxy.rs`（T3 归零）两条条目，其文件的生产 CJK 早在各自那一批就归零，条目却
 * 带着过期的「只剩 4 条 / 只剩 3 条」文字跨了其后好几个任务——闸门全程一声不响，直到收口任务
 * 逐文件重测才发现。**这就是当时没有棘轮的代价：过期条目不会自己浮出来，只能靠人重测。**
 * main() 另有一条守卫排在棘轮之前：它要求每个条目是一个**存在的文件**（挡目录条目与写错的路径）。
 * 棘轮在这一类条目上会**抛异常**而不是报出——`readFileSync` 对目录抛 EISDIR、对不存在的路径抛
 * ENOENT，两者都会中断整轮诊断、让同一轮里其它文件的违规全部消失——故守卫命中即报出并跳过该
 * 条目的棘轮检查。
 *
 * **⚠️ 表在变大 ≠ 债务在变大**：条目增长恰恰是**最后几个文件只剩非 CJK 义务**的证据——
 * 一个文件把错误文案迁完（可迁的部分归零）之后，若还剩数据类残留，它就从 `RUST_PENDING`
 * 挪进本表。所以本表变长与白名单变短是同一件事的两面，不是「豁免越来越多」。
 * 反过来，**靠删条目把数字压下来是禁止的**：每个数据类条目都替它的文件承担着规则 2 的失明，
 * 删掉而不迁走底层文案，只会把它从「债务」变成「规则 2 报错」。
 *
 * **全表只列文件，不得新增目录条目**（与 RUST_PENDING 同一条禁令），且命中按**整条路径精确
 * 比对**、不做前缀匹配（见 isExempted）：前缀匹配下一条目录条目会同时遮蔽整棵子树——规则 2 与
 * 棘轮一起失效。目录条目与失效条目由 main() 那条守卫报出：它要求每个条目是一个存在的**文件**，
 * 故它接住目录条目、写错的路径与被改名的文件；残量棘轮对这一类条目无能为力——它的判据是
 * 「条目里声称的残量与实测相符」，对着不存在的路径只会抛异常。**若少了那条守卫，这类条目不是
 * 被静默放行，而是让整轮诊断当场崩掉**：实测目录条目抛未捕获的 `EISDIR`、不存在的路径抛
 * `ENOENT`，都在棘轮的 readFileSync 上，都 exit 1 且打不出一条失败信息。
 *
 * **批次 6 的 B1 起不再为空**：`commands/profile.rs` 的生产 CJK 迁到只剩 4 条，而那 4 条
 * 都不是本计划的迁移目标，故它移出 `RUST_PENDING` 后落在这里。理由逐条写在条目上方。
 * **同批的 B2 又加两条**：`cli.rs`（只剩 1 条批处理脚本文案）与 `context_menu.rs`（只剩 4 条
 * 成功提示）——两者迁完错误站点后，残留的都是 R4 的数据类，同样不是本计划的迁移目标。
 * **B3 再加一条**：`proxy.rs`（只剩 3 条数据类展示名 / 分组标签，逐条点名见该条目）。
 * **B4 再加五条**：`commands/session.rs`（10 条）、`session.rs`（4 条）、
 * `commands/session_index/mod.rs`（2 条）、`commands/session_index/scan.rs`（3 条）四条迁完
 * 错误站点后只剩 R4 的数据类与 R5 的协议标记；`commands/session_index/tests.rs`（31 条）
 * 整份是测试代码。逐条点名见各自条目。
 * **B5 再加一条**：`db/session_archive.rs`（3 个错误站点迁完，剩 1 条数据类）。
 * **B6 再加一条**：`assistant/commands.rs`（12 个错误站点迁完，剩 1 条数据类）。
 * **B7 再加六条**：`parser/claude.rs`（5 个错误站点迁完，剩 1 条 `expect` 文案）、
 * `parser/antigravity.rs`（1 条截断标记 + 3 条比较哨兵）、`parser/claude_entry.rs`、
 * `parser/gemini.rs`、`parser/workbuddy.rs`（各 1 条截断标记）、
 * `parser/parse_bench.rs`（1 条 `#[ignore]` 属性，整份是测试代码）。逐条点名见各自条目。
 * **B8 再加三条**：`updater.rs`（2 条数据类）、`native_text.rs`（1 条 `panic!` 文案）、
 * `src-tauri-proxy/src/proxy.rs`（2 次 / 1 条唯一的数据类正文标记）。逐条点名见各自条目。
 * 本表因此 17 → **20** 条，`RUST_PENDING` 同时清零。
 * 空数组仍是**预期状态**，不是待办。
 *
 * **收口（计划 9 的 T11）的终态仍是 20 条**：`commands/session_index/tests.rs`（31 条）与
 * `parser/parse_bench.rs`（1 条）整份是测试代码，由别处的 `#[cfg(test)] mod X;` 引入、文件自身
 * 没有标记，于是被探测器算成生产 CJK。**它们没有义务可履行**，但本表是它们**唯一**的具名去处：
 * 删掉条目规则 2 会立刻把夹具判成硬编码（它们**不能靠迁移离开**，没有可迁的文案），
 * 而自动认出它们的尝试已被回退（理由见上面那段）。所以它们**留在这里，并且被明确标注为
 * 假阳性**——登记一个假阳性，好过让表头说谎，也好过在闸门里留一个静默的洞。
 * `findCjkRustProductionLiterals` 一个字没改：它仍然看得见这两个文件里的中文（实测 `tests.rs`
 * 31 条、`parse_bench.rs` 1 条），变的只是本表替它们承担了规则 2 的失明。
 *
 * **计划 10 的收口（T9）删掉 10 条归零条目**：本计划把「有渲染路径」的数据类文案（30 条）与
 * H 类死数据（2 条）处理完之后，在基线 `6d0fb1b0` 上逐文件重跑探测器，下列 10 个文件的生产
 * CJK 实测归零，**条目连同它们的注释块整体删除**——`context_menu.rs`、`src-tauri/src/proxy.rs`、
 * `commands/session_index/mod.rs`、`commands/session_index/scan.rs`、`assistant/commands.rs`、
 * `parser/claude_entry.rs`、`parser/gemini.rs`、`parser/workbuddy.rs`、`updater.rs`、
 * `src-tauri-proxy/src/proxy.rs`。**闸门对这一类删除是瞎的**（例外表是纯成员判定；残量棘轮只查
 * 「在表的条目声称的数与实测相符」，不查「某条条目该不该在」），所以这 10 条不是「闸门要求删」
 * 而是**逐文件重测后主动删**——
 * 留着它们等于让规则 2 永久看不见这 10 个文件里新写的中文。删完本表 **20 → 10 条**，
 * 真实义务残量 **46 → 13 条**。
 * ⚠️ **本节里出现的批次条数（含上面那段收口说明与下面的 B1–B8 批次日志）都是各批次当时的
 * 实测值，不是现值**，现值只看本节开头的构成说明。其中 `tests.rs` 的 31 → **42** 是计划 10 的
 * T6 往该文件加索引用例带来的（多重集差分、无删除），凡写 31 的地方都按 42 读。
 */
const RUST_PENDING_EXCEPTIONS = [
  // `commands/profile.rs`：**残 2 条，全属 G 类（`panic!`/`expect`）**，行号在 `6d0fb1b0` 上量得：
  // `:796` 与 `:809` 的 `.expect("对象已初始化")`，两处都在 `set_json_pointer_value`（`:785`）里，
  // 且都**紧跟同一函数里的 `ensure_json_object(current)`**——后者把 `current` 保证成 object，
  // 故 `as_object_mut()` 必为 `Some`。这条 `expect` 是**内部不变量**，不因用户操作触发。
  // 证据：触发即 panic、进程直接死，没有任何渲染路径；计划 9 的 R5 把 `panic!`/`expect` 文案
  // 整体排除在迁移之外，登记为具名义务。
  // 本文件此前的 2 条 R4 数据类（`"全局默认"`、`"{} 个目录配置不一致"`）已由计划 10 的 T4 改成
  // `None` / 结构化值，故本条目现在只承担 G 类。
  { path: "src-tauri/src/commands/profile.rs", residual: 2 },
  // `cli.rs`：**残 1 条，属 E 类（非 UI 载体）**，行号在 `6d0fb1b0` 上量得：`:639`。
  // 那条字面量是 `build_windows_context_menu_script`（`:631`，`#[cfg(target_os = "windows")]`）
  // 返回的**整段 Windows 批处理脚本正文**，其中 `echo 未提供目录路径。` 是脚本里给用户看的一行。
  // **谁执行它**：`context_menu.rs:394` 的 `write_windows_context_helper` 把这段正文写进
  // `<data_dir>/com.seshbuddy.app/config/context-menu-claude.cmd`（路径见 `:383-389`），
  // 再由 `windows_registry_command`（`:401-408`）拼成注册表命令并注册（`:316`/`:323`）。
  // **输出去了哪里**：用户右键目录时 `cmd.exe` 以 `/k` 执行那个 `.cmd`；只有目录参数为空时才
  // 走到 `echo`，那行字**打进命令行窗口**，不进 SeshBuddy 的任何界面。
  // 判据：没有任何语言包路径能到达它——脚本是磁盘上的静态文件，由 `cmd.exe` 而非 Rust 渲染。
  { path: "src-tauri/src/cli.rs", residual: 1 },
  // `commands/session.rs`：**残 2 条，全属 E 类（非 UI 载体）**，行号在 `6d0fb1b0` 上量得：
  // `:418-437`（含 `:428` 的 `' (分支)'` 与 `:429` 的 `'新建分支会话'`）与 `:443-447`
  // （含 `:446` 的 `'新建分支会话'`）——两段都是 `sync_workbuddy_forked_session_to_db`
  // （`:406`）里的 `INSERT INTO sessions …` 正文。
  // **谁执行它**：该函数由 fork 路径在 `:519` 调用；`:517` 打开的是
  // `cli::data_dir(WorkBuddy).join("workbuddy.db")`，即 **WorkBuddy 自己的库**
  // （`~/.workbuddy/workbuddy.db`），不是 SeshBuddy 的库。
  // **输出去了哪里**：字面量写进那个库的 `sessions.title` 列——那是 **WorkBuddy 的**标题列，
  // 由 WorkBuddy 自己的界面渲染；SeshBuddy 从不读它：本仓对 `workbuddy.db` 的唯一读取是
  // `parser/workbuddy.rs:713` 的 `SELECT id, custom_title FROM sessions …`，只取 `custom_title`
  // （`:689` 的文档也写明「WorkBuddy 把 UI 自定义会话名写在 `sessions.custom_title`」）。
  // 故这三处中文**没有一条路径到 SeshBuddy 渲染的界面**；修法（若将来要做）是「标题另存语义」
  // 或「由 WorkBuddy 侧本地化」，与「把错误串换成码」是两件事。
  // 本文件其余 8 条残量（导出成功提示 3、`search_text` 前缀 4、`未知项目` 1）已由计划 10 的
  // T2/T6/T7 迁走。
  { path: "src-tauri/src/commands/session.rs", residual: 2 },
  // `session.rs`：**残 2 条，全属 F 类（文本被解析承重）**，行号在 `6d0fb1b0` 上量得：
  // `:283` 的 `"[图表]"` 与 `:285` 的 `format!("[图表: {}]", title)`，都在 `tool_use_summary`
  // （`:230`）的 `"show_widget"` 分支里。
  // **谁解析它**：前端 `src/composables/useMessageTurns.ts:198` 与 `:334` 的
  // `/^\[(?:图表|show_widget):\s*|\]$/g`——它从 `part.summary` 里剥出 widget 卡片的标题。
  // 改这两个字面量（例如本地化成 `[Chart: …]`），正则的第一个分支 `^\[(?:图表|show_widget):`
  // 就永远命中不了，只剩第二个分支 `\]$` 还能剥掉末尾那个 `]`，标题会渲染成 `[Chart: title`
  // 这种半截形态。
  // **判据是「文本被解析承重」，不是「有没有渲染路径」**：这两条**同时也被原样渲染**——
  // `src/components/chat/ChatToolGroup.vue:137` 的 `{{ item.summary || item.tool_name }}`，
  // 以及 `src/components/chat/ChatMessageBody.vue:109` 把 `part.summary` 交给
  // `renderToolUseSummary`（`src/composables/useChatSearch.ts:505-507`，无搜索词时直接
  // `escapeHtml(text)` 原样输出）。两处都显示后端造的 `summary`。所以「有渲染路径」**不足以**
  // 把它们划进「该改」那一类；**禁止改它们的理由是解析**：本地化会让解析结果随语言变，
  // 那是行为变更不是措辞变更。
  // ⚠️ **引证要求：这两条渲染路径都不能是 `tool_name` 过滤后的。** `TimelineView.vue:66` 虽然也
  // 显示 `part.summary`，但它落在 `:60-62` 的 `p.tool_name === "Agent"` 过滤里，`show_widget`
  // 的部件永远走不到那里，**不能**拿它当这两条的渲染证据（它支撑的是 Agent 调用那一行）。
  // 另有 2 条（`未知项目` 的兜底值与它的比较哨兵，原在 `resolve_project_path` 一带）已由计划 10
  // 的 T6 迁走，故本条目现在只承担 F 类。
  { path: "src-tauri/src/session.rs", residual: 2 },
  // `commands/session_index/tests.rs`：**整份是测试代码**（42 条生产 CJK 全是断言消息与夹具数据，
  // 如 `"已知 12 条全部缺失时应拒绝清理: {stale:?}"`），没有一条面向用户。它被算成生产文件，
  // 只是因为 `#[cfg(test)]` 标记在**引入方**（`mod.rs:1644` 的 `#[cfg(test)] mod tests;`），
  // 文件自身没有标记。
  // ⚠️ **这一条没有义务可履行，是登记的假阳性**——留着它是因为它**没有别的具名去处**：
  // 删掉条目，规则 2 立刻把它的夹具判成硬编码中文；它也**不能靠迁移离开**（没有可迁的文案）。
  // 收口时试过让闸门按 `#[cfg(test)] mod X;` 自动认出这类文件，**已回退**（判据被五个可编译的
  // 反例打穿，理由见本表表头）。`parser/parse_bench.rs` 形状相同，紧邻的条目同理。
  // 计数是 69：计划 10 的 T6 往本文件加索引用例时一并加了断言消息（31 → 42，行号/条数在
  // `6d0fb1b0` 上量得）；接入 OpenCode 时新增的扫描、可见性、检索候选与子会话用例又加了
  // 27 条断言消息（42 → 69）。两次都是多重集差分、无删除，全部是断言消息与夹具数据，
  // 没有一条面向用户。
  { path: "src-tauri/src/commands/session_index/tests.rs", residual: 69 },
  // `db/session_archive.rs`：**残 1 条，属 E 类（非 UI 载体）**，行号在 `6d0fb1b0` 上量得：
  // `:525-526` 的两行 `-- 自定义名以 …` 是 `list_archived_sessions_inner`（`:506`）里那段
  // `r#"…"#` SQL（`:511-530`）内部的**注释**。
  // **谁执行它**：这段正文由 `conn.prepare`（`:510`）交给 rusqlite → SQLite，在 SQLite 的
  // 词法分析阶段就被丢掉。**输出去了哪里**：语句的产出是 `query_map`（`:533`）读出的行
  // （session_path / project_path / display_name / …），注释对结果没有任何贡献，也从不回传。
  // 判据：没有任何语言包路径能到达它，用户也看不到它；改它要么让注释按语言生成、要么把这条
  // 复合键的说明搬出 SQL，与「把错误串换成码」是两件事。
  // 其余 3 个错误站点已全部迁成 code（`db.archive_pin_source_missing` /
  // `db.restore_target_exists` / `db.archive_content_missing`）。
  { path: "src-tauri/src/db/session_archive.rs", residual: 1 },
  // `parser/claude.rs`：**残 1 条，属 G 类（`panic!`/`expect`）**，行号在 `6d0fb1b0` 上量得：
  // `:686` 的 `.expect("并行解析块 panic")`——`std::thread::scope` 里对每个分块线程
  // `h.join()` 的结果取 `expect`。
  // 证据：`join` 只在**那个线程自己 panic 时**返回 `Err`（`parse_chunk_lines` 的签名是
  // `Result<_, String>`，可恢复的失败走 `Err` 而不是 panic），而线程 panic 是 bug 不是用户操作，
  // 故这条 `expect` 是**内部不变量**、不面向用户；计划 9 的 R5 把 `panic!`/`expect` 文案整体
  // 排除在迁移之外，登记为具名义务。
  // 5 个错误站点已全部迁成 code（`parser.file_open_failed` / `parser.file_seek_failed` /
  // `parser.file_read_failed` / `parser.offset_out_of_range` / `parser.tool_result_mismatch`）；
  // 本文件另有 1 处截断标记与多处 `#[cfg(test)]` 断言里的中文，**都在测试代码里**
  // （截断标记那处落在 `#[cfg(test)] parse_content` 项内），由
  // `findCjkRustProductionLiterals` 的切片排除，不计入生产 CJK。
  { path: "src-tauri/src/parser/claude.rs", residual: 1 },
  // `parser/antigravity.rs`：**残 3 条，全属 F 类（文本被解析承重）**，行号在 `6d0fb1b0` 上量得：
  // `:1447`/`:1448`/`:1449` 的 `clean.starts_with("你正在执行")` / `"你正在对"` / `"你正在将"`，
  // 在 `is_subagent_session`（`:1416`）里。
  // **谁解析它**：**后端自己**——`:1443` 把 `step.content` 过 `clean_user_content` 之后用
  // `starts_with` 判定，命中即把该会话判成子代理会话（`return true`，结果是它从会话列表里
  // 被隐藏）。它们扫的是**用户内容**，旁边并列着英文哨兵 `"You are implementing Task"` /
  // `"You are reviewing Task"` / `"You are the Final Code Reviewer"`（`:1444-1446`）。
  // 判据：这三条是**协议分类器的判据**而非界面文案——本地化它们会让分类结果随语言变
  // （同一个会话在中文环境下被隐藏、在英文环境下不隐藏），那是行为变更不是措辞变更。
  // 本文件此前的 1 条截断标记（D 类）已由计划 10 的 T8 迁走，故条目里不再有 D 类计数。
  { path: "src-tauri/src/parser/antigravity.rs", residual: 3 },
  // `parser/parse_bench.rs`：同上——**整份是测试代码**（由 `parser/mod.rs:10` 的
  // `#[cfg(test)] mod parse_bench;` 引入，文件自身没有标记），生产 CJK 只剩 1 条：
  // `:166` 的 `#[ignore = "手动运行：需设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>"]` 是基准测试的
  // 忽略原因，只在 `cargo test -- --ignored` 的输出里出现，不面向用户。**同样是登记的假阳性。**
  // （`cargo test` 默认不跑 `#[ignore]` 用例，那条原因串只在显式 `--ignored` 时打印；文件里另有
  // `eprintln!` 的中文，被闸门的日志宏剥离排除，不计入这 1 条。）
  { path: "src-tauri/src/parser/parse_bench.rs", residual: 1 },
  // `native_text.rs`：**残 1 条，属 G 类（`panic!`）**，行号在 `6d0fb1b0` 上量得：`:39` 的
  // `panic!("语言包 {namespace}/{lang} 不是合法 JSON: {e}")`——它挂在 `serde_json::from_str` 的
  // `unwrap_or_else` 上（`:38-39`），而 `raw` 是 `include_str!` 进来的**构建期语言包产物**。
  // 证据：解析失败只可能是打包错误，且在 `LazyLock` 首次查表时触发；触发即 panic、进程直接死，
  // 没有任何渲染路径，属打包错误而非用户操作。计划 9 的 R5 把 `panic!`/`expect` 文案整体排除在
  // 迁移之外，登记为具名义务。
  { path: "src-tauri/src/native_text.rs", residual: 1 },
];

/**
 * 刻意不翻译的文件：不是待迁移的欠债，是设计上就不面向用户，永远不会从这份名单消失。
 * 逐项写明理由，免得后来者误当成漏迁。
 */
const RUST_UNTRANSLATED = [
  // 助手工具的 --json-help 输出与状态码只由模型读取（agent.rs 的系统提示词就是让模型跑它），
  // 用户看不到，故保持中文。
  "src-tauri-proxy/src/assistant.rs",
];

// workspace 的三个成员都要扫：漏掉一个，那个 crate 里的新硬编码就永久不可见，
// 而输出里的覆盖比例仍宣称守住了「后端」。
const RUST_SCAN_ROOTS = ["src-tauri/src", "src-tauri-proxy/src", "transcript-store/src"];

/**
 * 刻意引用不存在 key 的探针，逐项写明理由。规则 5 要求引用的 key 必须存在，
 * 而验证「缺 key 时返回 key 本身」这条回退行为恰恰需要一个不存在的 key——
 * 那是测试装置，不是会渲染到界面上的错别字，故豁免。
 */
const MISSING_KEY_PROBES = [
  "__nonexistent__.key",
  // `useSessions.store.test.ts` 的「resolves every outcome key in every supported
  // language」拿它做**阳性对照**：证明「渲染结果 === 键名」这条判据确实认得出缺键，
  // 于是同一用例里对结果键的遍历不是恒真。取名挂在该用例自己的命名空间下，
  // 与真键不会混淆（`app.batch.exportOutcome.single` 等三个真键都在四份包里）。
  "app.batch.exportOutcome.__missing__",
];

/**
 * 规则 1 的具名误报豁免，逐项写明「是什么」与「为什么不是缺陷」。
 * 与 MISSING_KEY_PROBES 同类：不是待迁移的欠债，是工具边界的具名记录，不会随批次消失。
 *
 * 判据是**整行的原文**（注释或代码），不是被误报出来的字符串：只按那个片段豁免的话，
 * 同文件里真写出同样的字符串也会被一并放过。
 * 按行文本豁免则只放过这一行本身，规则对别处照常生效。
 *
 * 代价是**被豁免的整行从此对规则 1 不可见**，故只用于整行除该误报外不含界面文案的行；
 * 命中行里若还写着别的中文文案，那条文案也会一起被遮住。
 *
 * **当前为空**（批次 6 起）：三条存量豁免是 `路径不存在` 那三处「按后端文案子串判协议状态」
 * 的整行豁免。那三处已改成按 `AppError::Coded` 的 code 判，行内不再有中文，
 * 规则 1 也就没有可误报的东西了。空数组是**预期状态**，不是待办：
 * 新加豁免要有同等的具名理由，不要用它把真文案放行。
 */
const RULE1_FALSE_POSITIVE_LINES = [];

/**
 * 注释里的 t( 不是引用、注释里的中文不是文案，扫描前先去掉注释。
 *
 * 逐字符扫描，不用正则：正则版不认字符串边界，字符串字面量里的块注释起始符号
 * （svg.ts 里探测 WorkBuddy 样式的那个字符串、i18n/index.ts 里 locales 的通配路径）
 * 会被当成注释起点，一路吞到下一个注释结束符，中间的真实代码对规则 1/2/5 全部不可见，
 * 而闸门照常报「通过」——共享基础设施里的静默跳过。
 *
 * keepOffsets 为真时把注释换成等长空白而非删除：规则 5 要报行号，块注释被整段删掉
 * 会让其后所有行号前移，CI 给出的位置就指错了地方。两种取值共用同一遍扫描，
 * 对「哪些位置是注释」判断一致，差别只在注释区间是替换成空白还是删掉。
 *
 * 正则字面量整体置空，理由与注释相同而后果更重：`/"/g` 里的那个引号会被当成字符串起始，
 * 引号配对从此错位，错位之后注释与代码的判断全部失准——**该文件其余部分再也不会被扫描**，
 * 而闸门照常报「通过」。实测 src/utils/json.ts 因这一处让花括号深度卡住，文件末尾新写的
 * `const X = t("a")` 一条都不报。这不是「可能误报」，是静默失明。
 *
 * 已知边界：正则与除号的分别是**启发式**（见 startsRegexLiteral），不是完整的 JS 词法分析——
 * 后者代价大过收益。判不出结束位置时按除号处理：宁可漏扫一段正则体，也不误吞真实代码。
 * 这个边界今天覆盖本仓全部真实文件（自检在真实树上零误报），但启发式终究可能判错；
 * 判错时由 findStripMisalignment 把它报出来，而不是继续静默失明。
 */
export function stripJsComments(src, keepOffsets = false) {
  return scanJsComments(src, keepOffsets).text;
}

/**
 * 剥注释的扫描核心：一遍扫出剥好的正文，外加「这次扫描可不可信」的两个判据。
 *
 * 自检与剥注释共用这一遍扫描：自检若另写一份实现，守的只是那一份实现自身的行为，
 * 而真正需要守住的是这条被规则 1/5 共用的路径。
 */
function scanJsComments(src, keepOffsets) {
  const blank = (text) => (keepOffsets ? text.replace(/[^\n]/g, " ") : "");
  const out = [];
  const state = { prev: "", word: "" };
  let depth = 0;
  let unterminated = false;
  let danglingQuotes = 0;
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (ch === '"' || ch === "'" || ch === "`") {
      // 字面量整体原样保留：里面的 /* 与 // 是内容，不是注释起始；\ 转义不结束字面量。
      const end = skipStringLiteral(src, i);
      if (end > i) {
        // 找不到收尾引号时字面量一路吞到文件末尾，其后的真实代码整段不可见。自检靠这一位。
        // 判据是**越界**而非 `>= src.length`：正好闭合在文件末尾的字面量结束下标也是
        // src.length（见 skipStringLiteral），用 `>=` 会把合法文件报成未终止。
        if (end > src.length) unterminated = true;
        out.push(src.slice(i, end));
        i = end;
        state.prev = '"';
        state.word = "";
      } else {
        // 不是字符串起始（见 skipStringLiteral）：按普通字符处理，不改变引号配对。
        // 数下来交给自检：脚本区出现这种引号说明扫描在这里判不准，其后可能整段被误读。
        if (ch !== "`") danglingQuotes += 1;
        out.push(ch);
        advanceLexState(state, ch);
        i += 1;
      }
    } else if (ch === "\\") {
      // 判不出正则起始时按除号处理，正则体里的转义斜杠（`/^https?:\/\//`）就落在这里：
      // 转义序列整体跳过，其中的 / 是内容而非注释起始，否则会把行尾真实代码剥掉。
      // 两个字符都要过一遍词法状态：`\` 会把 word 清空，跳过的字符一步都不推进时 word 会残留
      // 跳过之前的标识符，于是 `const x = return\` 换行 `/}/;` 里的 `/` 被残留的 `return` 判成
      // 正则起始、`/}/` 整段置空——真实代码静默消失（变异测试实测：原样保留 vs 被置空）。
      // 只推进其中**一个**字符并不够，而哪一个更承重是输入相关的：本仓语料（144 条合成输入）
      // 里只推进第二个字符有 24 条与原文不同、只推进第一个有 0 条，但两个方向都能构造出差异，
      // 故不挑一个推，两个都推。
      // 这一支的射程是实测的，别按「字符串外裸写 `\` 不合法、所以走不到」去推——那是错的：
      // 闸门喂给本函数的不只有脚本区。一个模板正文里写着 `C:\Users\x` 的 `.vue`，在闸门真实
      // 路径上会让这一支走到 **6** 次：整份 .vue 被规则 1 与规则 5 各扫一遍（每遍 2 次，
      // 该字符串里有两个 `\`），templateRegionSwallowsScript 把 script 块挖空后再扫一遍（2 次），
      // 脚本块自检 0 次。本仓真实树同样三路合计 **0** 次，故今天不产生任何误判。
      // 它守的是「扫描器在任何输入上都不静默吞代码」，不是真实代码的覆盖率。
      out.push(src.slice(i, i + 2));
      for (const consumed of src.slice(i, i + 2)) advanceLexState(state, consumed);
      i += 2;
    } else if (ch === "/" && src[i + 1] === "*") {
      const close = src.indexOf("*/", i + 2);
      const end = close === -1 ? src.length : close + 2;
      out.push(blank(src.slice(i, end)));
      i = end;
    } else if (ch === "/" && src[i + 1] === "/") {
      const newline = src.indexOf("\n", i);
      const end = newline === -1 ? src.length : newline;
      out.push(blank(src.slice(i, end)));
      i = end;
    } else if (ch === "/" && startsRegexLiteral(src, i, state.prev, state.word)) {
      const end = skipRegexLiteral(src, i);
      if (end > i) {
        out.push(blank(src.slice(i, end)));
        i = end;
        state.prev = '"';
        state.word = "";
      } else {
        // 判不出结束位置时按除号处理：误吞一段真实代码比漏扫一段正则体重（见 skipRegexLiteral）。
        out.push(ch);
        advanceLexState(state, ch);
        i += 1;
      }
    } else {
      // 代码位置的花括号记账：自检靠它判断这次扫描有没有错位。
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      out.push(ch);
      advanceLexState(state, ch);
      i += 1;
    }
  }
  return { text: out.join(""), braceDepth: depth, unterminated, danglingQuotes };
}

/**
 * 剥注释的自检：返回「这次扫描不可信」的原因，可信时返回 null。
 *
 * **入参是脚本区文本**：`.ts` 是整份文件，`.vue` 是 findVueScriptBlocks 切出的块。模板与样式
 * 不是脚本区，其中的撇号（`Don't`）与落单的花括号（`50} off`）是正文而不是代码，按 JS 剥必然
 * 误报——那是本任务要消灭的假阳性，故由调用方在块外隔离，不在这里判。
 *
 * 剥注释一旦错位，错位点之后的真实代码对规则 1/5 全部不可见，而闸门照常报「通过」——
 * 这正是本文件已经修过数次的那类缺陷。自检把静默变成响亮：代码位置的花括号不平衡、模板字面量
 * 一直没闭合、单/双引号同行没有收尾引号，三者任一就说明结果不可信，此时报出来并让闸门失败，
 * 而不是继续在错误的输入上跑规则。
 *
 * 判据刻意不指向任何单一原因：正则里的引号、任何让引号配错对的东西，只要**后果**落进这几种
 * 签名之一，自检就报出来。它不是「每一种成因都会落进签名」——`++` 之后被误判成正则起始就一个
 * 签名都不落（另有 startsRegexLiteral 那一侧挡它）。反过来说，签名不响不等于扫描可信，
 * 只是这几种后果没出现。
 *
 * 第三条签名（同行未闭合的引号）是脚本区专有的：单/双引号字符串本就不能跨行，脚本区里出现
 * 这种引号只可能是扫描判错了位置（`const a = '你好;`，或正则被误判成除号后其引号与后文配对）。
 * 少了它，这类错位既不产生花括号不平衡也不产生未闭合模板字面量，两个签名都不响——
 * 静默漏报规则 1/5，而旧实现在这里靠「引号吞到文件末尾」响亮报错。
 */
export function findStripMisalignment(src) {
  const { braceDepth, unterminated, danglingQuotes } = scanJsComments(src, false);
  const reasons = [];
  if (unterminated) reasons.push("扫描结束时仍在字符串字面量里");
  if (danglingQuotes) reasons.push(`单/双引号同行没有收尾引号（共 ${danglingQuotes} 处）`);
  if (braceDepth !== 0) reasons.push(`代码位置的花括号不平衡（净 ${braceDepth}）`);
  return reasons.length ? reasons.join("；") : null;
}

/**
 * Rust 侧的剥注释，与 stripJsComments 是同一道边界的两半：字符串字面量里的块注释起始符
 * 一旦被当成注释起点，其后直到下一个注释结束符的真实代码对规则 2 全部不可见，
 * 而闸门照常报「通过」——共享基础设施里的静默跳过。
 *
 * 不需要 keepOffsets 变体：规则 2 只报文件名、不报行号，注释直接删掉即可。
 * JS 侧要那个变体，是因为规则 5 的「文件:行号」诊断会被整段删除的块注释整体前移。
 *
 * 与 JS 侧的三处语言差异，都是本仓库真实存在的写法，不是假想：
 * - `'` 在 Rust 里首先是生命周期标记（`&'a str`）。当成字符串起始会让 `'a` 一路吞到
 *   下一个 `'`，两次生命周期之间的真实代码落进「字符串」里，其中的中文对规则 2 不可见。
 *   故只在形如 `'x'` / `'\n'` 的闭合字符字面量上整体跳过，其余按普通字符处理。
 *   `matches!(ch, '"' | …)` 这类含引号的字符字面量同样靠这条分支才不错位。
 * - 原始字符串（`r#"…"#`）里的 `\` 不是转义符，按普通字符串处理会让 `\"` 提前收尾，
 *   之后的对引号全错。context_menu.rs 的非测试代码里就有这种写法。
 * - 块注释可嵌套：内层注释的结束符不结束外层，故按深度计数。只认第一个结束符
 *   会把内层之后的注释正文当成代码——那是误报，但同样让扫描结果与事实不符。
 */
export function stripRustComments(src) {
  const out = [];
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    // 廉价前缀判断先挡一道：r/b 在 Rust 里满地都是（return、borrow…），
    // 每个都切一次剩余全文会退化成 O(n²)。
    if ((ch === "r" && (src[i + 1] === '"' || src[i + 1] === "#")) ||
        (ch === "b" && src[i + 1] === "r" && (src[i + 2] === '"' || src[i + 2] === "#"))) {
      const raw = /^(?:br|r)(#*)"/.exec(src.slice(i));
      if (raw) {
        // 井号数量决定结束符：r"…" 收于 `"`，r#"…"# 收于 `"#`，以此类推。
        const close = `"${raw[1]}`;
        const at = src.indexOf(close, i + raw[0].length);
        const body = src.slice(i + raw[0].length, at === -1 ? src.length : at);
        // 重新拼成一个普通字面量而非原样保留：原始字符串的内容里可以有不带转义的 `"`，
        // 原样保留会让引号个数变成奇数，下游按引号配对取字面量的正则从此错位，
        // 其后所有中文串都被看成「不在字符串里」——又一处静默跳过。
        // 转义后内容不变（中文照旧被检出），引号个数恒为 2。
        out.push(`"${body.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`);
        i = at === -1 ? src.length : at + close.length;
        continue;
      }
    }
    const charLiteral = ch === "'" ? /^'(?:\\.|[^'\\\n])'/.exec(src.slice(i)) : null;
    if (charLiteral) {
      // 字符字面量整个换成空白：它可能含 `"`（`matches!(ch, '"' | …)` 这类写法仓库里就有），
      // 留着同样会打乱下游的引号配对。字符字面量本就不在规则 2 的范围内
      // （findCjkRustLiterals 只取双引号串），丢掉它不损失任何检出。
      out.push(" ");
      i += charLiteral[0].length;
    } else if (ch === '"') {
      let end = i + 1;
      while (end < src.length && src[end] !== '"') end += src[end] === "\\" ? 2 : 1;
      end = Math.min(end + 1, src.length);
      out.push(src.slice(i, end));
      i = end;
    } else if (ch === "/" && src[i + 1] === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < src.length && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") {
          depth += 1;
          j += 2;
        } else if (src[j] === "*" && src[j + 1] === "/") {
          depth -= 1;
          j += 2;
        } else {
          j += 1;
        }
      }
      i = j;
    } else if (ch === "/" && src[i + 1] === "/") {
      const newline = src.indexOf("\n", i);
      i = newline === -1 ? src.length : newline;
    } else {
      out.push(ch);
      i += 1;
    }
  }
  return out.join("");
}

/**
 * 日志不是界面文案：console.* 的输出用户看不到，仓库既有的 warn 站点也大量是中文。
 * 与规则 2 排除日志宏对称。括号嵌套过深或跨语句时不匹配，此时原文保留，宁可误报不漏报。
 */
function stripConsoleCalls(src) {
  return src.replace(/console\.[A-Za-z]+\((?:[^;()]|\([^)]*\))*\)/g, "");
}

/** Rust 侧同理：tracing 日志宏与 println/eprintln 是开发者日志，不是面向用户的文案。 */
function stripRustLogMacros(src) {
  return src.replace(
    /\b(?:[A-Za-z_]+::)?(?:trace|debug|info|warn|error|println|eprintln)!\((?:[^;()]|\([^)]*\))*\)/g,
    ""
  );
}

/**
 * 规则 1 的前端字面量扫描：字符串与模板字面量里的中文。
 *
 * 已知边界：正则字面量被整体涂白（见 stripJsComments），其中的 CJK 对规则 1 不可见——
 * `/是|否/` 这种把中文写进正则的站点闸门一声不响。今天 src/ 无此站点，且不为此放宽：
 * 「哪里开始是正则」是启发式判断，放宽去扫正则体会把除号误当正则起始，吞掉其后的真实代码，
 * 换来的失明比这一处更糟。真出现时把那处中文提成字符串常量或语言包条目。
 *
 * **转义写成 `\\[\s\S]` 而不是 `\\.`**，与 Rust 侧的取串正则同一条判据、同一个理由：
 * JS 的字符串可以用行尾 `\` 续行（LineContinuation），而 `\\.` 里的 `.` 不匹配换行，
 * 那个续行串自身配不上对，引擎转而从它的**收尾引号**起配——收尾引号被当成某个串的开引号，
 * 其后所有中文串随引号配对一起错位、一条都取不到。静默失明，闸门照常报「通过」。
 *
 * 可达条件：`'`/`"` 分支的内容类 `[^"\\\n]` 排除换行，故只有续行串的收尾引号与下一个串的
 * 开引号**同一行**时才错位（隔行时那个换行本身就让配对失败）。Rust 侧的内容类不排除换行，
 * 那边没有这个限制，所以那边的同一个缺陷射程更宽。
 */
export function findCjkLiterals(source) {
  const stripped = stripConsoleCalls(stripJsComments(source));
  const hits = [];
  const re = /"((?:[^"\\\n]|\\[\s\S])*)"|'((?:[^'\\\n]|\\[\s\S])*)'|`((?:[^`\\]|\\[\s\S])*)`/g;
  let m;
  while ((m = re.exec(stripped)) !== null) {
    const lit = m[1] ?? m[2] ?? m[3] ?? "";
    if (CJK.test(lit)) hits.push(lit);
  }
  return hits;
}

/**
 * Rust 字面量里的取串正则。**`\\.` 换成 `\\[\s\S]` 是承重的**：`.` 不匹配换行，
 * 而 Rust 的字符串可以跨行（行尾 `\` 续行，本仓的 SQL 串遍地都是——
 * `"INSERT INTO … \` 换行 `VALUES (…)`）。用 `\\.` 时，从那个续行串起**整个文件的引号配对
 * 全部错位**：其后每一条中文串都取不到，闸门照常报「通过」——静默失明，不是误报。
 * 实测：assistant/conversations.rs 的生产 CJK 报 0（真实 3 条，全是 `AppError::business`），
 * assistant/quick_phrases.rs 报 2（真实 3 条）。
 *
 * 这也消掉了「同一道边界两套实现」：stripRustComments 找串时按 `\` 跳两格（含换行），
 * 取串正则必须与它同判，否则两套实现对「哪里是字符串」的答案不一致，而失明的是下游那一套。
 * 每次调用现建一个：带 `g` 的正则把 lastIndex 记在正则对象上，模块级共用会跨调用串味。
 *
 * 已知边界：判据落在**源文本**上，`"\u{4E2D}\u{6587}"` 这种用 unicode 转义写出的中文
 * 一个码点都不含，取到的是转义写法本身（实测返回 `[]`；混写如 `"\u{4E2D}文"` 仍报得出，
 * 因为那个 `文` 是真实码点）。与规则 1 的正则体盲区同类：都是「中文没以码点形态出现在源文本
 * 里」。今天本仓 0 处（全部 `\u{…}` 都落在字符字面量、注释或非 CJK 码点上），真出现时
 * 写成普通中文字面量即可。
 */
export function findCjkRustLiterals(source) {
  const stripped = stripRustLogMacros(stripRustComments(source));
  const hits = [];
  const re = /"((?:[^"\\]|\\[\s\S])*)"/g;
  let m;
  while ((m = re.exec(stripped)) !== null) {
    if (CJK.test(m[1])) hits.push(m[1]);
  }
  return hits;
}

/**
 * 把一段文本换成**等长空白**，换行原样保留：下标与行结构都与原文一一对应。
 * span 记的是原文下标、行号也按原文算，故遮蔽只能用等长空白，不能删掉。
 */
function blankKeepingNewlines(text) {
  return text.replace(/[^\n]/g, " ");
}

/**
 * 花括号配平用的遮蔽副本：注释、字符串字面量、字符字面量整体换成**等长空白**，
 * 其余字符原样保留。等长是为了让下标与原文一一对应（span 记的是原文的下标）。
 *
 * **必须与 CJK 检测用的副本分开**（见 findCjkRustProductionLiterals）：本函数把字符串内容
 * 抹掉，而规则 2 要检的正是字符串里的中文。反过来，在带字符串的源码上配平会静默错位——
 * 本仓测试体里有 JSON 夹具（`"{\"a\": 1"` 这类含半个花括号的串），
 * 字符串里的 `{` / `}` 会混进深度记账，span 要么收不了尾、要么提前收尾，
 * 于是生产代码被当成测试代码涂掉（漏报），或测试代码漏成生产代码（误报）。
 *
 * 注释同样要涂白：注释里的花括号一样会让深度错位。且 `#[cfg(test)]` 的查找也跑在这份副本上，
 * 字符串或注释里写着的标记不会被当成测试项起点。
 *
 * 字符字面量按 `'x'` / `'\n'` 的形状识别，生命周期标记（`&'a str`）不在此列——按 stripRustComments
 * 的同一条边界处理，理由见那里。
 */
function maskRustLiteralsAndComments(src) {
  const out = [];
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    const charLiteral = ch === "'" ? /^'(?:\\.|[^'\\\n])'/.exec(src.slice(i)) : null;
    if ((ch === "r" && (src[i + 1] === '"' || src[i + 1] === "#")) ||
        (ch === "b" && src[i + 1] === "r" && (src[i + 2] === '"' || src[i + 2] === "#"))) {
      const raw = /^(?:br|r)(#*)"/.exec(src.slice(i));
      if (raw) {
        // 井号数量决定结束符，与 stripRustComments 同一套判据。原始字符串里可以有不带转义的
        // `"` 与花括号，整体涂白而不是按普通字符串处理。
        const close = `"${raw[1]}`;
        const at = src.indexOf(close, i + raw[0].length);
        const end = at === -1 ? src.length : at + close.length;
        out.push(blankKeepingNewlines(src.slice(i, end)));
        i = end;
        continue;
      }
      out.push(ch);
      i += 1;
    } else if (ch === "/" && src[i + 1] === "/") {
      const newline = src.indexOf("\n", i);
      const end = newline === -1 ? src.length : newline;
      out.push(blankKeepingNewlines(src.slice(i, end)));
      i = end;
    } else if (ch === "/" && src[i + 1] === "*") {
      // 块注释可嵌套：内层注释的结束符不结束外层，故按深度计数（与 stripRustComments 同）。
      let depth = 1;
      let j = i + 2;
      while (j < src.length && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") {
          depth += 1;
          j += 2;
        } else if (src[j] === "*" && src[j + 1] === "/") {
          depth -= 1;
          j += 2;
        } else {
          j += 1;
        }
      }
      out.push(blankKeepingNewlines(src.slice(i, j)));
      i = j;
    } else if (charLiteral) {
      out.push(blankKeepingNewlines(src.slice(i, i + charLiteral[0].length)));
      i += charLiteral[0].length;
    } else if (ch === '"') {
      let end = i + 1;
      while (end < src.length && src[end] !== '"') end += src[end] === "\\" ? 2 : 1;
      end = Math.min(end + 1, src.length);
      out.push(blankKeepingNewlines(src.slice(i, end)));
      i = end;
    } else {
      out.push(ch);
      i += 1;
    }
  }
  return out.join("");
}

/**
 * 从 `#[cfg(test)]` 之后找出该测试项的结束下标（不含），判不出时返回 -1。
 *
 * 入参必须是 maskRustLiteralsAndComments 的产物：字符串、字符字面量与注释都已涂白，
 * 故这里可以放心按花括号配平。
 *
 * 判据是「跳过属性与空白后，找到该项的项体」：`{` 就配平到配对的 `}`（`mod x { … }` /
 * `fn f() { … }`），`;` 就到此为止（`use …;`、`mod x;`、trait 里的 `fn f();`）。
 * 签名里的 `(` / `[` 先成对跳过：`fn f() -> [u8; 2] { … }` 的返回类型里有分号，
 * 不跳会让项在签名中间就收尾，函数体漏成生产代码。
 *
 * 可见性（`pub` / `pub(crate)` / `pub(super)` / `pub(in …)`）与 `async` / `unsafe` 不需要
 * 单独认：它们都是普通记号，顺着主循环走过去即可，`pub(crate)` 的括号由深度记账消化。
 */
function rustTestItemEnd(masked, from) {
  let i = from;
  for (;;) {
    while (i < masked.length && /\s/.test(masked[i])) i += 1;
    // 标记与项之间还可以夹别的属性（`#[allow(…)]` 等）。属性里的字符串已涂白，
    // 故不会把 `]` 误当成属性结束——`#[doc = "a]b"]` 这种写法由遮蔽副本挡住。
    if (masked[i] === "#" && masked[i + 1] === "[") {
      const close = masked.indexOf("]", i + 2);
      if (close === -1) return -1;
      i = close + 1;
      continue;
    }
    break;
  }
  let depth = 0;
  while (i < masked.length) {
    const ch = masked[i];
    if (ch === "(" || ch === "[") {
      depth += 1;
      i += 1;
    } else if (ch === ")" || ch === "]") {
      depth -= 1;
      if (depth < 0) return -1;
      i += 1;
    } else if (depth === 0 && ch === "{") {
      let braces = 0;
      while (i < masked.length) {
        if (masked[i] === "{") braces += 1;
        else if (masked[i] === "}") {
          braces -= 1;
          if (braces === 0) return i + 1;
        }
        i += 1;
      }
      return -1;
    } else if (depth === 0 && ch === ";") {
      return i + 1;
    } else {
      i += 1;
    }
  }
  return -1;
}

/**
 * 规则 2 的生产代码切片：把每个 `#[cfg(test)]` 测试项的区间换成等长空白，只留非测试代码。
 *
 * 旧实现是 `src.split("#[cfg(test)]")[0]`，它**假设第一个标记必在文件末尾**。实测不是：
 * 本仓有 **8 个文件**的标记不落在文件末尾，其中三处的形态与后果（**以下行号在 `15cac67f`
 * 上量得**——那是这套判据落地时的树，文件其后长过，不要在 HEAD 上按这些数字找行；同段里的
 * 生产 CJK 增减计数是当时那套旧实现的实测记录，不随文件长度重算；文件长度则是该树的实测值）：
 * - `parser/claude.rs`：`#[cfg(test)] use` 在**第 10 行**，一刀切让其后 2416 行（全文件 2426 行）
 *   对规则 2 失明——该文件生产 CJK 实测 **0 → 6**；
 * - `proxy.rs`：测试模块在第 **282** 行，生产代码从其后继续，实测 **4 → 63**；
 * - `commands/session.rs`：`#[cfg(test)] fn` 在**第 721 行**，文件中段，实测 **27 → 32**。
 *
 * 全仓合计 **304 → 384**（+80），有 6 个文件的计数因此变化。
 *
 * **这一处缺陷的后果比「少报几处」重**：把这类文件从 RUST_PENDING 里划掉**不会让它开始受检**——
 * 扫描走到标记处就停，闸门照常报「通过」，于是「收紧白名单」只收紧了数字，覆盖没变。
 * 白名单的收紧与切片的修复是两件事，只做前一件会得到一个看起来更紧、实际没多查的闸门。
 *
 * **两份副本，不能混用**：配平在 maskRustLiteralsAndComments 的产物（字符串已涂白）上做，
 * CJK 检测在**字符串完整**的原文上做（findCjkRustLiterals 只剥注释）。
 * 混用任一个方向都是静默失准：在带字符串的源码上配平会错位（见 maskRustLiteralsAndComments），
 * 在涂白的副本上找 CJK 则一条都找不到、闸门永远报「通过」。
 *
 * 判不出某个标记的项末时**不遮蔽**（而不是涂到文件末尾）：那种情况下测试代码里的中文会被
 * 误报出来——方向是响亮的，会被看见并修掉；涂到文件末尾则是静默漏掉其后全部生产代码，
 * 正是本函数要消灭的那类缺陷。
 */
export function findCjkRustProductionLiterals(source) {
  return findCjkRustLiterals(blankRustTestSpans(maskRustLiteralsAndComments(source), source));
}

/**
 * 生产代码切片：把每个 `#[cfg(test)]` 测试项的区间换成**等长空白**，只留非测试代码。
 * 等长是为了让下标与原文一一对应（span 记的是原文下标），规则 10 要按它取字面量与行号。
 *
 * 规则 2 与规则 10 共用这一份，不是各写一遍：两处若各写一套，「哪些位置是测试代码」
 * 就会有两个答案，而失准的那一套是静默的——测试里的 `coded()` 会被当成生产站点。
 *
 * 入参两份，不能合并：span 在 `masked`（字符串与注释已涂白，花括号可放心配平）上算，
 * 遮蔽要在 `source` 上做（规则 10 要读字符串内容）。两者长度与行结构一一对应。
 */
function blankRustTestSpans(masked, source) {
  const marker = "#[cfg(test)]";
  let body = "";
  let cursor = 0;
  let at = masked.indexOf(marker);
  while (at !== -1) {
    const end = rustTestItemEnd(masked, at + marker.length);
    if (end === -1) {
      at = masked.indexOf(marker, at + marker.length);
      continue;
    }
    body += source.slice(cursor, at) + blankKeepingNewlines(source.slice(at, end));
    cursor = end;
    at = masked.indexOf(marker, end);
  }
  return body + source.slice(cursor);
}

/**
 * 规则 10 的站点扫描：Rust **生产代码**里 `coded("…")` 的字面量 code，附行号。
 *
 * 为什么要遮蔽测试项：`src-tauri/src/error.rs` 的 `#[cfg(test)]` 块里有三处 `coded("a.b")`，
 * 那是序列化形状的测试装置，不是会渲染给用户的 code。不遮蔽的话闸门会报「`a.b` 未登记」，
 * 而**正确的修法是把测试代码切掉，不是往四份语言包里塞一个 `a.b`**——后者是把测试装置
 * 污染成语言包条目，还会让反向判据认它是个真键。切片复用 T1 建好的 `rustTestItemEnd`。
 *
 * 扫描在**涂白副本**上找 `coded(`（字符串与注释都已置空，故注释或字符串里写着的 `coded(`
 * 不会被当成站点——`/// 改成 coded("x.y")` 这种文档写法在批量迁移里遍地都是），
 * 字面量本身回到 `source` 的同一下标上读——涂白保留长度与换行，两份副本的下标一一对应，
 * 行号因此是文件行号而不是块内行号。测试项的遮蔽在涂白副本上做，故搜索与切片共用一次配平。
 *
 * **函数定义不是站点**：`error.rs` 的 `pub fn coded(code: &'static str)` 也长成 `coded(`，
 * 而它的第一个参数是形参。按「前面是 `fn`」排除——这是本函数唯一的收窄，收窄的理由是
 * 定义与调用在文本上只差这一个词，别处没有这种同形。
 *
 * **实参不是普通字符串字面量时返回 `code: null`**（形如 `coded(变量)`、`coded(r"…")`），
 * 由调用方报出来。R2 把签名收成 `&'static str` 只挡住了「运行时拼码」，
 * **挡不住一个 `&'static str` 的变量**——它指向哪个 code 静态判不出来，不能假装校验过了。
 *
 * **`keys` 是直接链在这个 `coded()` 之后的 `.with(k, v)` 键名**，判不出来时为 `null`
 * （见 chainedWithKeys）——规则 10 的占位符判据要用它，`null` 由调用方计数报出。
 *
 * 已知边界（**方向不同，要分开看**）：
 * - **漏报（危险的那一侧）**：`use AppError::coded as mk;` 之后再写 `mk("x.y")`——
 *   判据只认字面量 `coded(`，改名导入的调用点一个都扫不到，于是那个 code 既不进前向判据、
 *   语言包里也不会有条目，用户看到的还是裸 code，而闸门照常报「通过」。
 *   这是**静默失明**，与下面两条响亮误报不是一回事，故单独列在最前面。
 *   R2 的 `&'static str` 挡不住它（改的是名字不是类型）。今天 0 处；
 *   真要用别名导入时，把本函数的正则一并扩到那个别名，或干脆别用别名。
 * - 实参写成**原始字符串**（`r"x.y"`）时判成非字面量——那是响亮误报，本仓 0 处。
 * - 整份是测试代码、由别处 `#[cfg(test)] mod X;` 引入的文件（`session_index/tests.rs` 等）
 *   没有内联标记，其中的 `coded()` 会被当成站点——同样是响亮误报，今天 0 处。
 */
export function findCodedCodeLiterals(source) {
  const masked = maskRustLiteralsAndComments(source);
  const searchable = blankRustTestSpans(masked, masked);
  const hits = [];
  const re = /(?<![\w$])coded\s*\(/g;
  let m;
  while ((m = re.exec(searchable)) !== null) {
    if (/(?:^|[^\w$])fn\s+$/.test(searchable.slice(Math.max(0, m.index - 8), m.index))) continue;
    const open = m.index + m[0].length - 1;
    const i = skipRustTrivia(source, open + 1);
    const line = source.slice(0, m.index).split("\n").length;
    // 链上的 `.with()` 在涂白副本上定位（字符串与注释已置空，括号配对才可信），
    // 键名回到 source 的同一下标上读——两份副本长度与换行一一对应。
    const close = closingParen(searchable, open);
    const keys = close === -1 ? null : chainedWithKeys(source, searchable, close);
    if (source[i] !== '"') {
      hits.push({ code: null, line, keys });
      continue;
    }
    let j = i + 1;
    while (j < source.length && source[j] !== '"') j += source[j] === "\\" ? 2 : 1;
    hits.push({ code: source.slice(i + 1, j), line, keys });
  }
  return hits;
}

/**
 * 与 `(` 配对的那个 `)` 的下标；配不平（扫到文件末尾）时返回 -1。
 *
 * 入参必须是 maskRustLiteralsAndComments 的产物：字符串、字符字面量与注释都已涂白，
 * 故串里的括号不会混进深度记账。在原文上配平会被 `")"` 这类字面量带偏，算出错位的结束位置，
 * 而那个位置决定「后面跟着的是不是 `.with(`」——错位的结果是静默的（规则在那处站点上失效）。
 */
function closingParen(masked, open) {
  let depth = 0;
  for (let i = open; i < masked.length; i += 1) {
    if (masked[i] === "(") depth += 1;
    else if (masked[i] === ")") {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** 下一个非空白字符的下标。注释已在涂白副本里置空，故只跳空白。 */
function firstNonSpace(masked, from) {
  let i = from;
  while (i < masked.length && /\s/.test(masked[i])) i += 1;
  return i;
}

/**
 * 直接链在某个 `coded(…)` 之后的 `.with(k, v)` 键名；判不出来时返回 `null`。
 *
 * **只认链在同一个表达式里的 `.with()`**。换行不算断开——`coded("x")` 换行接 `.with(…)` 是
 * 同一个表达式，本仓 `assistant/commands.rs:441-442`（行号在 `319f5622` 上量得）就是这种写法；
 * 判据若只认同一行，那处真实的 `{status}` 站点会被当成「没有 with()」，规则在那里一条抓力都没有。
 *
 * 判据刻意收窄到这里：`coded()` 与 `.with()` 可以分处两条语句（`let e = coded("x"); e.with(…)`），
 * 那时静态判不出 `with()` 挂在哪个错误对象上，硬按文本相邻去配会造出一条既漏报又误报的判据
 * （合法代码上报错，闸门一旦乱叫就会被调松）。故整类交给调用方**计数报出**。
 *
 * 返回 `null` 的三种情形都是「判不出来」，**不是**「没有 with()」：没有直接链上 `.with()`、
 * 第一个实参不是字符串字面量、括号配不平。两者混为一谈，规则要么漏报要么误报。
 */
function chainedWithKeys(source, masked, afterCall) {
  let keys = null;
  let i = afterCall + 1;
  for (;;) {
    i = firstNonSpace(masked, i);
    if (masked[i] !== ".") return keys;
    const nameAt = i + 1;
    const name = /^[A-Za-z_][A-Za-z0-9_]*/.exec(masked.slice(nameAt, nameAt + 16));
    if (!name || name[0] !== "with") return keys;
    const open = firstNonSpace(masked, nameAt + name[0].length);
    if (masked[open] !== "(") return keys;
    const close = closingParen(masked, open);
    if (close === -1) return null;
    // 实参位置在**原文**上取（与 findCodedCodeLiterals 取 coded 实参同一处判据）：
    // 涂白副本把整个字符串字面量（含引号）换成了空白，在那里找「第一个非空白字符」
    // 会一路跳到字面量之后的 `,`，于是每个链式站点都会被判成「实参不是字面量」——
    // 判据从此对全部站点失效，而闸门照常报「通过」。
    const argAt = skipRustTrivia(source, open + 1);
    if (source[argAt] !== '"') return null;
    let j = argAt + 1;
    while (j < source.length && source[j] !== '"') j += source[j] === "\\" ? 2 : 1;
    if (keys === null) keys = [];
    keys.push(source.slice(argAt + 1, j));
    i = close + 1;
  }
}

/**
 * 跳过空白与注释，返回下一个代码字符的下标。
 *
 * 只为回答「`(` 之后的实参从哪里开始」：`coded(` 与实参之间可以夹着注释，只跳空白会把
 * 注释起始的 `/` 当成实参，把一处合法站点报成「实参不是字面量」。
 *
 * **它不是第二套「哪些字符是注释」的口径**：规则的判定（这是不是一个站点）由涂白副本给出，
 * 本函数只在一个已经确认是站点的位置上定位实参。注释的判据（行注释到行尾、块注释可嵌套）
 * 与 stripRustComments / maskRustLiteralsAndComments 逐条相同，改动时三处要一起看。
 */
function skipRustTrivia(src, i) {
  for (;;) {
    while (i < src.length && /\s/.test(src[i])) i += 1;
    if (src[i] === "/" && src[i + 1] === "/") {
      const newline = src.indexOf("\n", i);
      i = newline === -1 ? src.length : newline;
      continue;
    }
    if (src[i] === "/" && src[i + 1] === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < src.length && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") {
          depth += 1;
          j += 2;
        } else if (src[j] === "*" && src[j + 1] === "/") {
          depth -= 1;
          j += 2;
        } else {
          j += 1;
        }
      }
      i = j;
      continue;
    }
    return i;
  }
}

/**
 * 规则 10 的两个方向：站点产出的 code 集合与 `errors.json`（en）的键集合互查。
 *
 * 前向（`missing`）：站点产出而语言包没有 → 前端 `renderAppError` 回退成**裸 code 上屏**。
 * 反向（`orphaned`）：语言包有而没有任何站点产出 → 条目成了**手工维护的清单**。
 * 两个方向都要：只做前向，站点删了/改名了，条目留在语言包里没人发现，
 * 而清单看起来仍像覆盖——本仓在「清单看起来像覆盖」上反复付过代价。
 *
 * 两侧都是集合而不是列表：同一 code 可以被多个站点产出（如 `streaming.panic` 有两处），
 * 重复不改变判据。集合口径也顺带保证判据**不假设 code 集是冻结的**——T5 会继续加站点，
 * 新 code 与新条目只要同时到位就照常通过。
 */
export function findCodePackMismatches(codes, packKeys) {
  const produced = new Set(codes);
  const declared = new Set(packKeys);
  return {
    missing: [...produced].filter((c) => !declared.has(c)).sort(),
    orphaned: [...declared].filter((k) => !produced.has(k)).sort(),
  };
}

/** 一段文案里的 `{…}` 占位符名集合。 */
function placeholdersOf(text) {
  return new Set([...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((m) => m[1]));
}

/**
 * 规则 3 的补充（调用点在规则 3 之后，覆盖**全部**命名空间）：
 * 同一个 key 在四份语言包里的 `{…}` 占位符集合必须一致。
 *
 * 某一份漏写或改名时（`{path}` → `{pfad}`），**该语言下渲染出来的就是字面的 `{path}`**——
 * 与裸 code 同类，都是把内部标识符推给用户看，而规则 3 只比键集合、看不见取值里的占位符。
 * `en` 是基准，其余语言逐份比；缺键**跳过**（那是规则 3 的管辖，不在这里重复报）。
 *
 * **站点一侧由 findCodedWithKeyMismatches 补上**：本函数只比四份语言包**彼此**，所以
 * `coded("x").with("pfad", p)` 配上四份都写 `{path}` 的语言包在这里是**通过**的。
 * 那半边判据收窄在「`.with()` 直接链在 `coded()` 之后」的单表达式形态上，理由与收窄的代价见那里。
 */
export function findPlaceholderMismatches(packs, langs) {
  const base = packs.en ?? {};
  const hits = [];
  for (const [key, text] of Object.entries(base)) {
    const expected = placeholdersOf(text);
    for (const lang of langs) {
      if (lang === "en") continue;
      const other = packs[lang]?.[key];
      if (typeof other !== "string") continue;
      const got = placeholdersOf(other);
      const same = got.size === expected.size && [...got].every((p) => expected.has(p));
      if (!same) {
        hits.push({ key, lang, expected: [...expected].sort(), got: [...got].sort() });
      }
    }
  }
  return hits.sort((a, b) => a.key.localeCompare(b.key) || a.lang.localeCompare(b.lang));
}

/**
 * 规则 10 的第四段：**单表达式链**站点的 `with()` 键名与 `en` 条目的 `{…}` 集合必须一一对应。
 *
 * **这条补的是 findPlaceholderMismatches 的缺口**：那条只比四份语言包**彼此**，所以
 * `coded("x").with("pfad", p)` 配上四份都写 `{path}` 的语言包是**通过**的——而该语言下
 * 渲染出来的就是字面的 `{path}`，与裸 code 同类，都是把内部标识符推给用户看。
 * 339 条迁移会一条条走进这个缺口，故判据必须落在**站点**这一侧。
 *
 * 只收 `keys !== null` 的站点（即 `.with()` 直接链在 coded() 之后的，见 chainedWithKeys）；
 * 其余站点由调用方**计数并报出**，不在这里静默略过——静默会让这条规则看起来比实际覆盖得宽。
 * 语言包里没有该 code 的站点**跳过**：那是 findCodePackMismatches 的前向判据（裸 code 上屏），
 * 两条诊断指向同一件事会把读者引开。
 *
 * 两个方向都报：语言包声明而站点没提供 → 用户看到字面的 `{…}`（重）；站点提供而语言包没声明
 * → 那个参数被丢掉，文案里少一处上下文（同样是缺陷，只是后果轻些）。
 */
export function findCodedWithKeyMismatches(sites, pack) {
  const hits = [];
  for (const site of sites) {
    if (site.keys === null) continue;
    const text = pack?.[site.code];
    if (typeof text !== "string") continue;
    const expected = placeholdersOf(text);
    const got = new Set(site.keys);
    const same = got.size === expected.size && [...got].every((k) => expected.has(k));
    if (!same) {
      hits.push({
        code: site.code,
        file: site.file,
        line: site.line,
        expected: [...expected].sort(),
        got: [...got].sort(),
      });
    }
  }
  return hits.sort(
    (a, b) => a.code.localeCompare(b.code) || a.file.localeCompare(b.file) || a.line - b.line
  );
}

/**
 * 点分 key 在语言包里的落点：leaf（取得到文本）、branch（是子键集合）、missing。
 *
 * 动态 key 的静态前缀必须是 branch——落到 leaf 上说明前缀后面还接着内容，
 * 拼出来的键不会是那个叶子，把这种情况判成通过就是假绿灯。
 */
export function resolveMessageKey(messages, path) {
  let cur = messages;
  for (const seg of path.split(".")) {
    if (cur === null || typeof cur !== "object" || Array.isArray(cur) || !Object.hasOwn(cur, seg)) {
      return "missing";
    }
    cur = cur[seg];
  }
  if (cur !== null && typeof cur === "object" && !Array.isArray(cur)) {
    return Object.keys(cur).length > 0 ? "branch" : "missing";
  }
  return "leaf";
}

/**
 * 提取源码里 t() 调用的 key 引用，供规则 5 校验存在性。
 *
 * 只认字面量实参，且实参后面紧跟 `,` 或 `)`：变量与拼接（`t("a." + x)`）静态判不出来，
 * 拿字面量那半截去查存在性只会误报——闸门一旦乱叫就会被调松，那比窄更糟。
 * 带插值的模板字面量取 `${` 之前的静态前缀，并截到最后一个点：截掉最后一段半拉子键名，
 * 相邻键（`common.a` 与 `common.ab`）并存时才不会把 `t(`common.a${x}`)` 误判成叶子后接内容。
 * 前缀里没有点（`t(`${ns}.title`)`）时无从校验，跳过。
 * 调用形式只认裸标识符 `t(`：属性访问（`obj.t(`）、`$t(` 与改名导入的别名都不在范围内，
 * 本仓库的 t 一律从 `src/i18n` 具名导入，收窄到这里足以覆盖全部真实调用。
 * 但收窄挡不住遮蔽：文件里写 `const t = …`、或让箭头参数叫 `t`（`(t) => …`）遮掉导入后，
 * 形如 `t("字面量")` 的本地调用仍会被当成引用；本函数是**纯语法扫描、不看该文件有没有导入 t**，
 * 故不遮蔽导入的同名局部 `t`（如 sse.ts 的 `const t = raw.trimStart()`、
 * ConversationHistory.vue 里的 Date 时间戳）同样会被误读。这两类站点**只举例、不列全**：
 * 枚举会随改名与新增而失效，而失效的清单比没有清单更糟——它看起来像验证，照它核对的读者
 * 会去查一个已不存在的变量、漏掉真正在的那些。这条断言的实际验证是**规则 5 今天 exit 0**：
 * 遮蔽处真传了字符串字面量的话，那个解析不到的 key 会被报出来。往后再写遮蔽时别给 t 传字符串字面量。
 * 已知误报面：字符串字面量内部写着的 `t("…")`（把调用示例当测试数据）也会被当成调用。
 * 真去认字符串边界就得写词法分析，代价大过收益——真撞上时改掉那处写法即可。
 *
 * **转义写成 `\\[\s\S]` 而不是 `\\.`**（`"` 与 `'` 两个分支），与 findCjkLiterals 同一条判据：
 * `\\.` 里的 `.` 不匹配换行，key 字面量若用行尾 `\` 续行，这条引用整条取不到、
 * 规则 5 便不再校验它的存在性。与规则 1/2 同型，但**不级联**——本函数的每条匹配都以 `t(`
 * 起头，一次失败不消耗任何文本（lastIndex 只前进一位），故只漏这一条，其后的 `t(…)` 照旧找到。
 *
 * 已知边界（**刻意未改**，见规则 5 的说明）：模板字面量分支 `` `([^`\\]*)` `` 完全不认转义，
 * 故 `` t(`a\n`) `` 这类含 `\` 的模板实参整条取不到。它与「换行盲」不是同一个形态
 * （模板体本就允许裸换行，`[^`\\]` 不排除换行），且修它会改变捕获到的文本、
 * 进而牵动 `${` 前缀判定这条**判据**，不在本函数只修边界的改动范围内。
 */
export function findTranslationKeyRefs(source) {
  const text = stripJsComments(source, true);
  const refs = [];
  const re = /(?<![\w$.])t\(\s*(?:"((?:[^"\\\n]|\\[\s\S])*)"|'((?:[^'\\\n]|\\[\s\S])*)'|`([^`\\]*)`)(?=\s*[,)])/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const line = text.slice(0, m.index).split("\n").length;
    if (m[3] === undefined) {
      refs.push({ key: m[1] ?? m[2] ?? "", line, dynamic: false });
      continue;
    }
    const interpolation = m[3].indexOf("${");
    if (interpolation === -1) {
      refs.push({ key: m[3], line, dynamic: false });
      continue;
    }
    const dot = m[3].slice(0, interpolation).lastIndexOf(".");
    if (dot > 0) refs.push({ key: m[3].slice(0, dot), line, dynamic: true });
  }
  return refs;
}

/**
 * 把 i18n key 当数据存放的源码（`changeKeys: ["app.changelog.v0_1_0.proxy"]`）里的 key 引用。
 * 规则 5 只认 `t("字面量")`，看不见这类 key：一个错别字不会报错，
 * vue-i18n 会把点分路径原样渲染到 AboutView 与 WhatsNewDialog 上，没有东西拦。
 *
 * 只取 `*Keys: [...]` 数组里的字符串字面量——那是该文件里 key 的实际存放形态，
 * 也是能静态判定的子集。放宽成「文件里所有点分字符串」会把 `version: "0.1.0"`
 * 这类非 key 数据一并当 key 查，立刻误报；闸门一旦乱叫就会被调松，那比窄更糟。
 * 往后再加 key 字段时按同样的数组形态放，或同步扩展本函数。
 *
 * **转义写成 `\\[\s\S]` 而不是 `\\.`**，与 findCjkLiterals 同一条判据。这里比规则 5 更需要它：
 * 本函数的取串正则是**裸串匹配器**（不像规则 5 那样每条匹配都以 `t(` 起头），
 * 所以换行盲会**级联**——错配的那一次把后一个开引号当成自己的收尾引号，
 * 同一个 `changeKeys` 数组里其后的 key 一条都取不到（实测：`["app.a.\⏎b", "app.c.d"]`
 * 在旧形态下只取出一个凭空造出来的 `", "`，`app.c.d` 整个消失）。
 *
 * 已知边界（**刻意未改**）：外层 `arrays` 用 `\[([\s\S]*?)\]` 非贪婪截到第一个 `]`，
 * 不认字符串边界，故某个 key 里若写着 `]` 会把数组体截断。那是另一种形态，不在本次改动范围。
 */
export function findDataKeyRefs(source) {
  const text = stripJsComments(source, true);
  const refs = [];
  const arrays = /\b\w*Keys\s*:\s*\[([\s\S]*?)\]/g;
  let m;
  while ((m = arrays.exec(text)) !== null) {
    const bodyStart = m.index + m[0].indexOf("[") + 1;
    const literal = /"((?:[^"\\\n]|\\[\s\S])*)"|'((?:[^'\\\n]|\\[\s\S])*)'/g;
    let lit;
    while ((lit = literal.exec(m[1])) !== null) {
      refs.push({
        key: lit[1] ?? lit[2] ?? "",
        line: text.slice(0, bodyStart + lit.index).split("\n").length,
      });
    }
  }
  return refs;
}

/**
 * 语言包节点是「子键集合」还是「文本」。数组按文本处理——消息值不会以数组出现。
 *
 * 已知边界：数组与 `null` 都归到「文本」，故 `{"a":"1"}` 与 `{"a":["1"]}`（或 `{"a":null}`）
 * 之间不算形状不一致，会静默通过。本仓库没有任何语言包在规则 3 覆盖的层级上用数组或
 * null，故不额外处理；真要用到时把这里改成三分（branch / text / 其它）并让调用方分别处理。
 */
function isBranch(node) {
  return node !== null && typeof node === "object" && !Array.isArray(node);
}

/**
 * 一个语言包展平成 `键 → 文本`，嵌套对象用 `.` 连接（与规则 3 的键口径一致）。
 *
 * 判据取 `isBranch` 而不是 `typeof v === "object"`：后者会把数组也递归进去，
 * 展开成 `0` / `1` 这样的下标键，于是同一份语言包在规则 3 与规则 9 下有两套键。
 * 规则 3 把数组按文本处理（见 isBranch），这里必须同口径。
 */
export function flattenMessages(pack, prefix = "") {
  const out = [];
  for (const [k, v] of Object.entries(pack)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (isBranch(v)) out.push(...flattenMessages(v, key));
    else out.push([key, String(v)]);
  }
  return out;
}

export function compareNamespaces(base, target, prefix = "") {
  const missing = [];
  for (const [k, v] of Object.entries(base)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (!(k in target)) {
      missing.push(key);
    } else if (isBranch(v)) {
      // 形状不一致（base 是子键集合、target 是文本）时**不能递归**：`in` 对字符串会抛
      // TypeError，而抛出会让 main() 在打印已积累的 failures 之前中断——同一轮里
      // 其它文件的违规全部消失，闸门从「漏报一处」变成「静默丢弃整轮诊断」。
      // 这里保守地报成「该键缺失」（其下确实一条都取不到），main() 再用
      // findNamespaceShapeMismatches 的结果把这几个路径从缺口/多余键里滤掉，
      // 改由形状那一条报出。守卫与过滤都在，去掉任一个都会退化成误报或崩溃。
      if (!isBranch(target[k])) missing.push(key);
      else missing.push(...compareNamespaces(v, target[k], key));
    }
  }
  return missing;
}

/**
 * 同一键在两个包里形状不同的键路径（一侧是文本、另一侧是子键集合）。
 *
 * 与 compareNamespaces 分开走一遍，是因为形状对不上时「缺哪些键 / 多哪些键」本身
 * 就没有意义：报成缺口会把读者引向错误的方向（键其实两边都在，只是类型不同）。
 */
export function findNamespaceShapeMismatches(base, target, prefix = "") {
  const mismatched = [];
  for (const [k, v] of Object.entries(base)) {
    if (!(k in target)) continue;
    const key = prefix ? `${prefix}.${k}` : k;
    if (isBranch(v) !== isBranch(target[k])) mismatched.push(key);
    else if (isBranch(v)) mismatched.push(...findNamespaceShapeMismatches(v, target[k], key));
  }
  return mismatched;
}

/**
 * Vue 模板里的裸文本节点（`<div>保存</div>`）没有引号，字面量扫描看不见它，
 * 而它恰恰是 .vue 里最常见的硬编码文案。
 *
 * 只取 `<template>` 块内标签之间的文本：属性值（含 `:title` 这类绑定）在标签内部，
 * 天然不在范围内；`{{ }}` 插值里的字面量由字面量扫描负责，先整体剔除以免同一处报两遍；
 * `<script>` / `<style>` 在模板块之外，不会被捕获。
 */
export function findVueTemplateCjkText(source) {
  const block = source.match(/<template[^>]*>([\s\S]*)<\/template>/)?.[1];
  if (!block) return [];
  const text = block
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/\{\{[\s\S]*?\}\}/g, "")
    .replace(/<[^>]*>/g, "");
  return text.match(CJK_TOKEN) ?? [];
}

/**
 * 规则 8 的允许清单：`{ 允许的字符串: 理由 }`，读仓内提交的 scripts/i18n-english-allowlist.json。
 *
 * **它不是「白名单豁免」，是已经逐条裁定过的裁决集的机器可读形式。** 规则 8 的判据
 * （模板文本节点里含 ASCII 字母）刻意放宽，噪声由这份清单消化而不是由判据消化——
 * 判据每加一条形状过滤，就多一处会与裁决集脱节的地方。
 *
 * **它由 scripts/i18n-english-adjudication.json 派生**（`node scripts/find-english-blind.mjs --rekey`
 * 一并重键裁决集并重写本文件），**只发「规则 8 会命中」的那些 keep**：keep 的文本去空白后是清单的
 * 一个键、`reason` 直接沿用，但规则 8 够不到的 keep——脚本区的字面量、以及当前不在任何模板位置上的
 * 文本——不进清单，它们由「重跑 find-english-blind.mjs」这条通道兜（同规则 9 的取舍，见
 * gen-drift-allowlist.mjs：永不命中的条目列进去只会让清单看起来比实际抓力大）。
 * **这条派生由 `find-english-blind.test.mjs` 的等值断言守着**（提交的清单解析后必须等于当场派生的
 * 结果，另配两条阳性对照）——没有它，手改清单或加了 keep 忘了 `--rekey` 都不会红。
 * 裁决集按 `文件 → 文本` 记（人能读），行号只是那段文本当时的位置（今天只出现在 `_orphans`
 * 的键里）；本清单按**字符串**记（闸门按字符串判定）——后者是前者的投影，
 * 两个文件不是两个事实来源。**新增条目要先写进裁决集再派生过来**：只改这一个文件，
 * 等于把一次裁定藏进闸门的数据里，下一个做测量的人不会知道它已经被裁过。
 *
 * 每个值必须写明理由（缺理由由 main() 报出）：理由正是「裁决集」与「无名豁免」的分界。
 *
 * 同一字符串在不同位置可以扮演不同角色，而闸门按字符串判定分不清——`Agent` 既有 keep 判断
 * （`toolName === 'Agent'` 的比较值），也有一处 migrate 判断（`useTabs.ts` 里 Agent Dashboard 的
 * Tab 标签）；后者在 `ef9acb60` 迁成 `t("dialogs.common.agent")` 后成了孤儿——文本已不在文件里，
 * 键原样保留在 `_orphans` 的 `src/composables/useTabs.ts:240`。
 * 一条 keep 的文本一旦进清单，同一个字符串在别处的用法就一并被放行（判据宽的一端让位于清单），
 * 否则那些标识符会被反复报出来、闸门一旦乱叫就会被调松。代价是将来在模板里新写一句恰好用上
 * 清单里已有文本的标签不会被报出来——这是字符串判定的固有边界，棘轮挡的是**新出现的字符串**，
 * 不是同一个字符串的新用法。**`Agent` 今天不在清单里**：它的 keep 判断都不在规则 8 的扫描域里
 * （`ChatToolUsePart.vue` 那处是模板属性表达式、`TimelineView.vue` 那处在 `script` 区），
 * 而清单只发规则 8 会命中的 keep；将来它若出现在模板文本节点里，规则 8 会报出来，那是一次新裁定。
 */
function loadEnglishAllowlist() {
  return JSON.parse(readFileSync(join(ROOT, "scripts/i18n-english-allowlist.json"), "utf8"));
}

/**
 * 规则 9 的允许清单：`{ `en` 取值: 理由 }`，读仓内提交的 scripts/i18n-drift-allowlist.json。
 *
 * **与规则 8 的允许清单同形、同性质**：它不是「白名单豁免」，是已经逐条裁定过的
 * 漂移裁决集的机器可读投影。裁决集（`scripts/i18n-drift-adjudication.json`）按 `en` 值记
 * （人能读、能追溯到「为什么这是两条不同的源文案」），本清单是闸门能查的投影——
 * 两个文件不是两个事实来源。**由 `scripts/gen-drift-allowlist.mjs` 派生，不要手写**：
 * 只改这一个文件，等于把一次裁定藏进闸门的数据里，下一个做测量的人不会知道它已经被裁过。
 *
 * **清单只收「规则 9 真会命中」的条目**（`keep` 且该 `en` 值下 `ja`/`de` 确有分歧）。
 * `keep` 但 ja/de 本来就一致的条目**不进清单**——规则 9 永远不会因它们报警，
 * 把它们列进去只会让清单看起来比实际抓力大。这个过滤由生成脚本做并打印两个计数。
 *
 * 每个值必须写明理由（缺理由由 main() 报出）：理由正是「裁决集」与「无名豁免」的分界。
 */
function loadDriftAllowlist() {
  return JSON.parse(readFileSync(join(ROOT, "scripts/i18n-drift-allowlist.json"), "utf8"));
}

/**
 * 规则 9 参与比对的语言：`en` 是基准，`ja`/`de` 是被检查的译文。
 * **`zh` 刻意不在其中**——理由见 findUnlistedDrift：`zh` 的分歧是「两条不同源文案」的证据，
 * 不是译文漂移。本仓受支持的语言就是这四个（`SUPPORTED_LOCALES`），
 * 将来若新增第五种语言，要在这里显式加进来，否则它对规则 9 静默不可见。
 *
 * **导出**：`gen-drift-allowlist.mjs` 的输入必须与这里的判据同源，
 * 否则允许清单的投影语言集与闸门实际取到的语言集会分叉。
 */
export const DRIFT_LANGS = ["en", "ja", "de"];

/**
 * 规则 9：同一 `en` 取值下 `ja` 或 `de` 取值不唯一（即译文有分歧）的组，
 * 取其中该 `en` 值**不在**允许清单里的那些。
 *
 * **这条规则的存在理由**：合并 / 统一 / 拆身份只修好了现状，没有阻止复发——将来任何人在任何
 * 命名空间里新建一个与既有键同 `en`、`ja`/`de` 却不同的键，闸门此前看不见。本仓已为此付过代价：
 * 同一个 `Close` 在计划 4 内被两个任务答成了相反方向。
 *
 * **判据按 Ruling R3，不是 spec 的原话。** spec 写的是「同一 `en` 值、目标语言取值不同即报警」，
 * 该前提在计划 7 开工前就已被证伪：实测 43 组漂移**全部是合法的 `keep`**（①A/①B 两条不同源文案），
 * 照原话写会在第一天就变红。故判据多了「且不在裁决集里」这半边。
 *
 * **`zh` 分歧不参与本规则，且它连参数位都没有**——`packs` 只取 `en`/`ja`/`de` 三语。
 * `zh` 的分歧正是「两条不同源文案」的**直接证据**（①A/①B：组内一为英文原文、一为中文原文，
 * 或原文都是中文但逐字不同），把它算成漂移会把 43 组合法分叉全部报出来。
 *
 * `en` 相同但组内只有一条取值时不算漂移（各键的 `en` 本就不同，那本来就不是同一句源文案）。
 * 在 ja/de 里缺键的条目**跳过**：那是规则 3 的管辖（它会报「语言包缺口」并让闸门失败），
 * 不在这里重复报一遍。
 */
export function findUnlistedDrift(packs, allowlist) {
  const byEn = new Map();
  for (const [key, en] of Object.entries(packs.en)) {
    if (!(key in packs.ja) || !(key in packs.de)) continue;
    if (!byEn.has(en)) byEn.set(en, { keys: [], ja: new Set(), de: new Set() });
    const group = byEn.get(en);
    group.keys.push(key);
    group.ja.add(packs.ja[key]);
    group.de.add(packs.de[key]);
  }
  const hits = [];
  for (const [en, group] of byEn) {
    if (group.ja.size < 2 && group.de.size < 2) continue;
    if (Object.hasOwn(allowlist, en)) continue;
    hits.push({ en, keys: [...group.keys].sort(), ja: [...group.ja], de: [...group.de] });
  }
  return hits.sort((a, b) => a.en.localeCompare(b.en));
}

/**
 * 规则 8 覆盖的**静态文本属性**：只有这四个位置的值是给人看的，其余（`class` / `style` /
 * `id` / `data-*` / 组件 prop…）不是文案。`type === 6` 的静态属性按构造是字面量，
 * 判据落在它上面与落在文本节点上一样精确。
 *
 * **与 scripts/find-english-blind.mjs 共用这一份**（那边 import 本常量）：
 * 规则 8 的允许清单是从那个脚本产出的候选裁定出来的，两边的位置口径必须一致——
 * 各留一份，将来一边加一个属性名，另一边的候选与闸门的判定就静默错开，
 * 而输出看起来只是「清单里某条落空了」。
 */
export const STATIC_TEXT_ATTRS = new Set(["title", "aria-label", "placeholder", "alt"]);

/**
 * .vue 模板里「按构造就是 UI 文案」的位置的值：文本节点（`<template>` 块 AST 的 TEXT 节点）
 * 正文，与静态文本属性（`STATIC_TEXT_ATTRS`）的值。
 *
 * 用 SFC 编译器而不是像规则 1 那样切文本：规则 8 的判据落在**整个文本节点**上，需要节点边界。
 * 正则切不出边界——`<div>{{ t("a") }} Save</div>` 里 `Save` 与相邻插值的分界、
 * 以及嵌套标签之间各自的文本，正则只能靠标签名猜。
 * 而 `{{ … }}` 在 AST 里是独立的 INTERPOLATION 节点、根本不在文本节点里，
 * 「不是纯插值」这一条因此不需要额外判据（`<div>{{ count }} ERR</div>` 里的 ` ERR`
 * 仍是一个文本节点，照判——那正是「不是纯插值」要留住的那一半）。
 *
 * 属性**只取 `type === 6`（静态属性）**：`:title="x"` 这类指令（`type === 7`）的值是表达式，
 * 按构造不是文案，混进来会把标识符当成文案报。窄口径的理由同 `STATIC_TEXT_ATTRS`。
 *
 * 解析报错时**抛出**，不静默取半棵树：AST 缺了整段，那些文本节点就此无人检查，
 * 而闸门照常报「通过」——正是本文件反复修的那类静默失明。调用方（main()）逐文件 catch 后报出来。
 * 判据用 `errors` 数组而不是 try/catch：SFC 编译器对多数错误只收集不抛（重复的 `<template>` 就是），
 * 只看异常会让那些文件带着残缺的 AST 悄悄通过。
 */
function vueTemplateTextPositions(source) {
  const { descriptor, errors } = parseSfc(source);
  if (errors.length) throw new SyntaxError(errors[0].message ?? String(errors[0]));
  const out = [];
  const walk = (node) => {
    if (!node || typeof node !== "object") return;
    if (node.type === 2) {
      out.push(node.content);
      return;
    }
    if (node.type === 1) {
      for (const p of node.props ?? []) {
        if (p.type === 6 && STATIC_TEXT_ATTRS.has(p.name) && p.value) out.push(p.value.content);
      }
    }
    // 只递归 ROOT(0) 与 ELEMENT(1)：其余类型（注释 3、插值 5、属性 6、指令 7）里没有文本位置。
    if (node.type === 0 || node.type === 1) (node.children ?? []).forEach(walk);
  };
  walk(descriptor.template?.ast);
  return out;
}

/**
 * 规则 8：「按构造就是 UI 文案」的模板位置里**疑似英文界面文案**的文本，
 * 取其中**不在允许清单里**的那些。位置有两类（见 `vueTemplateTextPositions`）：
 * 模板文本节点，与 `STATIC_TEXT_ATTRS` 的静态属性值。
 *
 * **这条规则的存在理由**：规则 1 只认 CJK 码点，一句新写的 `Save changes` 对它完全不可见——
 * 这个类已经因此漏过两次（计划 2 与计划 5 的最终评审），闸门是唯一能挡住它回归的地方。
 * 属性这一半是计划 6 的评审补的：只覆盖文本节点时，`title="Save changes"` 这类
 * **同样是新写的英文界面文案**仍然无声通过。
 *
 * 判据两条，刻意放宽：① 取到的值去空白后**含至少一个 ASCII 字母**
 * （`Save changes`、` KB`、`W` 都算）；② 它不在允许清单里。
 * 「不是纯插值」由 AST 天然满足（见 vueTemplateTextPositions），
 * 「不是纯符号/数字」（`×`、`→`、`50%`）由 ① 天然满足——一个字母都没有。
 *
 * **判定按整个值（trim 后），不按空白切词**：允许清单是裁决集 `text` 的投影，
 * 而裁决集记的就是整个文本节点/属性值。按词切会让 `history tab is missing a CLI identity`
 * 这类整句在清单里落空，把一条已经裁定过的条目重新报出来。
 *
 * 判据**刻意不学 find-english-blind.mjs 的 isEnglishCandidate 做形状过滤**（点分路径、
 * snake_case、CSS 值…）：那是第二套事实来源，每加一条就多一处会与裁决集脱节的地方。
 * 宁可让 `models.dev` 这类域名也进清单逐条带理由，也不在这里猜「什么像代码」。
 *
 * **已知限制：棘轮只覆盖上述两类模板位置，不覆盖脚本区。** 判据的精度来自位置——
 * 这两类位置按构造几乎总是 UI 文案，而同样的判据搬到别处就变了性质：
 * - **脚本区**（`<script>` 与 `.ts`）里的英文字面量：那里绝大多数英文字面量**不是** UI 文案
 *   （标识符、枚举值、事件名、CSS 类名、i18n 键、import 路径…），同样的判据搬过去会在每次新增
 *   标识符时误报、允许清单无界增长——正是 spec 论证过的「英文即违规活不过一天」。
 *   实测：往 useTabs.ts 注入 `const __probe = "Save changes";` 闸门 exit 0（规则 1 只认 CJK、
 *   规则 7 只管 t() 的求值时机，没有任何规则扫脚本区的英文字面量）。
 *   **这个向量由「重跑 scripts/find-english-blind.mjs」兜，不由闸门兜**——实测 T2–T4 迁走的 24 条
 *   里有 7 条在 `.ts` 文件里（必然是脚本区，本规则一条都覆盖不到），棘轮只挡住了复发面的一部分。
 */
export function findUnlistedEnglishTexts(source, allowlist) {
  const hits = [];
  for (const raw of vueTemplateTextPositions(source)) {
    const text = raw.trim();
    if (!/[A-Za-z]/.test(text)) continue;
    if (Object.hasOwn(allowlist, text)) continue;
    hits.push(text);
  }
  return hits;
}

/**
 * .vue 的 `<script>` / `<script setup>` 块正文，附「块首内容在文件里的行号」。
 *
 * 模块顶层声明在 script 块里，模板与样式块不是模块作用域，故要先取出块再扫。
 * 行号按块首在文件中的位置折算：直接把块正文当整份文件扫，报出的行号会指向文件开头，
 * CI 里给出的位置就指错了地方。
 *
 * 开标签按引号配对扫到真正的 `>`：`generic="T extends Record<string, any>"` 这种属性值里
 * 就有 `>`，按第一个 `>` 截断会让块正文以 `">` 开头，那个落单的引号随即把整块置空——
 * 又一处「闸门照常报通过、实际一个字都没扫」的静默跳过。
 */
export function findVueScriptBlocks(source) {
  const blocks = [];
  const re = /<script\b(?:[^>"']|"[^"]*"|'[^']*')*>([\s\S]*?)<\/script>/g;
  let m;
  while ((m = re.exec(source)) !== null) {
    const openLength = m[0].length - m[1].length - "</script>".length;
    blocks.push({ text: m[1], line: source.slice(0, m.index + openLength).split("\n").length });
  }
  return blocks;
}

/**
 * 字符串字面量结束后的下标；未闭合时取文件长度 + 1。
 *
 * 返回值刻意不夹到 src.length：**正好闭合在文件末尾**的字面量（`const a = "x"`）其结束下标
 * 就是 src.length，与「一路吞到 EOF」无法区分，调用方会把一份合法文件判成未终止。
 * 越界的下标由调用方按 `end > src.length` 判未终止，`src.slice(i, end)` 两种取值都安全。
 *
 * 单引号与双引号找不到**同一行**的收尾引号时返回起点，由调用方按普通字符处理——
 * 那个引号不是字符串起始。这是 `.vue` 里合法模板正文与 HTML 注释的撇号（`Don't`、`You're`）
 * 造成的假阳性：整份 .vue 是按 JS 剥的，撇号被当成引号起始后引号配对从此错位，
 * 一路吞到文件末尾，闸门在一份完全合法的文件上 exit 1。而 JS 的单/双引号字符串本就不能
 * 含未转义的换行（行尾 `\` 续行由上面的转义分支跳过），故「同行无收尾引号 ⇒ 不是字符串」
 * 与语法一致，下游按行取字面量的正则也是这么认的。
 *
 * 取舍：单/双引号因此再也报不出 `unterminated` 那一位（只剩合法跨行的模板字面量报得出），
 * 但自检**没有**少一层网：扫描把「这里判不准」记进 danglingQuotes，自检对**脚本区**报出来
 * （见 findStripMisalignment）。脚本区里引号必然同行闭合，这条判据比「吞到文件末尾」更早、
 * 也更准；模板区则由调用方隔离在自检之外。所以模板正文里的撇号既不再误报，也没有把
 * 「响亮地报错」换成「静默失明」。
 *
 * 另一条路是把自检收窄到脚本块了事、引号判定原样不动，那会让脚本区的 `const a = '你好;`
 * 从响亮报错变成一声不响（规则 1 看不见其中的中文），故不取。
 *
 * 模板字面量不能这样处理：它合法跨行，同行没有收尾反引号说明不了什么。
 */
function skipStringLiteral(src, i) {
  const quote = src[i];
  let j = i + 1;
  while (j < src.length && src[j] !== quote) {
    if (src[j] === "\\") {
      // `\` + CRLF 是**一次**续行，只跳两个字符会把 `\r` 留在原地，下一轮把 `\n` 读成换行，
      // 于是 `'a\` + CRLF + `}'` 这种合法写法被判成「同行没有收尾引号」，其中的 `}` 落进
      // 深度记账、在一份合法文件上报花括号不平衡。没有 .gitattributes，Windows 检出就是 CRLF。
      j += src[j + 1] === "\r" && src[j + 2] === "\n" ? 3 : 2;
      continue;
    }
    // `\r` 单独出现也是换行：CRLF 文件里的行尾先撞上它，不认就得多走一格才判出来。
    if (quote !== "`" && (src[j] === "\n" || src[j] === "\r")) return i;
    j += 1;
  }
  return j + 1;
}

/**
 * 正则字面量结束后的下标；判不出结束位置时返回起点，由调用方按除号处理。
 *
 * 判不出就退回除号是刻意的：误把除号当正则会吞掉一段真实代码，其中的花括号令整份文件的
 * 深度记账错位——而深度记账一旦错位，函数体里的声明会被当成模块顶层，闸门开始乱叫。
 * 正则不跨行，故遇到换行即认输；字符类里的 `/` 不结束正则（`/[a/]/` 这类写法仓库里可能有）。
 *
 * 这一支是承重的：`const re = /{/;` 里的 `{` 若不跳过，深度会永久多一层，其后整份文件都
 * 落在深度 > 0 上，模块级声明再也切不出来（全文件静默失明）。仓库里没有 `/{/` 这种写法，
 * 但 `/^(https?:|mailto:|tel:)/i` 这类正则满地都是，判错一次就够失明一次。
 */
function skipRegexLiteral(src, i) {
  let j = i + 1;
  let inClass = false;
  while (j < src.length) {
    const c = src[j];
    if (c === "\n") return i;
    if (c === "\\") {
      j += 2;
      continue;
    }
    if (inClass) {
      if (c === "]") inClass = false;
    } else if (c === "[") {
      inClass = true;
    } else if (c === "/") {
      return j + 1;
    }
    j += 1;
  }
  return i;
}

/**
 * `/` 之前出现这些字符时它开启正则而不是除号：标识符、`)`、`]`、字面量之后才是除号。
 *
 * 刻意不含 `<`：`</div>` 这类闭合标签遍地都是，而 `a < /re/` 这种写法没人写。
 * 把 `</` 认成正则起始会一路吞到同一行下一个 `/`，把标签之间的真实代码剥掉——
 * 实测 UsageDashboard.vue 与 SessionAnalyticsModal.vue 各有一行里的 t() 引用因此消失。
 * 方向上的取舍：漏认一个正则只是少扫一段正则体，误认一个正则会吞掉真实代码。
 */
const REGEX_PRECEDING_CHARS = "([{,;:=!&|?+-*%~^>";
/**
 * 这些关键字之后必然是正则：`return /^(https?:|mailto:|tel:)/i.test(href)` 这类写法仓库里真有，
 * 按除号处理会把正则体当代码扫。真正致命的是正则体里带花括号（`return /{/;`）——那会让深度
 * 永久错位，其后整份文件失明。
 */
const REGEX_PRECEDING_KEYWORDS = new Set([
  "return", "typeof", "instanceof", "new", "delete", "void", "case", "do", "else", "yield", "await", "throw",
]);

/**
 * `/` 开启的是正则还是除号：看它前一个非空白字符与它所在的那个词，再看紧跟前面的两个字符
 * 是不是成对的自增自减运算符。
 *
 * 成对的 `++` / `--` 之后 `/` 必然是除号：后缀自增的表达式已经完整，前缀自增后面也不可能跟正则。
 * 少了这一条，`i++ / 2; // 注释` 里的 ` / 2; /` 会被当成正则体吞掉，连同 `;` 与 `//` 起始一起：
 * 注释正文留在剥好的正文里，规则 1 会把注释里的中文当成硬编码文案报出来（`i++ / 2; // 显示
 * "保存" 按钮` 实测 exit 1），而花括号与引号都没错位，自检两个签名都不响。
 *
 * 判据只看**紧挨着的**两个同字符（中间的空白先跳过）：`const ok = a + +/{/.test(s);` 是合法 JS，
 * 两个 `+` 之间隔着空白，是二元加号接一元加号，那个正则体必须照旧整体置空。把它一并当成
 * 自增运算符，正则体里的 `{` 就会混进深度记账，在一份合法文件上报「花括号不平衡」。
 * 整类把 `+` / `-` 移出 REGEX_PRECEDING_CHARS 同样是这个后果，故都不取。
 */
function startsRegexLiteral(src, i, prev, word) {
  let j = i - 1;
  while (j >= 0 && /\s/.test(src[j])) j -= 1;
  if ((src[j] === "+" || src[j] === "-") && src[j - 1] === src[j]) return false;
  return REGEX_PRECEDING_CHARS.includes(prev) || REGEX_PRECEDING_KEYWORDS.has(word);
}

/**
 * 词法状态：`prev` 是上一个非空白字符，`word` 是正在读的标识符（读到其它字符即清空）。
 * 字面量整体跳过后按 `prev = '"'`、`word = ""` 记：字面量之后跟的是除号，不是正则起始。
 */
function advanceLexState(state, ch) {
  if (/\s/.test(ch)) return;
  state.word = /[\w$]/.test(ch) ? state.word + ch : "";
  state.prev = ch;
}

/**
 * 剥掉包在调用目标外面的包装，露出真正被调用的那个节点。
 *
 * 剥的是**值就是其操作数**的那几种包装，零歧义：
 * - 括号 `(t)("a")`；
 * - 逗号表达式 `(0, t)("a")` 与赋值表达式 `(t = t)("a")`——两者的值都是右操作数；
 * - TS 的纯类型层包装 `t!("a")`（NonNullExpression）、`(t as any)("a")`（AsExpression）、
 *   `(<any>t)("a")`（TypeAssertionExpression）、`(t satisfies any)("a")`（SatisfiesExpression）
 *   ——它们**发射出来就是 `t("a")`**（实测 ts.transpileModule 的输出逐字相同），
 *   运行时调用的就是 t，只是多了一层只存在于类型层的写法。
 *
 * 刻意**不**剥按真值取值的包装（`(t || obj.f)("a")`、`(t ?? obj.f)("a")`）：调的是 t 还是 obj.f
 * 取决于 t 在运行时是不是真值，静态判不出来，故整类记在下面的假阴性列表里，不在这里猜。
 */
function unwrapCallee(node) {
  let current = node;
  for (;;) {
    if (
      ts.isParenthesizedExpression(current) ||
      ts.isNonNullExpression(current) ||
      ts.isAsExpression(current) ||
      ts.isTypeAssertionExpression(current) ||
      ts.isSatisfiesExpression(current)
    ) {
      current = current.expression;
      continue;
    }
    if (
      ts.isBinaryExpression(current) &&
      (current.operatorToken.kind === ts.SyntaxKind.CommaToken ||
        current.operatorToken.kind === ts.SyntaxKind.EqualsToken)
    ) {
      current = current.right;
      continue;
    }
    return current;
  }
}

/**
 * 规则 7 的判据：这个节点是不是在调用裸标识符 `t`（`t("…")` / `new t("…")`）。
 *
 * `new t(…)` 一并认下：构造调用同样会执行到 t，`new t("a")` 写在模块顶层就是加载期求值一次，
 * 与 `t("a")` 的症状完全一样。调用目标外面包着括号或逗号表达式时先剥掉（见 unwrapCallee）。
 *
 * 只认裸标识符，与规则 5 同一处收窄：本仓库的 t 一律从 src/i18n 具名导入，属性访问（`obj.t(`）
 * 与 `$t(` 不在范围内。收窄挡不住遮蔽——文件里写 `const t = …`、或让箭头参数叫 `t` 遮掉导入后，
 * 形如 `t("字面量")` 的本地调用仍会被当成调用。本函数是**纯语法扫描、不看该文件有没有导入 t**，
 * 与规则 5 同属刻意留窄：闸门一旦乱叫就会被调松，那比窄更糟。
 */
function invokesTranslation(node) {
  if (!ts.isCallExpression(node) && !ts.isNewExpression(node)) return false;
  const callee = unwrapCallee(node.expression);
  return ts.isIdentifier(callee) && callee.text === "t";
}

/** 带 static 修饰符的类成员。只看自己的修饰符，不向上找父节点。 */
function isStaticMember(node) {
  return (node.modifiers ?? []).some((m) => m.kind === ts.SyntaxKind.StaticKeyword);
}

/**
 * 这条语句里有没有**加载期求值**的 t()：当且仅当该调用不在任何「求值点不是加载期」的节点内。
 *
 * 两种机制，症状相同，判据因此只按位置定义，两种文件同一套规则：
 * - `.ts`：模块顶层，**import 时**求值一次，语言在那一刻被定死；
 * - `.vue` 的 `<script setup>`：块内顶层代码编译进 `setup()`，严格说**不是模块作用域**，
 *   而是在**组件挂载时**求值一次——组件不重挂载就再不重算，症状与前者一致。
 * t() 读的是响应式的 locale，写在模板或函数体里都没问题，问题只出在这种「求值一次」的位置。
 *
 * 边界一：函数类节点，由 ts.isFunctionLike 给出——箭头函数、函数表达式、函数声明、方法声明、
 * 构造器、get/set 访问器。它同时也是判据的全部——`computed(() => t(…))` 与
 * `watch(…, () => t(…))` 的回调天然落在这条边界内，不需要再维护「整段含不含 `=>`」那种近似。
 *
 * 边界二：**实例字段的初始化式**。它在每次 `new` 时求值，不是只求值一次——运行时实测顺序是
 * 「类定义结束 → 实例字段」，故其中的 t() 与函数体同类，不报。
 * 同一族里其余四种形态的求值点都在类定义时（实测顺序都是「类定义结束」之前），故照报：
 * `static` 字段的初始化式、`static {}` 块、字段名里的**计算键**（`class A { [t("a")] = 1 }`
 * 里的 t 在类定义时求值，而同一个字段的初始化式要等到构造）、以及方法 / 访问器的**计算名**
 * （`class A { [t("a")]() {} }`——方法体是懒的，名字却要先求出来才能建方法）。
 * 实例字段因此只跳过初始化式，字段名照旧向下走——这一处区分是运行时 oracle 定出来的，
 * 不是从 AST 形状推的。
 *
 * 对象字面量里的 `{ a: () => 1, b: t("c") }` 只放过箭头那半边，`b` 照报——属性不是函数类节点，
 * 只有箭头回调本身是。
 *
 * 已知假阴性（刻意保留，方向与「宁可漏也不乱叫」一致）：
 * - **函数类节点在模块求值期间被调用**这一类：判据只看「这个 t() 在不在函数体里」，看不到函数
 *   随后被调用。IIFE（`(() => t("a"))()`）只是它最显眼的一种，同类的还有 `function f() { return
 *   t("a") }` 后跟 `f()`、`const f = () => t("a")` 后跟 `f()`——后者两种没有 IIFE 那么显眼，
 *   但漏报机理完全相同。结构性地修 IIFE 不会关掉这一类（后两种仍在），只会造出一条新的、
 *   同样随意的边界，故整类一并记下不修：要认出它们得做调用图与求值顺序分析，代价大过收益。
 * - 标签模板 ``t`a` ``：它同样在加载期调用 t，但不是 vue-i18n 的调用形态（本仓的 t 只以
 *   `t("key")` 形式出现），故不收；真出现时改成普通调用即可。
 * - 按真值取值的包装：`(t || obj.f)("a")`、`(t ?? obj.f)("a")` 调的是 t 还是 obj.f 取决于 t
 *   在运行时是不是真值，静态判不出来，故不剥（见 unwrapCallee）。
 * - 装饰器。分四种形态，逐条实测过，**别把它们的理由混成一句话**：
 *   ① 会求值且判据报出：字段装饰器（`@dec(t("a")) x = 1`，含 `#x` / `accessor x` / `[k]` 与
 *      static 各种字段形态）、方法 / 访问器装饰器（`@dec(t("a")) m() {}`、`@dec(t("a")) get x()`）、
 *      类级装饰器（`@dec(t("a")) class A {}`）——求值点都在类定义时；
 *   ② 不会求值且判据不报：构造器装饰器（`@dec(…) constructor() {}`）——TS 在 legacy 与标准两种
 *      装饰器模式下都不发射它（实测 `transpileModule` 输出里连调用都没有）；
 *   ③ 不会求值但判据报出，**假阳性**：`@dec(…) static {}`——同上两种模式都不发射，报出来是
 *      假阳性（本仓 0 处，且这种写法没人写，按裁定记名不处理）；
 *   ④ 分模式：参数装饰器（`constructor(@dec(t("a")) x) {}`）只有 legacy 模式会求值、标准模式
 *      不发射，判据不报；私有字段装饰器（`#x`）反之，legacy 不发射、标准发射，判据报的是标准
 *      模式的语义（本仓 tsconfig 未设 experimentalDecorators）。
 * - 求值位置由运行时决定的写法（动态 import、条件分支里的声明）一概按书写位置算。
 *
 * 已知假阳性（刻意保留，方向相反，一并记下）：**遮蔽导入的同名 t**（见 invokesTranslation）。
 * 文件里写 `const t = (x) => x` 之后再写 `t("a")`，报出的是那个局部函数、不是 i18n 的 t，
 * 运行时也不加载期求值——判据是纯语法扫描，不看该文件有没有导入 t。本仓真实树 0 处，
 * 真写遮蔽时别给局部 t 传字符串字面量。
 *
 * 用 AST 而不是语句切分：切分启发式在本文件里被打了四次补丁，每次留下一个更窄的洞——最后一次
 * 留下的两个是「空体多行箭头在合法代码上误报」（`const f = (x) =>` 换行接 `t("a")`，而本仓
 * 以 `=>` 结尾的行有 74 行）与「段首不是候选的语句整段丢弃」（`const A = 1` 换行接
 * `arr.push(t("a"))` 漏报）。两者都不是判据的边界，而是「在文本上猜语句边界」这一做法的固有
 * 产物：判据本该按语法结构定义，而语法结构由解析器给出，不需要猜。本工程在剥注释上做过同样的
 * 判断——正则换成逐字符扫描，理由一模一样。留下第二套实现就是留下第二个事实来源。
 */
function statementHasEagerTranslationCall(statement) {
  let found = false;
  const visit = (node, insideFunction) => {
    if (found) return;
    // 先算本节点有没有跨进函数体：箭头函数、函数表达式、函数声明本身就是边界，边界内的
    // 所有后代一律不算加载期求值。
    const nested = insideFunction || ts.isFunctionLike(node);
    if (!nested && invokesTranslation(node)) {
      found = true;
      return;
    }
    if (ts.isPropertyDeclaration(node) && !isStaticMember(node)) {
      // 实例字段：初始化式在构造时求值，不报；但**字段装饰器**在类定义时求值，与 static 字段的
      // 装饰器一样，故先按外层求值点把修饰符走一遍，再跳过初始化式。
      // 少了这一步，`@dec(t("a")) x = 1` 会被整条跳过，而 `@dec(t("a")) static x = 1` 照报——
      // 同一个装饰器写法因 static 与否而结论相反，是说不通的。
      for (const modifier of node.modifiers ?? []) visit(modifier, nested);
      visit(node.name, nested);
      return;
    }
    // 方法 / 访问器的**计算名**与**装饰器**在类定义（或对象字面量求值）时就算，与函数体不是
    // 一回事：它们本身是函数类节点，但名字要先求出来才能建方法（`class A { [t("a")]() {} }`
    // 里的 t 在类定义时求值，方法体里的 t 要等调用），装饰器表达式同理（`@dec(t("a")) m() {}`）。
    // 故这两处按**外层**的求值点判定，函数体照旧不报。
    // 构造器除外：TS 在 legacy 与标准两种装饰器模式下都不发射构造器的装饰器（实测
    // transpileModule 输出里连调用都没有），它根本不会被求值，认下只会是假阳性。
    if (ts.isFunctionLike(node) && !ts.isConstructorDeclaration(node)) {
      for (const modifier of node.modifiers ?? []) visit(modifier, insideFunction);
      if (node.name && ts.isComputedPropertyName(node.name)) visit(node.name, insideFunction);
    }
    ts.forEachChild(node, (child) => {
      visit(child, nested);
    });
  };
  visit(statement, false);
  return found;
}

/**
 * 解析脚本正文。**解析失败一律抛出**，由调用方报出来并让闸门失败，绝不静默跳过：
 * 跳过等于这份文件的加载期 t() 无人检查，而闸门照常报「通过」——那正是本文件六次缺陷的共同形状。
 *
 * `setParentNodes` 关掉：判据只需要「这个节点在不在函数体里」，遍历时沿语句向下传一个标志位
 * 就够，不必向上找父节点，省掉一遍父指针挂接。
 * `ScriptKind.TS`：`.vue` 的 `<script setup>` 是合法 TS（`defineProps<…>()` 由 vue-tsc 解析），
 * 类型标注、`as`、泛型实参在本仓真实代码里遍地都是，按 JS 解析会把它们判成语法错误。
 */
function parseModule(source) {
  const sourceFile = ts.createSourceFile("module.ts", source, ts.ScriptTarget.Latest, false, ts.ScriptKind.TS);
  if (sourceFile.parseDiagnostics.length > 0) {
    const first = sourceFile.parseDiagnostics[0];
    const { line, character } = sourceFile.getLineAndCharacterOfPosition(first.start ?? 0);
    const detail = ts.flattenDiagnosticMessageText(first.messageText, " ");
    const error = new SyntaxError(`第 ${line + 1} 行第 ${character + 1} 列：${detail}`);
    // 行号是**块内**行号。`.vue` 的脚本块不在文件开头，调用方必须把它折算回文件行号，
    // 故三个分量一并挂在错误上，别让调用方去解析消息字符串（折算错方向会把读者指到模板上）。
    error.line = line + 1;
    error.column = character + 1;
    error.detail = detail;
    throw error;
  }
  return sourceFile;
}

/**
 * 预过滤用的判据：正文里出现过独立的 `t` 标识符。
 *
 * **不能只找 `t(` 这三个连续字符**：`t ("a")`、`t` 换行 `("a")`、`t?.("a")`，以及 t 与 `(`
 * 之间夹一段块注释的写法，四者都在加载期调用 t、都解析成同一个调用表达式，而它们都不含 `t(`
 * 这个连续子串——只看 `t(` 会让这几种写法整份文件不进解析器，闸门对它们一声不响。
 *
 * 要求出现独立的 `t` 标识符则是**必要**条件：调用点必然含它，故不会漏掉任何命中。
 * 唯一的例外是用 unicode 转义写出的标识符（把 t 写成反斜杠 + u0074 的转义形式）——判据本身
 * 认得它（解析器会把转义解出来），是预过滤放行了才会漏。本仓不写这种，真出现时改成普通写法即可。
 *
 * 代价是正文里出现的独立 `t` 也可能只是字符串或注释里的一个字母，那就白解析一遍——
 * 只亏时间，不亏正确性。实测本仓非白名单脚本块 114 个，此判据跳过 34 个（只看 `t(` 是跳过 32 个）。
 */
const TRANSLATION_IDENTIFIER = /(?<![\w$])t(?![\w$])/;

/**
 * 在「只求值一次、此后不再跟随 locale」的位置上调用 t()：用户之后切换语言，这些文案不动。
 * 判据见 statementHasEagerTranslationCall。
 *
 * 一条语句只报一次（语句里两处加载期 t() 合成一条），行号是语句起点、snippet 是那条语句的原文
 * （含末尾的分号）：读者要改的是整条语句，逐处列举会把同一处缺陷刷成好几条，也会把 snippet
 * 切碎到看不出上下文。
 *
 * **先做廉价预过滤**（见 TRANSLATION_IDENTIFIER）：正文里没有独立的 `t` 标识符就直接返回，
 * 不进解析器。代价是这类文件里的语法错误也不会由本函数报出来——本规则只对 t() 负责，
 * 那不在它的管辖范围。
 */
export function findModuleLoadT(source) {
  if (!TRANSLATION_IDENTIFIER.test(source)) return [];
  const sourceFile = parseModule(source);
  const hits = [];
  for (const statement of sourceFile.statements) {
    if (!statementHasEagerTranslationCall(statement)) continue;
    const start = statement.getStart(sourceFile);
    const text = source.slice(start, statement.end).replace(/\s+/g, " ").trim();
    hits.push({
      line: sourceFile.getLineAndCharacterOfPosition(start).line + 1,
      snippet: text.length > 80 ? `${text.slice(0, 80)}…` : text,
    });
  }
  return hits;
}

/**
 * HTML 入口里注释与标签之外的文本，命中中文即视为硬编码文案。
 * 规则 1 只扫 .ts/.vue，入口 HTML 的 <title> 与正文是同一类面向用户的文案，
 * 不扫就等于留了一条永久后门。
 */
export function findHtmlCjkText(source) {
  const text = source.replace(/<!--[\s\S]*?-->/g, "").replace(/<[^>]*>/g, "");
  return text.match(CJK_TOKEN) ?? [];
}

/**
 * 静态 lang 是运行时 setLocale 覆盖之前的初值，声明成不受支持的语言会让
 * 首帧与读屏软件拿到错误的语言。返回违规的取值，合规时返回 null。
 */
export function findUnsupportedHtmlLang(source, langs) {
  const m = source.match(/<html\b[^>]*?\slang\s*=\s*"([^"]*)"/i);
  if (!m) return "缺少 lang 属性";
  return langs.includes(m[1]) ? null : m[1];
}

function walk(dir, filter) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) out.push(...walk(full, filter));
    else if (filter(full)) out.push(full);
  }
  return out;
}

/**
 * 规则 8 的扫描域：`src/` 下的全部 `.vue`（即 `main()` 里规则 8 那一轮 walk 的文件集合）。
 *
 * 导出给 `find-english-blind.mjs` 派生规则 8 的允许清单时复用。清单要保证的是「规则 8 报得出的
 * 文本都在清单里」，故**文件集合与过滤条件都必须是同一份判据**；各写一遍 walk 就是第二份事实
 * 来源，两边一旦分叉（比如闸门改了过滤条件），清单会在无人察觉处与闸门对不上。
 */
export function listRule8Files(root = ROOT) {
  return walk(join(root, "src"), (p) => p.endsWith(".vue"));
}

function isPending(file, list) {
  const rel = relative(ROOT, file).replace(/\\/g, "/");
  return list.some((p) => rel.startsWith(p));
}

/**
 * 具名例外表是否命中该文件。**按整条路径精确比对，不做前缀匹配**——这是本表与 isPending 的
 * 唯一差别，且是刻意的。
 *
 * 前缀匹配（`rel.startsWith(p)`）在 RUST_PENDING / RUST_UNTRANSLATED 上是历史形态，那两张表
 * 曾经用过目录条目。本表**只收文件**，若也按前缀比，一条 `"src-tauri/src/commands/"` 就会
 * 同时遮蔽整棵子树：规则 2 与棘轮一起失效。**但「被遮蔽」不等于「闸门放行」**——目录条目会被
 * main() 的守卫报出来（它要求条目是一个存在的**文件**），少了那条守卫则连一条失败信息都打不
 * 出来：实测目录条目抛未捕获的 `EISDIR: illegal operation on a directory, read`、不存在的
 * 路径抛 `ENOENT: no such file or directory`，两者都落在棘轮的 readFileSync 上、都 exit 1。
 * 而 T2–T10 九批都会往本表里写条目。故这里**只有整条路径相等才算命中**。
 *
 * 命中不了目录条目的后果由 main() 的守卫接住：例外表的条目必须是一个**存在的文件**，
 * 目录条目与写错/被改名的失效条目都会被报出来（见那里）。
 */
function isExempted(file) {
  const rel = relative(ROOT, file).replace(/\\/g, "/");
  return RUST_PENDING_EXCEPTIONS.some((e) => e.path === rel);
}

/**
 * `.vue` 的非脚本区里有没有落单的反引号（把 script 块挖掉后仍有反引号配不成对）。
 *
 * 要守的是最坏的一类静默失明：模板正文里落单的反引号若与脚本区的反引号配了对，两区之间的
 * 真实代码——含整段脚本——会被当成一个字面量，规则 1/5 全部看不见它，而按块自查看不出来，
 * 因为块自己是对的。这是把自检收窄到脚本块时唯一会漏掉的跨区错位，故单独判一次。
 *
 * 判据：把 script 块挖成等长空白再扫一遍，非脚本区里落单的反引号这时必然吞到文件末尾。
 * 只看 `unterminated`——模板正文里的撇号与落单花括号是正文，在这一遍里同样会出现，不参与判据。
 * 反引号是唯一搭得了桥的字面量：单/双引号被同行规则限在一行内，块注释两侧都要求闭合符。
 *
 * 已知代价（刻意保留，不修）：判据只看「非脚本区有没有落单的反引号」，不看脚本区有没有反引号
 * 可配对。于是脚本区没有反引号时（没有 script 块，或反引号全在注释里）同样会报——那是一份
 * 合法文件上的响亮失败。今天树上 0 处，且要同时满足「模板正文里有**落单**的反引号」与
 * 「脚本区反引号数为奇」两个少见形态。不为此加更聪明的启发式：它守的是最坏的那类失败模式，
 * 把一个少见但响亮的误报换成整段脚本静默不被扫描，方向是反的；真出现再说。
 *
 * 挖块用 split/join 而非 replace：两块正文相同时 replace 只换掉一处，另一块就漏挖了。
 * 空块正文跳过——`"".split("")` 会把整份文件切碎。
 */
function templateRegionSwallowsScript(src, blocks) {
  let text = src;
  for (const block of blocks) {
    if (!block.text) continue;
    text = text.split(block.text).join(block.text.replace(/[^\n]/g, " "));
  }
  return scanJsComments(text, false).unterminated;
}

/**
 * 把 RULE1_FALSE_POSITIVE_LINES 命中的整行换成等长空白再交给规则 1。
 * 等长是为了保住行号：规则 1 的报错带路径不带行号，但同文件其它诊断（规则 5）要行号。
 *
 * 路径先归一化成正斜杠：清单里的 file 写的是仓库相对路径的正斜杠形式，而 Windows 检出的
 * relative() 给的是反斜杠，不归一化则逐条比对全部落空、具名豁免静默失效，
 * 闸门在一棵干净的树上 exit 1。与 isPending 同一处归一化，两处的清单格式才一致。
 */
function blankExemptedLines(relPath, src) {
  const rel = relPath.replace(/\\/g, "/");
  const rules = RULE1_FALSE_POSITIVE_LINES.filter((r) => r.file === rel);
  if (rules.length === 0) return src;
  return src
    .split("\n")
    .map((line) =>
      rules.some((r) => line.includes(r.line)) ? " ".repeat(line.length) : line
    )
    .join("\n");
}

function main() {
  ROOT = fileURLToPath(new URL("..", import.meta.url));
  const failures = [];
  const notes = [];

  // 剥注释自检：剥错位之后，错位点起的真实代码对规则 1/5 全部不可见，而闸门照常报「通过」。
  // 这是共享基础设施里的静默失明，先确认每个文件的剥注释结果可信，再拿它去跑规则；
  // 不可信就报出来并让闸门失败，而不是继续在错误的输入上跑规则。
  // 范围取规则 5 的那一份（src 下全部 .ts/.vue，含测试与白名单文件）：失明与文件迁没迁移无关。
  // .vue 只查 <script> 块：模板与样式不是脚本区，其中的撇号（Don't）与落单的花括号（50} off）
  // 是正文而不是代码，按 JS 剥必然误报。而规则 1/5 按整份文件剥时，那些撇号只可能影响它自己
  // 那一行（见 skipStringLiteral 的同行规则），脚本区仍完整地受这里保护。
  for (const f of walk(join(ROOT, "src"), (p) => /\.(ts|vue)$/.test(p))) {
    const src = readFileSync(f, "utf8");
    const isVue = f.endsWith(".vue");
    const blocks = isVue ? findVueScriptBlocks(src) : [];
    for (const region of isVue ? blocks.map((b) => b.text) : [src]) {
      const reason = findStripMisalignment(region);
      if (reason) failures.push(`剥注释结果不可信 ${relative(ROOT, f)}: ${reason}（该处之后的代码可能整段未被扫描）`);
    }
    if (isVue && templateRegionSwallowsScript(src, blocks)) {
      failures.push(
        `剥注释结果不可信 ${relative(ROOT, f)}: 脚本区之外有落单的反引号（该处之后的代码可能整段未被扫描）`
      );
    }
  }

  // 规则 1：前端不得新增硬编码中文（字符串字面量 + Vue 模板文本节点）
  const jsFiles = walk(join(ROOT, "src"), (f) => /\.(ts|vue)$/.test(f) && !f.endsWith(".test.ts"));
  let vueTotal = 0;
  let vueChecked = 0;
  let jsPending = 0;
  for (const f of jsFiles) {
    const isVue = f.endsWith(".vue");
    if (isVue) vueTotal += 1;
    if (isPending(f, FRONTEND_PENDING)) {
      jsPending += 1;
      continue;
    }
    if (isVue) vueChecked += 1;
    const rel = relative(ROOT, f);
    const src = blankExemptedLines(rel, readFileSync(f, "utf8"));
    const hits = findCjkLiterals(src);
    if (hits.length) failures.push(`硬编码中文 ${rel}: ${hits.slice(0, 3).join(" / ")}`);
    const textHits = isVue ? findVueTemplateCjkText(src) : [];
    if (textHits.length) {
      failures.push(`硬编码中文（模板文本）${rel}: ${textHits.slice(0, 3).join(" / ")}`);
    }
  }
  // 跳过多少文件必须出现在 CI 日志里：闸门看起来守住了前端、实际只查了一部分，
  // 比没有这条规则更糟。触发条件取「有文件被跳过」而非「一个都没查」——批次 2 起
  // 每批都会移出若干 .vue，后者会在仍有大批文件被跳过时提前静默。
  if (jsPending > 0) {
    notes.push(
      `rule 1：前端 ${jsPending}/${jsFiles.length} 个文件待迁移（白名单跳过），本规则在批次 2–5 前覆盖有限` +
        (vueChecked === 0 ? `；Vue 模板文本分支未检查任何文件（${vueTotal} 个 .vue 全部跳过）` : "")
    );
  }

  // 规则 2：后端非测试代码不得新增硬编码中文（日志宏除外）
  let rsTotal = 0;
  let rsPending = 0;
  for (const root of RUST_SCAN_ROOTS) {
    for (const f of walk(join(ROOT, root), (p) => p.endsWith(".rs"))) {
      rsTotal += 1;
      if (isPending(f, RUST_UNTRANSLATED)) continue;
      // 具名例外表：有非 CJK 义务的文件（纯测试文件、数据类文案…）不是待迁移欠债，
      // 与 RUST_UNTRANSLATED 在同一处放行。放在 RUST_PENDING 之前判：两张表都收的文件
      // 不该被算成「待迁移」——它恰恰是**不**打算迁的那一类。
      if (isExempted(f)) continue;
      if (isPending(f, RUST_PENDING)) {
        rsPending += 1;
        continue;
      }
      const src = readFileSync(f, "utf8");
      // 切片按每个 `#[cfg(test)]` 测试项的语法 span 遮蔽，不是按第一个标记一刀切：
      // 标记在本仓有 8 个文件里不出现在文件末尾，一刀切会让标记之后的生产代码对规则 2
      // 静默失明（见 findCjkRustProductionLiterals）。
      const hits = findCjkRustProductionLiterals(src);
      if (hits.length) failures.push(`硬编码中文 ${relative(ROOT, f)}: ${hits.slice(0, 3).join(" / ")}`);
    }
  }
  // 棘轮：白名单条目必须**仍然必要**。计划 8 结束时留着两条已零 CJK 的条目（最终评审发现）——
  // 那是 T1 修掉的那类缺陷的镜像：文件已经迁完，条目却还留着，规则 2 从此看不见它，
  // 之后写进去的中文无人过问。判据因此是「文件必须仍有生产 CJK」；零 CJK 就报出来，
  // 唯一的出口是 RUST_PENDING_EXCEPTIONS（具名非 CJK 义务），没有它就只剩删条目一条路。
  //
  // 覆盖的是**白名单条目**而不是扫描根下的文件：这正是本规则与规则 2 的分工——
  // 规则 2 守「没被跳过的文件里不许有中文」，棘轮守「被跳过的文件必须还值得被跳过」。
  //
  // 唯一的出口是 RUST_PENDING_EXCEPTIONS。**纯测试文件（`commands/session_index/tests.rs` 与
  // `parser/parse_bench.rs`）走的就是这条路**：它们整份是测试代码、没有义务，但探测器把夹具
  // 算成生产 CJK，故棘轮对它们**报不出任何东西**（判据是「文件仍有生产 CJK」，它们满足），
  // 于是两条条目合法地留在例外表里——表头把它们标注为**登记的假阳性**。自动认出这类文件的
  // 尝试已回退，理由见表头。
  for (const rel of RUST_PENDING) {
    if (isExempted(join(ROOT, rel))) continue;
    const full = join(ROOT, rel);
    // 「文件不在了」与「文件里没有中文」是两种失效，**原因与修法都不同**（前者多半是路径写错、
    // 文件被改名或删除，后者是迁完了），故分开报：合成一条会把读者引向错误的方向。
    // 判据落在**文件**上还有一处承重：目录条目（两张表历史上用过）不能走 readFileSync——
    // 它会抛 EISDIR 中断整轮诊断，同一轮里其它文件的违规全部消失（与 compareNamespaces 的守卫同理）。
    if (!existsSync(full) || !statSync(full).isFile()) {
      failures.push(
        `白名单条目指向的文件不存在（或不是文件）${rel}：条目已失效，请删除它` +
          "（这与「文件里已无中文」是两回事：多半是路径写错、文件被改名或删除）"
      );
      continue;
    }
    if (findCjkRustProductionLiterals(readFileSync(full, "utf8")).length === 0) {
      failures.push(
        `白名单条目已无生产 CJK ${rel}：该文件里的中文已迁完（或本就一条都没有），` +
          "留着这条等于让规则 2 看不见其中新写的中文；请删除该条目，" +
          "或在文件仍有非 CJK 义务（纯测试文件、数据类文案…）时写进 RUST_PENDING_EXCEPTIONS 并写明理由"
      );
    }
  }
  // 例外表的条目必须是**存在的文件**。它是各批把文件移出白名单后的指定去处（T2–T10 九批都会
  // 往里写），也是**唯一**会回头看「这条条目还指得准吗」的东西：残量棘轮只比对数字，对写错的
  // 路径没有意见（它只会在 readFileSync 上抛异常）。**少了这条守卫，这类条目会让整轮诊断当场
  // 崩掉**：实测删掉本 if 块后跑一次，目录条目抛未捕获的 `EISDIR: illegal operation on a
  // directory, read`，不存在的路径抛 `ENOENT: no such file or directory`，两者都在棘轮的
  // readFileSync 上、都 exit 1，且**一条失败信息都打不出来**。守卫把这两种崩溃换成一条逐条目
  // 的可读失败，条目「有具名义务」的声明才落得到实处（声明与事实不符就是无名豁免）。
  // 目录条目还要额外挡：精确比对下它谁也豁免不了（见 isExempted），但把它留在表里等于让下一个
  // 读者以为「那个目录已被豁免」。
  //
  // 这条守卫同时是**下面棘轮的护栏**，故它排在前面且命中即 `continue`：棘轮要 readFileSync 该
  // 路径，对目录抛 EISDIR、对不存在的路径抛 ENOENT，两者都会中断整轮诊断、让同一轮里其它文件的
  // 违规全部消失。先由守卫把「条目指向的不是一个文件」变成一条可读的失败，再跳过该条目的棘轮。
  for (const entry of RUST_PENDING_EXCEPTIONS) {
    const full = join(ROOT, entry.path);
    if (!existsSync(full) || !statSync(full).isFile()) {
      failures.push(
        `例外表条目不是已存在的文件 ${entry.path}：该条目豁免不了任何东西（例外表按整条路径精确比对），` +
          "多半是路径写错、文件被改名或删除，或写成了一条目录；请改正或删除它"
      );
      continue;
    }
    // 例外表棘轮：条目里写的 residual 必须等于该文件实测的生产 CJK 数。
    // 判据来自被判对象本身（findCjkRustProductionLiterals），不是条目里那个数：
    // 拿 residual 去比 residual 是恒真的，那样这条棘轮永远不会响。
    // 少了它，文件再长出中文、或残留被迁走，闸门都不会响——过期条目只能靠人重测浮出来。
    const actual = findCjkRustProductionLiterals(readFileSync(full, "utf8")).length;
    if (actual !== entry.residual) {
      failures.push(
        `例外表棘轮：${entry.path} 声称残 ${entry.residual} 条，实测 ${actual} 条（差 ${actual - entry.residual}）`
      );
    }
  }
  // 跳过多少文件必须出现在 CI 日志里：闸门看起来守住了后端、实际只查了一小部分，
  // 比没有这条规则更糟。提示点名扫描根而非说「后端」——比例的分母是扫描根之和，
  // 措辞不能大过实际范围。批次 6 收紧白名单后此提示自动消失。
  if (rsPending > 0) {
    notes.push(`rule 2：扫描根 ${RUST_SCAN_ROOTS.join("、")} 下 ${rsPending}/${rsTotal} 个文件待迁移（白名单跳过），本规则在批次 6 前覆盖有限`);
  }

  // 规则 3：语言包 key 集合与 en 一致（双向）
  //
  // 只走 en 的键是半个检查：目标语言里多出来的键静默通过，只存在于非 en 目录的
  // 命名空间文件根本不会被打开——而这是语言包形状的唯一守卫。故命名空间取所有语言的
  // 并集，两个方向都比：`en \ target` 报缺口，`target \ en` 报多余键。
  //
  // `_meta.json` 是**语言包元数据**（`reviewed` / `translatedFrom` / `reviewer`），不是命名空间，
  // 故下面按文件名排除它。**它的字段没有任何消费方**——没有代码读它们，它们是写给复核者的声明。
  // 正因为没人读，这里写下它们的契约，免得下一个改包的人凭字面猜：
  // - `translatedFrom` = **本包取值是从哪个包翻来的**；`null` 表示不是从别的包翻来的。
  //   `en` 是 `null`（它是基准）；`ja`/`de` 是 `"en"`（机翻打底、等社区校对，故 `reviewed: false`）；
  //   **`zh` 也是 `null`**——zh 的取值是**迁移前的原文逐字**（`errors.*` 的 zh 取自 Rust 中文原文，
  //   实测 193 键全部含中文、0 例外），它不是从 en 翻来的，en 才是那些句子的译文。
  //   ⚠️ **不要**把 zh 的 `translatedFrom` 改回 `"en"`：那个值会邀请复核者拿 zh 去对 en，
  //   把正确的中文当成错译「修」掉——正是 `scripts/check-zh-verbatim.mjs` 开头点名的那类假阳性方向。
  // - `reviewed: false`（zh 与 ja/de）说的是「**没有人工复核过这个包**」，与来源可靠性是两件事，
  //   不要因为它与机翻并列就顺手改成 `true`。
  const localeDir = join(ROOT, "src/locales");
  const langs = readdirSync(localeDir).filter((n) => statSync(join(localeDir, n)).isDirectory());
  const namespacesOf = (lang) =>
    existsSync(join(localeDir, lang))
      ? readdirSync(join(localeDir, lang)).filter((f) => f.endsWith(".json") && f !== "_meta.json")
      : [];
  const namespaces = [...new Set(langs.flatMap(namespacesOf))].sort();
  for (const ns of namespaces) {
    if (!existsSync(join(localeDir, "en", ns))) {
      const owners = langs.filter((l) => namespacesOf(l).includes(ns));
      failures.push(`语言包缺口 en/${ns}: 命名空间文件缺失（${owners.join("、")} 中存在）`);
      continue;
    }
    const base = JSON.parse(readFileSync(join(localeDir, "en", ns), "utf8"));
    for (const lang of langs.filter((l) => l !== "en")) {
      if (!existsSync(join(localeDir, lang, ns))) {
        failures.push(`语言包缺口 ${lang}/${ns}: 命名空间文件缺失`);
        continue;
      }
      const target = JSON.parse(readFileSync(join(localeDir, lang, ns), "utf8"));
      // 形状不一致（一侧是文本、另一侧是子键集合）单独报一条：那两个方向上的「缺口 /
      // 多余键」会把读者引向错误的方向（键两边都在，只是类型不同），且旧实现在这里会抛栈、
      // 让整轮诊断一起消失。
      // 但只**按 key 过滤**，不能整份命名空间跳过：形状对不上的只有 shape 里那几个路径，
      // 同一命名空间里其它键的缺口是各自独立的发现，跳掉就是把真缺陷静默丢弃——
      // 正是本文件已经修过两次的那类缺陷。
      const shape = findNamespaceShapeMismatches(base, target);
      const mismatched = new Set(shape);
      if (shape.length) {
        failures.push(
          `语言包形状不一致 ${lang}/${ns} 与 en/${ns}: ${shape.slice(0, 5).join(", ")}（共 ${shape.length}；同一键在一侧是文本、另一侧是子键集合）`
        );
      }
      const missing = compareNamespaces(base, target).filter((k) => !mismatched.has(k));
      if (missing.length) failures.push(`语言包缺口 ${lang}/${ns}: ${missing.slice(0, 5).join(", ")}（共 ${missing.length}）`);
      // 反向复用同一个函数：参数互换后，它列出的就是 target 有而 en 没有的键。
      const extra = compareNamespaces(target, base).filter((k) => !mismatched.has(k));
      if (extra.length) failures.push(`语言包多余键 ${lang}/${ns}: ${extra.slice(0, 5).join(", ")}（共 ${extra.length}）`);
    }
  }

  // 规则 3 的补充：同一个 key 在四份语言包里的 `{…}` 占位符集合必须一致。
  //
  // 上面那条只比**键集合**，看不见取值里的占位符：`{count}` 在 en 里写、在 de 里漏掉时，
  // 键集合逐份相等、闸门是绿的，而德语用户看到的是字面的 `{count}`——与裸 code 同类，
  // 都是把内部标识符推给用户看。
  //
  // **范围是全部命名空间**：只查 `errors.json` 时，数据类文案（`api-log` / `app` /
  // `session` / `common` 里的计数、路径、编码名等键）恰好整类落在管辖之外——它们不经过
  // `coded()`，规则 10 的站点判据也覆盖不到，于是那批键的占位符一致性没有任何东西在守。
  // 键名带命名空间前缀（`session.searchResult.matchedTitle`），故报出来的键能直接定位到文件。
  const allPacksByLang = {};
  for (const lang of langs) {
    allPacksByLang[lang] = {};
    for (const ns of namespaces) {
      if (!existsSync(join(localeDir, lang, ns))) continue;
      const name = ns.replace(/\.json$/, "");
      const pack = JSON.parse(readFileSync(join(localeDir, lang, ns), "utf8"));
      for (const [k, v] of flattenMessages(pack)) allPacksByLang[lang][`${name}.${k}`] = v;
    }
  }
  for (const hit of findPlaceholderMismatches(allPacksByLang, langs)) {
    failures.push(
      `占位符不一致 ${hit.key}（${hit.lang} 与 en）：en 有 ${hit.expected.join(" ") || "（无）"}，` +
        `${hit.lang} 有 ${hit.got.join(" ") || "（无）"}——该语言下缺失的占位符会原样渲染成 {…} 给用户看`
    );
  }

  // 规则 4：HTML 入口的静态文案与语言标记
  const htmlFiles = [
    ...readdirSync(ROOT)
      .filter((n) => n.endsWith(".html"))
      .map((n) => join(ROOT, n)),
    ...walk(join(ROOT, "src"), (f) => f.endsWith(".html")),
  ];
  for (const f of htmlFiles) {
    const src = readFileSync(f, "utf8");
    const rel = relative(ROOT, f);
    const hits = findHtmlCjkText(src);
    if (hits.length) failures.push(`硬编码中文 ${rel}: ${hits.slice(0, 3).join(" / ")}`);
    const badLang = findUnsupportedHtmlLang(src, langs);
    if (badLang) failures.push(`语言标记不受支持 ${rel}: ${badLang}`);
  }

  // 规则 5：src/ 中 t() 引用的 key 必须在 en 包里存在。
  // missingWarn 关掉后 vue-i18n 对缺失 key 返回 key 本身，写错的路径会原样渲染到界面上；
  // 用 t() 写的断言也抓不到——组件和断言两边拿到同一串路径，断言照常通过。
  // 命名空间复用规则 3 的扫描结果（语言包按 <lang>/<namespace>.json 组织，文件名即 key 首段）。
  // 白名单不适用：待迁移文件里已有的 t() 同样要引用真实 key，规则 1 的白名单理由（存量硬编码）
  // 在这里不成立。.test.ts 也不排除——错别字最典型的藏身处正是「组件与断言写同一个错 key」，
  // 而规则 1 排除测试文件的理由是夹具里本就有中文样例数据，与 key 是否存在无关。
  const enMessages = {};
  for (const ns of namespaces) {
    if (!existsSync(join(localeDir, "en", ns))) continue;
    enMessages[ns.replace(/\.json$/, "")] = JSON.parse(readFileSync(join(localeDir, "en", ns), "utf8"));
  }
  for (const f of walk(join(ROOT, "src"), (p) => /\.(ts|vue)$/.test(p))) {
    const rel = relative(ROOT, f);
    for (const ref of findTranslationKeyRefs(readFileSync(f, "utf8"))) {
      if (!ref.dynamic && MISSING_KEY_PROBES.includes(ref.key)) continue;
      const where = resolveMessageKey(enMessages, ref.key);
      if (ref.dynamic ? where !== "branch" : where !== "leaf") {
        const verdict = where === "missing" ? "不存在" : where === "leaf" ? "指向文本" : "指向子键集合";
        const what = ref.dynamic ? `动态 key 前缀${verdict}` : `引用的翻译 key ${verdict}`;
        failures.push(`${what} ${rel}:${ref.line}: ${ref.key}`);
      }
    }
  }

  // 规则 6：把 i18n key 当数据存放的文件（src/changelog.ts）里的 key 同样要在 en 包里解析到叶子。
  // 与规则 5 同源，只是 key 的写法不是 t("…") 而是数组里的字符串；复用同一套解析与判定。
  const dataKeyFiles = ["src/changelog.ts"];
  for (const rel of dataKeyFiles) {
    const full = join(ROOT, rel);
    if (!existsSync(full)) continue;
    for (const ref of findDataKeyRefs(readFileSync(full, "utf8"))) {
      const where = resolveMessageKey(enMessages, ref.key);
      if (where !== "leaf") {
        const verdict = where === "missing" ? "不存在" : "指向子键集合";
        failures.push(`数据里的翻译 key ${verdict} ${rel}:${ref.line}: ${ref.key}`);
      }
    }
  }

  // 规则 7：只求值一次的 t()（.ts 在 import 时、.vue 的 <script setup> 在组件挂载时，
  // 见 findModuleLoadT 的说明）。语言包文案必须在渲染时求值，否则用户切换语言后这些文案不动。
  // 这一类缺陷在本分支上被反复漏掉，闸门是唯一能挡住它回归的地方。
  // 白名单与规则 1 同一份：待迁移文件里的存量问题随各自任务处理，文件一旦移出白名单，
  // 本规则即开始保护它。测试文件按规则 1 的理由排除（夹具里本就有意构造的样例，
  // 且这类 t() 不面向界面）；规则 5 收测试文件是因为「key 是否存在」与夹具数据无关。
  for (const f of walk(join(ROOT, "src"), (p) => /\.(ts|vue)$/.test(p) && !p.endsWith(".test.ts"))) {
    if (isPending(f, FRONTEND_PENDING)) continue;
    const rel = relative(ROOT, f);
    const src = readFileSync(f, "utf8");
    const blocks = f.endsWith(".vue") ? findVueScriptBlocks(src) : [{ text: src, line: 1 }];
    for (const block of blocks) {
      let hits;
      try {
        hits = findModuleLoadT(block.text);
      } catch (err) {
        // 解析不出来就报出来，绝不跳过：跳过等于这份文件的加载期 t() 无人检查，而闸门照常报
        // 「通过」——那正是本文件六次缺陷的共同形状。
        // 逐块 catch 而不是让异常冒出去：抛出会中断 main()，同一轮里其它文件的违规全部消失，
        // 闸门从「漏报一处」变成「静默丢弃整轮诊断」（与 compareNamespaces 里那条守卫同理）。
        // 行号折算成**文件**行号，与命中报出的行号同一个算法：.vue 的脚本块不在文件开头，
        // 直接报块内行号会把读者指到模板上（报 `src/Comp.vue:4` 而错误其实在第 6 行）。
        const at = block.line + (err.line ?? 1) - 1;
        const why = err.detail ? `${rel}:${at}:${err.column}：${err.detail}` : `${rel}：${err.message}`;
        failures.push(`规则 7 无法解析 ${why}（该文件的加载期 t() 未被检查）`);
        continue;
      }
      for (const hit of hits) {
        failures.push(`模块加载/组件挂载时求值一次的 t() ${rel}:${block.line + hit.line - 1}: ${hit.snippet}`);
      }
    }
  }

  // 规则 8：模板文本节点与静态文本属性里**新增**的英文界面文案。规则 1 只认 CJK 码点，
  // 这类改动对闸门完全不可见，而这个类已经因此漏过两次（计划 2 与计划 5 的最终评审）。
  // 允许清单的性质、派生方式与代价见 findUnlistedEnglishTexts 上方的说明。
  // 与规则 1/7 不同，本规则**不套用前端白名单**：白名单记的是存量中文欠债（今天只剩一个 .ts），
  // 而英文文案是新增即违规——文件在不在白名单里都不改变这一点。
  const allowlist = loadEnglishAllowlist();
  const reasonless = Object.entries(allowlist).filter(([, reason]) => typeof reason !== "string" || !reason.trim());
  if (reasonless.length) {
    const named = reasonless.slice(0, 3).map(([text]) => text).join(" / ");
    failures.push(
      `规则 8 允许清单缺理由（${named}${reasonless.length > 3 ? ` 等 ${reasonless.length} 条` : ""}）：` +
        "它是裁决集的投影，每条必须写明属哪一类（品牌名/格式名/标识符/协议字段名/键盘键名…）"
    );
  }
  for (const f of listRule8Files()) {
    const rel = relative(ROOT, f);
    let hits;
    try {
      hits = findUnlistedEnglishTexts(readFileSync(f, "utf8"), allowlist);
    } catch (err) {
      // 解析不出来就报出来，绝不跳过：跳过等于这份文件的模板文本无人检查，而闸门照常报
      // 「通过」——那正是本文件六次缺陷的共同形状。逐文件 catch 而不是让异常冒出去，
      // 理由与规则 7 相同：抛出会中断 main()，同一轮里其它文件的违规全部消失。
      failures.push(`规则 8 无法解析 ${rel}：${err.message}（该文件的模板文本与静态属性未被检查）`);
      continue;
    }
    if (hits.length) failures.push(`新增英文界面文案（模板文本/静态属性）${rel}: ${hits.slice(0, 3).join(" / ")}`);
  }
  // 脚本区敞口必须出现在 CI 日志里，理由同规则 1/2 的跳过提示：批量执行者跑的是闸门、
  // 看的是它的输出，**不会去读本文件的注释**——写在注释里的限制等于没有这个限制。
  // 规则 8 覆盖不到 `<script>` 与 `.ts` 的英文字面量（为什么刻意不覆盖见 findUnlistedEnglishTexts），
  // 实测本计划迁走的 24 条里 7 条在 `.ts` 里，故这个向量只能靠重跑测量兜。
  // 无触发条件：它与「今天有没有违规」无关，是规则本身的覆盖边界，每轮都该看见。
  notes.push(
    "rule 8：不覆盖脚本区（<script> 与 .ts 里的英文字面量），新增批次前须重跑 node scripts/find-english-blind.mjs"
  );

  // 规则 9：同一 `en` 取值下 ja/de 的译文漂移。合并 / 统一 / 拆身份只修好了现状，
  // 本规则是那条**棘轮**——判据、允许清单的派生方式与「zh 不参与」的理由见 findUnlistedDrift。
  // 与规则 8 一样**不套用前端白名单**：漂移是「新增即违规」，文件在不在白名单里都不改变这一点。
  const driftAllowlist = loadDriftAllowlist();
  const reasonlessDrift = Object.entries(driftAllowlist).filter(
    ([, reason]) => typeof reason !== "string" || !reason.trim()
  );
  if (reasonlessDrift.length) {
    const named = reasonlessDrift.slice(0, 3).map(([en]) => en).join(" / ");
    failures.push(
      `规则 9 允许清单缺理由（${named}${reasonlessDrift.length > 3 ? ` 等 ${reasonlessDrift.length} 条` : ""}）：` +
        "它是裁决集的投影，每条必须写明属哪一类（两条不同的源文案 / 上下文语义不同 / …）"
    );
  }
  // 只读规则 9 参与的三语；`zh` 连读都不读，故它不可能被卷进判据（见 DRIFT_LANGS）。
  const driftPacks = {};
  for (const lang of DRIFT_LANGS) {
    driftPacks[lang] = {};
    for (const ns of namespaces) {
      if (!existsSync(join(localeDir, lang, ns))) continue;
      const name = ns.replace(/\.json$/, "");
      const pack = JSON.parse(readFileSync(join(localeDir, lang, ns), "utf8"));
      for (const [k, v] of flattenMessages(pack)) driftPacks[lang][`${name}.${k}`] = v;
    }
  }
  for (const hit of findUnlistedDrift(driftPacks, driftAllowlist)) {
    const keys = hit.keys.slice(0, 5).join(" / ");
    failures.push(
      `译文漂移（同一 en 值下 ja/de 不一致）${keys}` +
        `${hit.keys.length > 5 ? ` 等 ${hit.keys.length} 个键` : ""}：en=${JSON.stringify(hit.en)}` +
        `（ja ${hit.ja.length} 种取值 / de ${hit.de.length} 种取值）——同一 en 值下 ja/de 必须一致；` +
        "若这确实是两条不同的源文案，先把该 en 值写进 scripts/i18n-drift-adjudication.json，" +
        "再跑 node scripts/gen-drift-allowlist.mjs 派生允许清单"
    );
  }

  // 规则 10：`coded()` 的 code 与 `errors.*` 语言包条目**双向闭合**。
  //
  // 这条规则的存在理由：后端不再产出面向用户的文案，只发 `{ code, params }`，
  // 前端按 `errors.<code>` 取当前语言的文案。**码表由前端语言包定义，Rust 侧穷举不出来**
  // （R2 的注释明写），所以「这个 code 在语言包里存在吗」没有任何编译期检查——
  // 缺条目时 `renderAppError` 回退成**裸 code 上屏**（`system_ops.path_missing`），
  // 不崩、不报错，只是把内部标识符给用户看。
  //
  // **规则 5 不是这条路径的守卫**：`src/utils/invokeApp.ts` 把键提成
  // `const key = \`errors.${err.code}\`` 再当**变量**传给 `t()`，而规则 5 只认
  // 字面量与模板实参的静态前缀——它结构上覆盖不到这里，改也改不了（键在运行时才拼出来）。
  // 故规则 10 是这条路径唯一的守卫，别以为规则 5 已经管了。
  //
  // **不套用 RUST_PENDING / RUST_UNTRANSLATED**：那两张表管的是规则 2 的存量 CJK 欠债
  // （迁完一批删一条），与「这个 code 有没有语言包条目」是两件事。实测反证：T3 引入的四个
  // code 全部落在白名单文件里（`streaming.rs`、`commands/session.rs`、`assistant/commands.rs`），
  // 套用白名单会让前向判据一个站点都看不到、反向判据把四个条目全报成孤儿。
  //
  // 判据的两个方向与「已知缺口」见 findCodePackMismatches / findCodedWithKeyMismatches
  // （后者补的是「四份语言包彼此一致、站点却对不上」那个缺口）。
  // 语言包**彼此**的占位符一致性不在这条规则里：它由规则 3 之后的检查覆盖全部命名空间。
  const errorPackByLang = {};
  for (const lang of langs) {
    const file = join(localeDir, lang, "errors.json");
    errorPackByLang[lang] = existsSync(file)
      ? Object.fromEntries(flattenMessages(JSON.parse(readFileSync(file, "utf8"))))
      : {};
  }
  const codeSites = [];
  const unverifiedSites = [];
  for (const root of RUST_SCAN_ROOTS) {
    for (const f of walk(join(ROOT, root), (p) => p.endsWith(".rs"))) {
      const rel = relative(ROOT, f);
      for (const hit of findCodedCodeLiterals(readFileSync(f, "utf8"))) {
        if (hit.code === null) {
          failures.push(
            `规则 10：coded() 的实参不是字符串字面量 ${rel}:${hit.line}` +
              "（R2 的 &'static str 挡不住变量，它指向哪个 code 无从校验，故报出而不是静默跳过）"
          );
          continue;
        }
        codeSites.push({ code: hit.code, line: hit.line, file: rel, keys: hit.keys });
        if (hit.keys === null) unverifiedSites.push(`${rel}:${hit.line}`);
      }
    }
  }
  const { missing, orphaned } = findCodePackMismatches(
    codeSites.map((s) => s.code),
    // `?? {}`：语言目录里没有 en 时 `errorPackByLang.en` 是 undefined，直接 `Object.keys`
    // 会抛栈——而抛出会中断 main()，同一轮里其它文件的违规全部消失（同 compareNamespaces 的守卫）。
    Object.keys(errorPackByLang.en ?? {})
  );
  for (const code of missing) {
    const at = codeSites
      .filter((s) => s.code === code)
      .map((s) => `${s.file}:${s.line}`)
      .join(" / ");
    failures.push(
      `错误码未登记 errors.${code}（${at}）：四份语言包里都没有该条目，前端 renderAppError 会回退成裸 code 上屏`
    );
  }
  for (const key of orphaned) {
    failures.push(
      `errors.${key} 没有任何 coded() 站点产出：语言包条目会漂移成手工维护的清单；` +
        "若站点刚被删掉或改名，请一并删掉四份语言包里的该键"
    );
  }
  // 四份语言包**彼此**的占位符一致性不在这里查：那条覆盖全部命名空间（含 `errors`），
  // 已随规则 3 一起跑（见那里的注释）。本规则的两段判据只比键集合与站点，看不见取值里的占位符。
  // 四份语言包彼此一致**不等于**站点与语言包一致：四份都写 `{path}`、站点却写
  // `.with("pfad", …)` 时，上一条判据是绿的，而用户看到的仍是字面的 `{path}`。
  // 这条把站点一侧的键名与 `en` 条目的 `{…}` 比一遍（判据与收窄的理由见 findCodedWithKeyMismatches）。
  for (const hit of findCodedWithKeyMismatches(codeSites, errorPackByLang.en ?? {})) {
    failures.push(
      `规则 10：占位符不一致 errors.${hit.code}（${hit.file}:${hit.line}）：` +
        `站点 with() 提供 ${hit.got.join(" ") || "（无）"}，en 语言包声明 ${hit.expected.join(" ") || "（无）"}` +
        "——语言包声明而站点没提供的那个占位符会原样渲染成 {…} 给用户看"
    );
  }
  // 判据的缺口必须出现在 CI 日志里，理由同规则 1/2/8 的跳过提示：批量执行者跑的是闸门、
  // 看的是它的输出，**不会去读本文件的注释**——写在注释里的限制等于没有这个限制。
  // 计数的是「键名静态判不出来」的站点（没写 with()，或 with() 与 coded() 不在同一表达式里，
  // 或 with() 的实参不是字面量）：它们**没有被校验**，而这条规则看起来像是在校验全部站点。
  if (unverifiedSites.length > 0) {
    const named = unverifiedSites.slice(0, 3).join(" / ");
    notes.push(
      `rule 10：${unverifiedSites.length}/${codeSites.length} 个 coded() 站点的 with() 键名静态判不出来` +
        `（站点没写 with()，或 with() 与 coded() 不在同一表达式里、实参不是字面量），其占位符未校验：${named}` +
        `${unverifiedSites.length > 3 ? ` 等 ${unverifiedSites.length} 处` : ""}`
    );
  }

  if (notes.length) console.log(notes.map((n) => `  · ${n}`).join("\n"));
  if (failures.length) {
    console.error("i18n 闸门未通过：\n" + failures.map((f) => `  ✗ ${f}`).join("\n"));
    process.exit(1);
  }
  console.log("i18n 闸门通过");
}

// 用 realpath 比较：经由软链接路径调用时（macOS 的 /tmp、被链接的检出目录），
// argv[1] 的写法与 import.meta.url 解析出的真身不同，直接比较会让 main() 静默不执行——
// 闸门一声不响地退出 0，比不跑更糟。
if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
  main();
}
