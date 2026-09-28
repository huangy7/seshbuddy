//! Claude JSONL 条目的类型化反序列化层。
//!
//! ## 为什么需要它
//!
//! 样本实测（1 GB 真实会话）：
//! - `tool_result` 正文占**整个文件的 93.1%**（1000 MB / 1074 MB），
//!   而 50 KB 截断后只需要 97.5 MB —— 约 **900 MB 的分配、拷贝与反转义是纯浪费**；
//! - 其余所有字段加起来不足全文件 1%（`toolUseResult` 1.5 MB、`uuid` 0.5 MB、`timestamp` 0.3 MB…）。
//!
//! ## 设计取舍（都是被实测数字逼出来的）
//!
//! 1. **不再为每一行建 `serde_json::Value` 树**，只把 `message.content` 当作需要特殊处理的字段；
//!    其余小字段保留 `Value` 语义，与改造前逐字段等价（原因见 [`Field`]）。
//! 2. **单遍解析，绝不对同一段字节重复扫描**。这是本层最容易踩的坑：
//!    嵌套 `RawValue`（先捕获数组、再逐元素捕获、再逐元素重新解析）会让同一份正文
//!    被扫 3 遍；再加上「先扫一遍判断是否超长 → 复制成字面量 → 再交给 serde_json 反转义」，
//!    总共要碰 6 遍字节 —— 实测比原来的 `Value` 路径**还慢 1.4 倍**。
//!    现在的做法是：内容块数组一次遍历成型（[`RawPart`] 手工 map 访问器），
//!    正文在**反转义的过程中**就产出结果并随时中止（[`decode_prefix`]），
//!    批量片段用 `str::find` 走 memchr 快路径整段拷贝。
//! 3. **截断发生在反转义过程中**，而不是"先物化 1000 MB 再截到 50 KB"。
//!
//! ## 与改造前的差异
//!
//! 只有一处，且落在"输入本身异常"的范畴：**字符串字段都以裸片段承接**，因此其中的坏转义
//! （如孤立代理对 `\uD800`）不会被 `serde_json` 校验 —— 它只会让**该 part** 消失，
//! 而不是像改造前那样拖垮整行。降级方式是丢字段而非产出静默错值。
//!
//! 此外，同一个对象出现**重复键**时整行按不可解析丢弃（旧的 `Value` 路径是后者胜出）——
//! 重复键无法由 JSON 序列化器产生，会话文件是机器写入的。
//! 以上均有单测钉住，真实样本上的等价性测试（12,081 行 / 24,162 次比对）证明实际数据零分歧。

use crate::session::{tool_use_summary, ContentPart};
use serde::de::{Deserializer, Error as DeError, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::Value;
use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

/// 单条 tool_result 正文的截断上限 —— 与改造前保持一致（改这里等于改用户可见行为）。
pub(crate) const TOOL_RESULT_MAX_LEN: usize = 50_000;

/// 懒加载预览长度（设计稿 §4 拍板：2 KB）。
/// 超过此长度且未命中白名单的 tool_result 只带前 2 KB 预览 + 定位信息，
/// 全文由 `get_tool_result_full_content` 命令按 `source_offset` 按需回取。
pub(crate) const TOOL_RESULT_PREVIEW_LEN: usize = 2_000;

/// 白名单标记 —— 命中即带全文（`truncated_preview=false`），与前端探测口径逐字一致：
/// - `<svg` / `widget_code`：`svg.ts:22` 的 widget 部件探测（行号在 `987b91a0` 上量得）
/// - `visualizer_show_widget_result`：`toolPartScan.ts` 的 `VISUALIZER_ACK_MARKER`
/// - `[Tool Error]`：`toolPartScan.ts` 的 `TOOL_ERROR_MARKER`
///
/// 前端探测发生在「截断后 50 KB 全文」上，故后端也在同一载体上探测：
/// 只要 50 KB 内含标记就带全文，widget 渲染 / 错误样式不会因懒加载失效。
const TOOL_RESULT_FULL_MARKERS: [&str; 4] = [
    "<svg",
    "widget_code",
    "visualizer_show_widget_result",
    "[Tool Error]",
];

/// 「键是否存在」的精确载体。
///
/// 改造前一律用 `entry.get(key)` 取值：**键存在（哪怕值是 `null`）就会进入后续分支**。
/// 若直接换成 `Option<Value>`，`"usage": null` 会被折叠成「键不存在」，
/// 于是 `token_usage` 从 `Some(全 0)` 悄悄变成 `None` —— 这类静默语义漂移正是本层最大的风险来源，
/// 故用一个薄包装把区分保留下来。
pub(crate) struct Field<T>(Option<T>);

impl<T> Default for Field<T> {
    fn default() -> Self {
        Field(None)
    }
}

impl<'de, T> Deserialize<'de> for Field<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // 只有键真的出现时 serde 才会调用到这里，因此 `Some` 即「键存在」
        T::deserialize(deserializer).map(|value| Field(Some(value)))
    }
}

impl<T> Field<T> {
    pub(crate) fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

/// Claude JSONL 顶层条目：只声明 `parse_jsonl_line` 真正用到的字段。
#[derive(Deserialize)]
pub(crate) struct ClaudeEntry<'a> {
    #[serde(rename = "type", default)]
    entry_type: Field<Value>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: Field<Value>,
    #[serde(rename = "isMeta", default)]
    is_meta: Field<Value>,
    #[serde(default)]
    uuid: Field<Value>,
    #[serde(default)]
    timestamp: Field<Value>,
    #[serde(rename = "toolUseResult", default)]
    tool_use_result: Field<Value>,
    #[serde(default, borrow)]
    message: Field<ClaudeMessage<'a>>,
}

#[derive(Deserialize)]
pub(crate) struct ClaudeMessage<'a> {
    #[serde(default)]
    role: Field<Value>,
    #[serde(default)]
    model: Field<Value>,
    #[serde(default)]
    usage: Field<Value>,
    /// 全文件的 93% 都在这个字段里，**刻意不建树**。
    /// `borrow` 是必须的：serde 只会为 `&'a str` / `Cow<'a, str>` 自动推导 `'de: 'a`，
    /// 自定义的 `MessageContent<'a>` 需要显式声明。
    #[serde(default, borrow)]
    content: Field<MessageContent<'a>>,
}

impl<'a> ClaudeEntry<'a> {
    /// 解析一行 JSONL。等价于改造前的 `serde_json::from_str::<Value>(line).ok()`，
    /// 但不会把 `message.content` 物化成树。
    pub(crate) fn parse(line: &'a str) -> Option<Self> {
        serde_json::from_str::<Self>(line).ok()
    }

    /// 等价于改造前的 `entry.get("type").and_then(|v| v.as_str()).unwrap_or("")`。
    pub(crate) fn entry_type(&self) -> &str {
        value_as_str(self.entry_type.get())
    }

    /// 等价于 `entry.get("isSidechain").and_then(|v| v.as_bool()).unwrap_or(false)`。
    pub(crate) fn is_sidechain(&self) -> bool {
        self.is_sidechain
            .get()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// 等价于 `entry.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false)`。
    pub(crate) fn is_meta(&self) -> bool {
        self.is_meta.get().and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// 等价于 `entry.get("uuid").and_then(|v| v.as_str()).map(|s| s.to_string())`。
    pub(crate) fn uuid(&self) -> Option<String> {
        self.uuid
            .get()
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// 等价于 `entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("")`。
    pub(crate) fn timestamp(&self) -> String {
        value_as_str(self.timestamp.get()).to_string()
    }

    /// 等价于 `entry.get("toolUseResult").and_then(|v| v.get("agentId")).and_then(|v| v.as_str())`。
    pub(crate) fn subagent_id(&self) -> Option<&str> {
        self.tool_use_result
            .get()
            .and_then(|v| v.get("agentId"))
            .and_then(|v| v.as_str())
    }

    /// 等价于 `entry.get("message")`：`None` 表示键缺失。
    pub(crate) fn message(&self) -> Option<&ClaudeMessage<'a>> {
        self.message.get()
    }
}

impl<'a> ClaudeMessage<'a> {
    /// 等价于 `message.get("role").and_then(|v| v.as_str())`。
    pub(crate) fn role(&self) -> Option<&str> {
        self.role.get().and_then(|v| v.as_str())
    }

    /// 等价于 `message.get("model").and_then(|v| v.as_str()).map(|s| s.to_string())`。
    pub(crate) fn model(&self) -> Option<String> {
        self.model
            .get()
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// 等价于 `message.get("usage")`：`None` 表示键缺失（键存在但为 null 时返回 `Some(Null)`）。
    pub(crate) fn usage(&self) -> Option<&Value> {
        self.usage.get()
    }

    /// 会话正文。`None` 表示键缺失（与"键存在但不可解析"一样产出空内容）。
    pub(crate) fn content(&self) -> Option<&MessageContent<'a>> {
        self.content.get()
    }
}

fn value_as_str(value: Option<&Value>) -> &str {
    value.and_then(|v| v.as_str()).unwrap_or("")
}

/// `message.content` 的三种形态：整段字符串（用户正文）、内容块数组、其它（一律视为空）。
pub(crate) enum MessageContent<'a> {
    Text(Cow<'a, str>),
    Parts(Vec<RawPart<'a>>),
    Empty,
}

impl<'de: 'a, 'a> Deserialize<'de> for MessageContent<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MessageContentVisitor(PhantomData))
    }
}

struct MessageContentVisitor<'a>(PhantomData<&'a ()>);

impl<'de: 'a, 'a> Visitor<'de> for MessageContentVisitor<'a> {
    type Value = MessageContent<'a>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string or an array of content blocks")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Text(Cow::Owned(value.to_string())))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Text(Cow::Borrowed(value)))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Text(Cow::Owned(value)))
    }

    /// 内容块数组：一次遍历直接成型，不再"先取裸片段、再逐个重新解析"。
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut parts = Vec::new();
        while let Some(part) = seq.next_element::<RawPart<'a>>()? {
            parts.push(part);
        }
        Ok(MessageContent::Parts(parts))
    }

    /// 顶层是对象：改造前 `as_str()`/`as_array()` 都取不到 → 空内容
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(MessageContent::Empty)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(MessageContent::Empty)
    }
}

/// 单个内容块（数组元素）。
///
/// 所有字段都用 `&RawValue`（裸片段）承接：裸片段捕获**永不因类型不符而失败**，
/// 与改造前 `item.get(k).and_then(as_str/as_bool/...)` 的宽容语义一一对应 ——
/// 字段类型不对时退化为「取不到」，而不是让整行解析失败。
///
/// 之所以手写 `Deserialize` 而不用 `derive`：`derive` 遇到非对象元素会直接判定整个数组
/// 解析失败，而改造前 `item.get(..)` 对非对象一律取不到、等价于跳过该元素。
/// 顺带的好处是重复键变成"后者胜出"，与旧 `Value` 语义一致。
#[derive(Default)]
pub(crate) struct RawPart<'a> {
    part_type: Option<&'a RawValue>,
    text: Option<&'a RawValue>,
    thinking: Option<&'a RawValue>,
    id: Option<&'a RawValue>,
    name: Option<&'a RawValue>,
    /// 唯一需要区分「键存在但为 null」的字段：`input: null` 与 `input` 缺失
    /// 会分别产出展示串 `"null"` 与 `"{}"`，两者不可混同。
    input: Field<&'a RawValue>,
    is_error: Option<&'a RawValue>,
    content: Option<&'a RawValue>,
    source: Option<&'a RawValue>,
    tool_use_id: Option<&'a RawValue>,
}

impl<'de: 'a, 'a> Deserialize<'de> for RawPart<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawPartVisitor(PhantomData))
    }
}

struct RawPartVisitor<'a>(PhantomData<&'a ()>);

impl<'de: 'a, 'a> Visitor<'de> for RawPartVisitor<'a> {
    type Value = RawPart<'a>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Claude content block")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut part = RawPart::default();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match key.as_ref() {
                "type" => part.part_type = map.next_value()?,
                "text" => part.text = map.next_value()?,
                "thinking" => part.thinking = map.next_value()?,
                "id" => part.id = map.next_value()?,
                "name" => part.name = map.next_value()?,
                "input" => part.input = map.next_value()?,
                "is_error" => part.is_error = map.next_value()?,
                "content" => part.content = map.next_value()?,
                "source" => part.source = map.next_value()?,
                "tool_use_id" => part.tool_use_id = map.next_value()?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(part)
    }

    // 非对象元素：改造前 `item.get(..)` 一律取不到 → 等价于「不是一个内容块」
    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_borrowed_str<E>(self, _: &'de str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_string<E>(self, _: String) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(RawPart::default())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        RawPart::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(RawPart::default())
    }
}

/// 把 `message.content` 转成渲染用的部件列表。
/// 与改造前 `parse_content(Option<&Value>)` 的分支顺序、过滤条件逐条对应。
pub(crate) fn content_parts(content: &MessageContent<'_>) -> Vec<ContentPart> {
    match content {
        MessageContent::Empty => Vec::new(),
        MessageContent::Text(text) => {
            // 顶层字符串**不截断**（与改造前一致）；注意此处没有空串过滤，
            // `content: ""` 会产出一个空 Text part 并保留该消息
            match crate::parser::claude::try_parse_collapsible(text) {
                Some(part) => vec![part],
                None => vec![ContentPart::Text {
                    text: text.to_string(),
                }],
            }
        }
        MessageContent::Parts(parts) => parts.iter().filter_map(build_part).collect(),
    }
}

/// 取第一个内容块的 `tool_use_id` —— 子 Agent 归属映射用。
/// 改造前是 `content[0].tool_use_id`（要求 content 是数组），故 `Text`/`Empty` 一律 `None`。
pub(crate) fn first_tool_use_id(content: &MessageContent<'_>) -> Option<String> {
    match content {
        MessageContent::Parts(parts) => {
            let first = parts.first()?;
            first.tool_use_id.and_then(raw_as_str).map(Cow::into_owned)
        }
        _ => None,
    }
}

fn build_part(part: &RawPart<'_>) -> Option<ContentPart> {
    // `type` 缺失或非字符串 → 改造前 item_type == "" → 命中 `_ =>` 分支被跳过
    let kind = part.part_type.and_then(raw_as_str)?;
    match kind.as_ref() {
        "text" => {
            let text = raw_as_str(part.text?)?;
            if text.is_empty() {
                return None;
            }
            Some(match crate::parser::claude::try_parse_collapsible(&text) {
                Some(part) => part,
                None => ContentPart::Text {
                    text: text.into_owned(),
                },
            })
        }
        "tool_use" => {
            let name = part
                .name
                .and_then(raw_as_str)
                .unwrap_or_else(|| Cow::Borrowed("Tool"));
            let tool_use_id = part.id.and_then(raw_as_str).map(Cow::into_owned);
            // `input` 是任意 JSON（样本里合计仅 0.3 MB），沿用 Value 语义：
            // 摘要与展示串都需要它，且 `null`/缺失要分别产出 `null` 与 `{}`。
            let input_val = match part.input.get() {
                Some(raw) => serde_json::from_str::<Value>(raw.get().trim())
                    .unwrap_or(Value::Object(serde_json::Map::new())),
                None => Value::Object(serde_json::Map::new()),
            };
            let summary = tool_use_summary(&name, &input_val);
            let input_str = serde_json::to_string_pretty(&input_val).unwrap_or_default();
            Some(ContentPart::ToolUse {
                summary,
                tool_name: name.into_owned(),
                input: input_str,
                tool_use_id,
            })
        }
        "thinking" => {
            let thinking = raw_as_str(part.thinking?)?;
            if thinking.is_empty() {
                return None;
            }
            Some(ContentPart::Thinking {
                thinking: thinking.into_owned(),
            })
        }
        "image" => {
            // 样本中 0 字节；base64 正文本来就要完整保留，故直接用 Value 语义
            let source = serde_json::from_str::<Value>(part.source?.get().trim()).ok()?;
            let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if source_type != "base64" {
                return None;
            }
            let media_type = source
                .get("media_type")
                .and_then(|v| v.as_str())
                .unwrap_or("image/png")
                .to_string();
            let data = source.get("data").and_then(|v| v.as_str())?.to_string();
            Some(ContentPart::Image { media_type, data })
        }
        "tool_result" => {
            let is_error = part.is_error.and_then(raw_as_bool).unwrap_or(false);
            let (content, truncated) = tool_result_content(part.content?);
            if content.is_empty() {
                return None;
            }
            let full_len = content.len() as u64;
            let (content, truncated_preview) = preview_tool_result(content);
            Some(ContentPart::ToolResult {
                summary: if is_error { "[Tool Error]" } else { "[Tool Result]" }.to_string(),
                content,
                is_error,
                tool_use_id: part
                    .tool_use_id
                    .and_then(raw_as_str)
                    .map(|s| s.into_owned()),
                full_len,
                truncated_preview,
                truncated,
                source_offset: 0, // 由解析调用方按行 offset 回填
            })
        }
        _ => None,
    }
}

/// 组装 tool_result 正文：字符串形式直接取；数组形式按顺序拼接 `text` 并各补一个 `\n`。
/// 两条分支最终都走同一套 50 KB 截断（与改造前一致）。
///
/// 返回 `(正文, 是否被截断)` —— 截断提示语由前端按结构化标志渲染
/// （`common.toolResult.truncatedSuffix`），后端不再往正文里拼中文。
///
/// 正文刻意以**裸片段**承载而不是让 `serde_json` 直接给出 `String`：
/// 实测对 1 GB 正文，「扫描但不反转义」明显快于「扫描 + 反转义 + 交出所有权」——
/// 后者便宜不了那 900 MB 的拷贝，反而把反转义的代价提前付掉了。
fn tool_result_content(raw: &RawValue) -> (String, bool) {
    let text = raw.get().trim();
    if text.starts_with('"') {
        return truncate_tool_result(decode_prefix(raw, TOOL_RESULT_MAX_LEN));
    }
    if !text.starts_with('[') {
        return (String::new(), false);
    }

    let mut accumulated = String::new();
    let mut deserializer = serde_json::Deserializer::from_str(text);
    if let Ok(items) = Vec::<&RawValue>::deserialize(&mut deserializer) {
        for item in items {
            // 已越过上限就不必再看后面的项：它们注定会被截掉
            if accumulated.len() > TOOL_RESULT_MAX_LEN {
                break;
            }
            let Ok(sub) = serde_json::from_str::<RawSubText<'_>>(item.get().trim()) else {
                continue;
            };
            let Some(sub_text) = sub.text else {
                continue;
            };
            // 只解码到「刚好够越过上限」为止，避免把将被丢弃的正文整段展开
            let budget = TOOL_RESULT_MAX_LEN
                .saturating_add(1)
                .saturating_sub(accumulated.len())
                .max(1);
            accumulated.push_str(&decode_prefix(sub_text, budget));
            accumulated.push('\n');
        }
    }
    truncate_tool_result(accumulated)
}

/// 与改造前完全一致的截断：按字节取到 `TOOL_RESULT_MAX_LEN`，回退到字符边界。
/// 返回 `(正文, 是否被截断)`；`true` 时正文本身不完整，截断提示由前端渲染。
///
/// `pub(crate)`：legacy 参考实现（`claude.rs` 的 `parse_content`）复用同一函数，
/// 保证对拍测试逐字段一致（含 `truncated` 标志）。
pub(crate) fn truncate_tool_result(mut decoded: String) -> (String, bool) {
    if decoded.len() <= TOOL_RESULT_MAX_LEN {
        return (decoded, false);
    }
    let cut = char_boundary_floor(&decoded, TOOL_RESULT_MAX_LEN);
    decoded.truncate(cut);
    (decoded, true)
}

/// 懒加载预览：正文超过 [`TOOL_RESULT_PREVIEW_LEN`] 且未命中白名单 → 只保留前 2 KB
/// （字符边界对齐），并置 `truncated_preview=true`；小内容与白名单 part 原样返回，
/// 行为与改造前完全一致。
///
/// `pub(crate)`：legacy 参考实现（`claude.rs` 的 `parse_content`）复用同一函数，
/// 保证对拍测试逐字段一致。
pub(crate) fn preview_tool_result(full: String) -> (String, bool) {
    if full.len() <= TOOL_RESULT_PREVIEW_LEN
        || TOOL_RESULT_FULL_MARKERS.iter().any(|m| full.contains(m))
    {
        return (full, false);
    }
    let cut = char_boundary_floor(&full, TOOL_RESULT_PREVIEW_LEN);
    let mut preview = String::with_capacity(cut);
    preview.push_str(&full[..cut]);
    (preview, true)
}

/// 从一行 JSONL 中取出指定 tool_result 的**全文**（50 KB 截断后），
/// 供 `get_tool_result_full_content` 命令按 `source_offset` 回取。
///
/// 反漂移校验：part 类型必须是 `tool_result`，且 `tool_use_id` 与期望值严格一致
/// （期望 `None` 时仅接受实际也缺 `tool_use_id` 的 part）。校验不过返回 `None`，
/// 由命令层转成明确错误，前端回退展示预览。
pub(crate) fn tool_result_full_content_in_line(
    line: &str,
    expected_tool_use_id: Option<&str>,
) -> Option<String> {
    let entry = ClaudeEntry::parse(line)?;
    let message = entry.message()?;
    let MessageContent::Parts(parts) = message.content()? else {
        return None;
    };
    for part in parts {
        if part.part_type.and_then(raw_as_str).as_deref() != Some("tool_result") {
            continue;
        }
        let actual_id = part.tool_use_id.and_then(raw_as_str);
        if actual_id.as_deref() != expected_tool_use_id {
            continue;
        }
        // 截断标志不在此返回：调用方（part）已按同一函数、同一判据算过，
        // 再经这里回传就是第二个真相来源。
        let (content, _truncated) = tool_result_content(part.content?);
        if content.is_empty() {
            return None;
        }
        return Some(content);
    }
    None
}

/// **边解析边截断**：反转义 JSON 字符串裸片段，最多解码到越过 `max_len` 一个字符为止。
///
/// 返回值的保证（调用方据此套用 [`truncate_tool_result`]）：
/// - 它是完整解码结果的前缀，切点落在字符边界上；
/// - 长度不超过 `max_len + 4`；
/// - 当完整解码结果超过 `max_len` 时，返回值长度**必定大于** `max_len`。
///
/// 单遍完成，且普通片段走 `str::find`（memchr）整段拷贝：
/// 旧实现要在同一段字节上"扫一遍判断超长 → 复制成字面量 → 交给 serde_json 反转义"，
/// 实测比原本的 `Value` 路径还慢，正是被这些重复扫描吃掉的。
fn decode_prefix(raw: &RawValue, max_len: usize) -> String {
    let text = raw.get();
    let trimmed = text.trim();
    let Some(inner) = trimmed.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return String::new();
    };
    // 解码后长度不会超过原始长度，容量按二者较小值取即可 —— 绝不按整段正文分配
    let mut out = String::with_capacity(inner.len().min(max_len + 4));
    let mut index = 0usize;

    loop {
        if out.len() > max_len || index >= inner.len() {
            return out;
        }
        let remaining = max_len - out.len();
        // 只在「足够越过上限」的窗口里找下一个转义符：找不到就说明该窗口内全是普通字节，
        // 于是不必扫描整段正文就能断定"一定会截断"。
        // 端点必须上调到字符边界，否则切片会在多字节字符中间 panic。
        let window_raw = index
            .saturating_add(remaining)
            .saturating_add(1)
            .min(inner.len());
        let window_end = char_boundary_ceil(inner, window_raw);

        match inner[index..window_end].find('\\') {
            // 窗口内无转义：窗口内解码长度 == 原始长度
            None => {
                if window_end < inner.len() {
                    let run = &inner[index..window_end];
                    out.push_str(prefix_over_char_boundary(run, remaining));
                    return out;
                }
                // 剩余部分整体落在窗口内且无转义 → 全部属于结果
                out.push_str(&inner[index..]);
                return out;
            }
            Some(offset) => {
                let run_end = index + offset;
                let run = &inner[index..run_end];
                if run.len() > remaining {
                    out.push_str(prefix_over_char_boundary(run, remaining));
                    return out;
                }
                out.push_str(run);
                index = run_end;

                let Some((raw_len, ch)) = decode_escape(inner.as_bytes(), index) else {
                    // 坏转义：交回 serde_json 完整解码，由它给出确定结果（通常判整段非法）
                    return decode_json_string_literal(trimmed).unwrap_or_default();
                };
                out.push(ch);
                index += raw_len;
            }
        }
    }
}

/// 从裸片段取出 JSON 字符串；非字符串返回 `None`（对应 `Value::as_str()`）。
/// 无转义时零拷贝借用，有转义才交给 serde_json 反转义。
fn raw_as_str<'a>(raw: &'a RawValue) -> Option<Cow<'a, str>> {
    let text = raw.get().trim();
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    if !inner.contains('\\') {
        return Some(Cow::Borrowed(inner));
    }
    decode_json_string_literal(text).map(Cow::Owned)
}

/// 对应 `Value::as_bool()`。
fn raw_as_bool(raw: &RawValue) -> Option<bool> {
    match raw.get().trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn decode_json_string_literal(literal: &str) -> Option<String> {
    serde_json::from_str::<String>(literal).ok()
}

/// 解码一个 JSON 转义序列，返回 `(占用的原始字节数, 解码出的字符)`。
/// 遇到非法转义（未定义的转义字母、孤立代理对）返回 `None`，由调用方回退。
fn decode_escape(bytes: &[u8], index: usize) -> Option<(usize, char)> {
    match *bytes.get(index + 1)? {
        b'u' => {
            let high = read_hex4(bytes, index + 2)?;
            if (0xD800..0xDC00).contains(&high) {
                // 高代理：必须紧跟一个低代理转义，否则 serde_json 会把整串判为非法
                if bytes.get(index + 6) == Some(&b'\\') && bytes.get(index + 7) == Some(&b'u') {
                    let low = read_hex4(bytes, index + 8)?;
                    if (0xDC00..0xE000).contains(&low) {
                        let code_point = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                        return Some((12, char::from_u32(code_point)?));
                    }
                }
                return None;
            }
            if (0xDC00..0xE000).contains(&high) {
                return None;
            }
            Some((6, char::from_u32(high)?))
        }
        b'"' => Some((2, '"')),
        b'\\' => Some((2, '\\')),
        b'/' => Some((2, '/')),
        b'b' => Some((2, '\u{8}')),
        b'f' => Some((2, '\u{c}')),
        b'n' => Some((2, '\n')),
        b'r' => Some((2, '\r')),
        b't' => Some((2, '\t')),
        _ => None,
    }
}

fn read_hex4(bytes: &[u8], start: usize) -> Option<u32> {
    let digits = bytes.get(start..start + 4)?;
    let mut value = 0u32;
    for &digit in digits {
        value = (value << 4) | (digit as char).to_digit(16)?;
    }
    Some(value)
}

/// 不超过 `max` 的最大字符边界（与改造前的 `while cut > 0 && !is_char_boundary` 等价）。
fn char_boundary_floor(text: &str, max: usize) -> usize {
    if text.len() <= max {
        return text.len();
    }
    let mut cut = max;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// 不小于 `target` 的最小字符边界。
/// 切 `&str` 之前必须把窗口端点上调到边界，否则会在多字节字符中间 panic。
fn char_boundary_ceil(text: &str, target: usize) -> usize {
    if target >= text.len() {
        return text.len();
    }
    let mut end = target;
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    end
}

/// 严格超过 `max` 的最小字符边界前缀（用于「已知会被截断」时少解码一点）。
fn prefix_over_char_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = char_boundary_ceil(text, max);
    if end == max && end < text.len() {
        end += text[end..].chars().next().map_or(0, |c| c.len_utf8());
    }
    &text[..end.min(text.len())]
}

/// 数组形式 tool_result 里 `content: [{ "text": ... }]` 的单项。
#[derive(Deserialize, Default)]
struct RawSubText<'a> {
    #[serde(default, borrow)]
    text: Option<&'a RawValue>,
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn content_of(line: &str) -> Vec<ContentPart> {
        let entry = ClaudeEntry::parse(line).expect("entry should parse");
        let message = entry.message().expect("message");
        match message.content() {
            Some(content) => content_parts(content),
            None => Vec::new(),
        }
    }

    /// 取 tool_result 的**全文**（50 KB 截断后）。
    /// 走取全文函数而不是 part 的 `content` 字段 —— 懒加载开启后 part 里可能只是 2 KB 预览，
    /// 而本组测试钉的是 50 KB 截断语义。
    fn tool_result_text(line: &str) -> String {
        tool_result_full_content_in_line(line, None).expect("tool_result full content")
    }

    /// 把一段正文编成一行 JSONL 的 tool_result 条目。
    fn line_with_tool_result(content_json: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":{content_json}}}]}}}}"#
        )
    }

    /// 改造前的参考实现：整段反转义后再按字节截断。
    /// 截断提示语已移出正文（见 `truncate_tool_result`），故这里只比对切点与长度。
    fn reference_truncate(full: &str) -> String {
        if full.len() <= TOOL_RESULT_MAX_LEN {
            return full.to_string();
        }
        let cut = char_boundary_floor(full, TOOL_RESULT_MAX_LEN);
        full[..cut].to_string()
    }

    #[test]
    fn parses_plain_message_unchanged() {
        let parts = content_of(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"你好"}]}}"#,
        );
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "你好"));
    }

    #[test]
    fn empty_string_content_still_yields_a_text_part() {
        // 改造前此处没有空串过滤：`content: ""` 会保留一条空文本消息
        let parts = content_of(r#"{"type":"user","message":{"role":"user","content":""}}"#);
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text.is_empty()));
    }

    #[test]
    fn non_string_non_array_content_yields_nothing() {
        for content in ["123", "null", "{}", "[]", "true", r#"{"a":1}"#] {
            let parts = content_of(&format!(
                r#"{{"type":"user","message":{{"role":"user","content":{content}}}}}"#
            ));
            assert!(parts.is_empty(), "content={content} 不应产出任何 part");
        }
    }

    #[test]
    fn raw_value_survives_escapes_and_unicode() {
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"a\"b\\c\nd\t\u4e2d\ud83d\ude00"}]}}"#,
        );
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "a\"b\\c\nd\t中😀"));
    }

    #[test]
    fn tool_use_keeps_summary_and_pretty_input() {
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":"/a/b/plan.md"}}]}}"#,
        );
        match &parts[0] {
            ContentPart::ToolUse {
                summary,
                tool_name,
                input,
                tool_use_id,
            } => {
                assert_eq!(summary, "[Read: plan.md]");
                assert_eq!(tool_name, "Read");
                assert_eq!(tool_use_id.as_deref(), Some("tu_1"));
                assert!(input.contains("\"file_path\""));
            }
            other => panic!("unexpected part: {other:?}"),
        }
    }

    #[test]
    fn tool_use_without_input_falls_back_to_empty_object() {
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#,
        );
        match &parts[0] {
            ContentPart::ToolUse { summary, input, .. } => {
                assert_eq!(summary, "[Bash: ]");
                assert_eq!(input, "{}");
            }
            other => panic!("unexpected part: {other:?}"),
        }
    }

    #[test]
    fn tool_use_with_null_input_keeps_literal_null() {
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":null}]}}"#,
        );
        match &parts[0] {
            ContentPart::ToolUse { input, .. } => assert_eq!(input, "null"),
            other => panic!("unexpected part: {other:?}"),
        }
    }

    #[test]
    fn tool_result_short_string_is_untouched() {
        let line = line_with_tool_result(r#""hello\nworld""#);
        assert_eq!(tool_result_text(&line), "hello\nworld");
    }

    #[test]
    fn tool_result_exactly_at_limit_is_not_truncated() {
        let body = "x".repeat(TOOL_RESULT_MAX_LEN);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        assert_eq!(tool_result_text(&line).len(), TOOL_RESULT_MAX_LEN);
        assert!(!tool_result_truncated(&line));
    }

    #[test]
    fn tool_result_one_byte_over_limit_is_truncated() {
        let body = "x".repeat(TOOL_RESULT_MAX_LEN + 1);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let out = tool_result_text(&line);
        assert_eq!(out, reference_truncate(&body));
        assert!(tool_result_truncated(&line), "超限必须置 truncated 标志");
        assert_eq!(out.len(), TOOL_RESULT_MAX_LEN);
    }

    #[test]
    fn tool_result_truncation_matches_full_decode_on_char_boundaries() {
        // 用 3 字节字符把截断点推到字符中间，验证回退行为与「整段解码再截断」一致
        let prefix = "x".repeat(TOOL_RESULT_MAX_LEN - 2);
        let body = format!("{prefix}中中中中");
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let out = tool_result_text(&line);
        assert_eq!(out, reference_truncate(&body));
        // 第 49,999 字节处正好是「中」的首字节，截断必须回退到它之前
        assert_eq!(out.len(), TOOL_RESULT_MAX_LEN - 2);
        assert!(tool_result_truncated(&line));
    }

    #[test]
    fn tool_result_truncation_matches_full_decode_with_heavy_escapes() {
        // 每 2 个原始字节产出 1 个解码字节，切点必然落在转义序列中间
        let body = "\\n".repeat(TOOL_RESULT_MAX_LEN);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let out = tool_result_text(&line);
        assert_eq!(out, reference_truncate(&body));
    }

    #[test]
    fn tool_result_truncation_handles_astral_chars_at_boundary() {
        // 代理对（解码后 4 字节）跨过上限，必须整体保留再回退
        for pad in 0..8 {
            let prefix = "y".repeat(TOOL_RESULT_MAX_LEN - pad);
            let body = format!("{prefix}😀😀");
            let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
            assert_eq!(
                tool_result_text(&line),
                reference_truncate(&body),
                "pad={pad} 时代理对边界处理应与整段解码一致"
            );
        }
    }

    #[test]
    fn tool_result_truncation_without_any_escape_matches_reference() {
        // 纯普通字节 + 恰好越界一个字符：覆盖「窗口内找不到转义符」这条早退路径
        for extra in 1..6usize {
            let body = "z".repeat(TOOL_RESULT_MAX_LEN + extra);
            let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
            assert_eq!(tool_result_text(&line), reference_truncate(&body), "extra={extra}");
        }
    }

    #[test]
    fn tool_result_multi_byte_boundary_stress_matches_reference() {
        // 用中日韩字符 + 转义混排，覆盖各种切点与上限的相对位置
        for shift in 0..12usize {
            let mut body = String::new();
            while body.len() < TOOL_RESULT_MAX_LEN + shift + 8 {
                body.push('中');
                body.push_str("\\n\"x");
                body.push('😀');
            }
            let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
            assert_eq!(
                tool_result_text(&line),
                reference_truncate(&body),
                "shift={shift} 应一致"
            );
        }
    }

    #[test]
    fn tool_result_array_form_joins_with_newlines() {
        let line = line_with_tool_result(
            r#"[{"type":"text","text":"line1"},{"type":"text","text":"line2"}]"#,
        );
        assert_eq!(tool_result_text(&line), "line1\nline2\n");
    }

    #[test]
    fn tool_result_array_form_ignores_items_without_text() {
        let line = line_with_tool_result(
            r#"[{"type":"text"},{"type":"image"},{"type":"text","text":"kept"},{"text":"bare"}]"#,
        );
        assert_eq!(tool_result_text(&line), "kept\nbare\n");
    }

    #[test]
    fn tool_result_array_form_truncation_matches_reference() {
        let chunk = "z".repeat(20_000);
        let json = format!(
            "[{{\"type\":\"text\",\"text\":{}}},{{\"type\":\"text\",\"text\":{}}},{{\"type\":\"text\",\"text\":{}}}]",
            serde_json::to_string(&chunk).unwrap(),
            serde_json::to_string(&chunk).unwrap(),
            serde_json::to_string(&chunk).unwrap()
        );
        let line = line_with_tool_result(&json);
        let expected_full = format!("{chunk}\n{chunk}\n{chunk}\n");
        assert_eq!(tool_result_text(&line), reference_truncate(&expected_full));
    }

    #[test]
    fn tool_result_array_form_over_limit_while_many_empty_items() {
        // 边界：先勉强不超限，随后一堆空文本项把总长推过上限
        let head = "a".repeat(TOOL_RESULT_MAX_LEN);
        let mut items = vec![format!(
            "{{\"type\":\"text\",\"text\":{}}}",
            serde_json::to_string(&head).unwrap()
        )];
        for _ in 0..5 {
            items.push(r#"{"type":"text","text":""}"#.to_string());
        }
        let json = format!("[{}]", items.join(","));
        let line = line_with_tool_result(&json);
        let expected = format!("{head}\n\n\n\n\n\n");
        assert_eq!(tool_result_text(&line), reference_truncate(&expected));
    }

    #[test]
    fn tool_result_array_form_with_escaped_and_multibyte_text() {
        // 数组项里既有转义又有多字节字符，且整体越界
        let piece = format!("{}中\\n{}", "q".repeat(20_000), "😀".repeat(4_000));
        let json = format!(
            "[{{\"type\":\"text\",\"text\":{}}},{{\"type\":\"text\",\"text\":{}}}]",
            serde_json::to_string(&piece).unwrap(),
            serde_json::to_string(&piece).unwrap()
        );
        let line = line_with_tool_result(&json);
        let expected = format!("{piece}\n{piece}\n");
        assert_eq!(tool_result_text(&line), reference_truncate(&expected));
    }

    #[test]
    fn tool_result_non_string_non_array_yields_no_part() {
        for content in ["null", "123", "{}"] {
            let parts = content_of(&line_with_tool_result(content));
            assert!(parts.is_empty(), "content={content} 不应产出 part");
        }
    }

    #[test]
    fn tool_result_error_flag_drives_summary() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#;
        assert!(matches!(
            &content_of(line)[0],
            ContentPart::ToolResult { summary, is_error, .. } if summary == "[Tool Error]" && *is_error
        ));
    }

    #[test]
    fn empty_tool_result_content_is_dropped() {
        let line = line_with_tool_result(r#""""#);
        assert!(content_of(&line).is_empty());
    }

    // ---------- 懒加载（预览 + 白名单 + 取全文） ----------

    /// 取 part 视角的三元组（content, full_len, truncated_preview）。
    fn tool_result_part(line: &str) -> (String, u64, bool) {
        content_of(line)
            .into_iter()
            .find_map(|part| match part {
                ContentPart::ToolResult {
                    content,
                    full_len,
                    truncated_preview,
                    ..
                } => Some((content, full_len, truncated_preview)),
                _ => None,
            })
            .expect("tool_result part")
    }

    /// 取 part 上的 `truncated` 标志（正文是否在 50 KB 处被截断）。
    /// 截断提示语已不在正文里，故这条标志是「截断发生了」的唯一判据。
    fn tool_result_truncated(line: &str) -> bool {
        content_of(line)
            .into_iter()
            .find_map(|part| match part {
                ContentPart::ToolResult { truncated, .. } => Some(truncated),
                _ => None,
            })
            .expect("tool_result part")
    }

    #[test]
    fn tool_result_over_preview_limit_without_markers_is_previewed() {
        let body = "x".repeat(TOOL_RESULT_PREVIEW_LEN + 500);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let (content, full_len, truncated) = tool_result_part(&line);
        assert!(truncated, "无标记超长正文应只带预览");
        assert_eq!(content.len(), TOOL_RESULT_PREVIEW_LEN);
        assert!(body.starts_with(&content), "预览必须是全文前缀");
        assert_eq!(full_len as usize, body.len(), "full_len 记录全文长度");
        // 取全文函数能拿回完整正文
        assert_eq!(tool_result_full_content_in_line(&line, None).unwrap(), body);
    }

    #[test]
    fn tool_result_at_preview_limit_is_untouched() {
        let body = "y".repeat(TOOL_RESULT_PREVIEW_LEN);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let (content, full_len, truncated) = tool_result_part(&line);
        assert!(!truncated);
        assert_eq!(content, body);
        assert_eq!(full_len as usize, TOOL_RESULT_PREVIEW_LEN);
    }

    #[test]
    fn tool_result_preview_cut_aligns_to_char_boundary() {
        // 「界」3 字节，跨在 2000 字节边界上 → 预览应回退到 1999
        let mut body = "a".repeat(TOOL_RESULT_PREVIEW_LEN - 1);
        body.push('界');
        body.push_str(&"b".repeat(100));
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let (content, _, truncated) = tool_result_part(&line);
        assert!(truncated);
        assert_eq!(content.len(), TOOL_RESULT_PREVIEW_LEN - 1);
        assert!(body.starts_with(&content));
    }

    #[test]
    fn tool_result_whitelist_marker_after_preview_window_keeps_full() {
        // 标记在预览窗口之外也要命中白名单（设计稿 §8「无泄漏」条款）
        for marker in ["<svg", "widget_code", "visualizer_show_widget_result", "[Tool Error]"] {
            let mut body = "z".repeat(TOOL_RESULT_PREVIEW_LEN + 100);
            body.push_str(marker);
            let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
            let (content, full_len, truncated) = tool_result_part(&line);
            assert!(!truncated, "marker={marker} 命中白名单应带全文");
            assert_eq!(content, body, "marker={marker}");
            assert_eq!(full_len as usize, body.len(), "marker={marker}");
        }
    }

    #[test]
    fn tool_result_whitelist_marker_near_50kb_tail_keeps_full() {
        // 探测口径 = 截断后 50 KB 全文：标记在 50 KB 尾部（预览窗口远之后）同样命中
        let mut body = "w".repeat(49_990);
        body.push_str("<svg");
        body.push_str(&"v".repeat(2_000)); // 超出 50 KB 的部分被截掉
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let (content, _, truncated) = tool_result_part(&line);
        assert!(!truncated, "50 KB 尾部的标记也要命中白名单");
        assert!(content.contains("<svg"));
        assert!(tool_result_truncated(&line), "正文仍在 50 KB 处被截断");
    }

    #[test]
    fn tool_result_full_content_in_line_checks_tool_use_id() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"abc"}]}}"#;
        assert_eq!(
            tool_result_full_content_in_line(line, Some("tu_1")).as_deref(),
            Some("abc")
        );
        // 反漂移：tool_use_id 不匹配 / 期望 None 但实际有 id → 一律拒绝
        assert_eq!(tool_result_full_content_in_line(line, Some("tu_other")), None);
        assert_eq!(tool_result_full_content_in_line(line, None), None);
    }

    #[test]
    fn tool_result_full_content_in_line_rejects_wrong_lines() {
        // 反漂移：非 tool_result 行 / 非 JSONL 行 → None（由命令层转成明确错误）
        let text_line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        assert_eq!(tool_result_full_content_in_line(text_line, None), None);
        assert_eq!(tool_result_full_content_in_line("not json at all", None), None);
    }

    #[test]
    fn tool_result_preview_then_full_fetch_roundtrip_on_array_form() {
        // 数组形式正文同样适用：part 带预览，取全文函数还原拼接结果
        let long_text = "arr".repeat(2_000); // 6000 字节，无标记
        let json = format!(
            "[{{\"type\":\"text\",\"text\":{}}}]",
            serde_json::to_string(&long_text).unwrap()
        );
        let line = line_with_tool_result(&json);
        let (content, full_len, truncated) = tool_result_part(&line);
        assert!(truncated);
        assert_eq!(content.len(), TOOL_RESULT_PREVIEW_LEN);
        let expected_full = format!("{long_text}\n");
        assert_eq!(full_len as usize, expected_full.len());
        assert_eq!(tool_result_full_content_in_line(&line, None).unwrap(), expected_full);
    }

    /// 同一条线上两处判据必须一致。
    ///
    /// `tool_result_full_content_in_line` 只回正文、**不回**截断标志，它的注释写明前提是
    /// 「调用方（part）已按同一函数、同一判据算过」——那是一条**无强制约束的耦合**：
    /// 全文路径若哪天换了上限或改成回预览，编译器与标志都不会响，前端会拿到与
    /// `truncated` 标志不符的正文。这里把那个前提钉住：全文路径拿回的正文长度必须与
    /// part 记下的 `full_len` 逐字节相等。
    ///
    /// ⚠️ 别把断言写成 `truncated == (full_len > TOOL_RESULT_MAX_LEN)`：**标志不可由长度反推** ——
    /// 截断后的长度是 `char_boundary_floor(原长, 上限)`，ASCII 下**恰好等于上限**，
    /// 与「原长恰好等于上限、未截断」的 `full_len` 逐字节相同。这正是它必须单独回传的原因。
    #[test]
    fn full_content_fetch_and_part_truncation_flag_agree_on_the_same_limit() {
        // 越过上限：part 说截断，全文路径必须拿回同一段被截的正文
        let body = "x".repeat(TOOL_RESULT_MAX_LEN + 4_096);
        let line = line_with_tool_result(&serde_json::to_string(&body).unwrap());
        let (_, full_len, _) = tool_result_part(&line);
        let full = tool_result_full_content_in_line(&line, None).expect("全文");
        assert_eq!(full.len() as u64, full_len, "两处必须按同一上限截断");
        assert!(tool_result_truncated(&line));
        assert!(
            (TOOL_RESULT_MAX_LEN - 4..=TOOL_RESULT_MAX_LEN).contains(&(full_len as usize)),
            "截断后的长度必须落在 [上限-4, 上限]：越过上限、且切点只退到字符边界: {full_len}"
        );

        // 恰好等于上限：两处长度仍相等，且都不算截断（`>` 而不是 `>=`）
        let line = line_with_tool_result(&serde_json::to_string(&"y".repeat(TOOL_RESULT_MAX_LEN)).unwrap());
        let (_, full_len, _) = tool_result_part(&line);
        let full = tool_result_full_content_in_line(&line, None).expect("全文");
        assert_eq!(full.len() as u64, full_len);
        assert_eq!(full_len as usize, TOOL_RESULT_MAX_LEN);
        assert!(!tool_result_truncated(&line), "恰好等于上限不截断");
    }

    #[test]
    fn image_part_requires_base64_source() {
        let img = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AA=="}}]}}"#;
        assert!(matches!(
            &content_of(img)[0],
            ContentPart::Image { media_type, data } if media_type == "image/jpeg" && data == "AA=="
        ));

        let url_img = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image","source":{"type":"url","url":"http://x"}}]}}"#;
        assert!(content_of(url_img).is_empty());
    }

    #[test]
    fn part_fields_with_wrong_types_are_ignored_not_fatal() {
        // 与改造前 as_str()/as_bool() 的宽容语义一致：类型不符只是取不到
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":42},{"type":"thinking","thinking":true},{"type":"text","text":"kept"}]}}"#,
        );
        assert_eq!(parts.len(), 1, "类型不符的 text/thinking 应被跳过而不是让整行失败");
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "kept"));
    }

    #[test]
    fn tool_use_with_non_string_name_falls_back_to_placeholder() {
        // 改造前 `name.unwrap_or("Tool")`：name 缺失或非字符串都会落到占位名
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":7}]}}"#,
        );
        assert!(matches!(
            &parts[0],
            ContentPart::ToolUse { summary, tool_name, .. } if summary == "[Tool]" && tool_name == "Tool"
        ));
    }

    #[test]
    fn unknown_part_types_are_skipped() {
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"future_thing","x":1},{"type":"text","text":"kept"}]}}"#,
        );
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "kept"));
    }

    #[test]
    fn non_object_array_elements_are_skipped_without_failing_the_rest() {
        // 这是手写 Deserialize 而非 derive 的主要原因：derive 会因非对象元素让整个数组失败
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":["bare",42,null,true,[1,2],{"type":"text","text":"kept"}]}}"#,
        );
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "kept"));
    }

    #[test]
    fn entry_accessors_match_value_semantics() {
        let entry = ClaudeEntry::parse(
            r#"{"type":"user","isSidechain":true,"isMeta":true,"uuid":"u1","timestamp":"T","message":{"role":"user","model":"m","content":"hi"}}"#,
        )
        .expect("parse");
        assert_eq!(entry.entry_type(), "user");
        assert!(entry.is_sidechain());
        assert!(entry.is_meta());
        assert_eq!(entry.uuid().as_deref(), Some("u1"));
        assert_eq!(entry.timestamp(), "T");
        let message = entry.message().expect("message");
        assert_eq!(message.role(), Some("user"));
        assert_eq!(message.model().as_deref(), Some("m"));
    }

    #[test]
    fn usage_key_presence_is_distinguishable_from_null() {
        // 键缺失 → None；键存在但为 null / 非对象 → Some，交由上层产出全 0 的 TokenUsage
        let absent = ClaudeEntry::parse(
            r#"{"type":"assistant","message":{"role":"assistant","content":"x"}}"#,
        )
        .unwrap();
        assert!(absent.message().unwrap().usage().is_none());

        for raw in ["null", "7"] {
            let line = format!(
                r#"{{"type":"assistant","message":{{"role":"assistant","content":"x","usage":{raw}}}}}"#
            );
            let entry = ClaudeEntry::parse(&line).unwrap();
            let usage = entry.message().unwrap().usage();
            assert!(usage.is_some(), "usage={raw} 必须保留「键存在」的事实");
        }
    }

    #[test]
    fn entry_without_message_or_type_is_rejected_like_before() {
        assert!(ClaudeEntry::parse(r#"{"type":"system"}"#).unwrap().message().is_none());
        // message 不是对象：改造前 `entry.get("message")?` 取到非对象后会一路取不到字段并丢弃
        let entry = ClaudeEntry::parse(r#"{"type":"user","message":"oops"}"#);
        assert!(entry.is_none(), "message 非对象应整行失败（与 Value 路径同结论）");
    }

    #[test]
    fn subagent_id_reads_nested_agent_id() {
        let entry = ClaudeEntry::parse(
            r#"{"type":"user","toolUseResult":{"agentId":"abc123"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#,
        )
        .unwrap();
        assert_eq!(entry.subagent_id(), Some("abc123"));
    }

    #[test]
    fn first_tool_use_id_reads_head_element() {
        let entry = ClaudeEntry::parse(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x"},{"type":"tool_result","tool_use_id":"t2","content":"y"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            first_tool_use_id(entry.message().unwrap().content().unwrap()).as_deref(),
            Some("t1")
        );

        // 非数组形态一律取不到（改造前要求 content 是数组）
        let text_only = ClaudeEntry::parse(
            r#"{"type":"user","message":{"role":"user","content":"plain"}}"#,
        )
        .unwrap();
        assert_eq!(first_tool_use_id(text_only.message().unwrap().content().unwrap()), None);
    }

    #[test]
    fn malformed_escape_degrades_to_dropping_only_the_field() {
        // 有意差异（更宽容）：所有字符串字段都以裸片段承接，不被 serde_json 校验转义，
        // 坏转义只让该 part 消失，不会拖垮整行。属"输入本身异常"的范畴，
        // 且降级方式是丢字段而非产出静默错值。
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_result","content":"\ud800"},{"type":"text","text":"survived"}]}}"#,
        );
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "survived"));
    }

    #[test]
    fn duplicate_keys_are_treated_as_malformed_input() {
        // 已知且有意接受的差异：`serde` 的 derive 会对重复键报 `duplicate_field`，
        // 而旧的 `Value` 路径是「后者胜出」。重复键无法由 JSON 序列化器产生
        // （会话文件是机器写入的），且此处的降级方式与「任何其它解析失败」一致 ——
        // 整行当作不可解析丢弃，而不是产出半截数据。样本上的等价性测试会证明实际数据里
        // 不存在这种情况；若将来真的出现，这里会以「丢一条消息」而非「静默错值」暴露。
        assert!(ClaudeEntry::parse(
            r#"{"type":"user","uuid":"a","uuid":"b","message":{"role":"user","content":"x"}}"#
        )
        .is_none());
    }

    #[test]
    fn duplicate_keys_inside_a_content_block_take_last() {
        // 内容块是手写访问器，重复键与旧 `Value` 语义一致（后者胜出）
        let parts = content_of(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"first","text":"second"}]}}"#,
        );
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "second"));
    }

    #[test]
    fn parses_realistic_assistant_line_with_many_parts() {
        let line = r#"{"parentUuid":"p","isSidechain":false,"type":"assistant","uuid":"u","timestamp":"2026-09-11T00:00:00Z","message":{"role":"assistant","model":"claude-opus-4","usage":{"input_tokens":10,"output_tokens":20,"cache_creation_input_tokens":0,"cache_read_input_tokens":5},"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"ok"},{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls -la"}}]}}"#;
        let entry = ClaudeEntry::parse(line).unwrap();
        let message = entry.message().unwrap();
        assert_eq!(message.usage().unwrap()["input_tokens"], json!(10));
        let parts = content_parts(message.content().unwrap());
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], ContentPart::Thinking { thinking } if thinking == "hmm"));
        assert!(matches!(&parts[2], ContentPart::ToolUse { summary, .. } if summary == "[Bash: ls -la]"));
    }

    /// 逐字节对照：`decode_prefix` 的前缀必须与「整段用 serde_json 反转义」完全一致。
    #[test]
    fn decode_prefix_matches_full_unescape_on_random_strings() {
        // 用确定性伪随机构造各种字符/转义混排，覆盖切点落在转义序列与多字节字符中间的情况
        let alphabet = [
            "a", "中", "😀", "\\n", "\\\"", "\\\\", "\\t", "\\u0041", "\\ud83d\\ude00", " ", "~",
        ];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for case in 0..64usize {
            let mut body = String::new();
            // 长度围绕上限附近摆动
            let target = TOOL_RESULT_MAX_LEN - 6 + case * 3;
            while body.len() < target {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                body.push_str(alphabet[(seed as usize) % alphabet.len()]);
            }
            let full = decode_json_string_literal(&serde_json::to_string(&body).unwrap())
                .expect("reference decode");

            let raw = RawValue::from_string(serde_json::to_string(&body).unwrap()).unwrap();
            let prefix = decode_prefix(&raw, TOOL_RESULT_MAX_LEN);

            assert!(full.starts_with(&prefix), "case={case} 前缀必须是完整解码结果的前缀");
            if full.len() > TOOL_RESULT_MAX_LEN {
                assert!(prefix.len() > TOOL_RESULT_MAX_LEN, "case={case} 超长时必须能触发截断");
            }
            assert!(prefix.len() <= TOOL_RESULT_MAX_LEN + 4, "case={case} 前缀长度必须受限");
            assert!(prefix.is_char_boundary(prefix.len()));
            // 截断后的最终结果必须与「整段解码再截断」逐字节一致
            let (out, truncated) = truncate_tool_result(prefix);
            assert_eq!(out, reference_truncate(&full), "case={case}");
            assert_eq!(truncated, full.len() > TOOL_RESULT_MAX_LEN, "case={case}");
        }
    }

    #[test]
    fn decode_prefix_returns_whole_string_when_within_limit() {
        let raw = RawValue::from_string(serde_json::to_string("abcd\nef中😀").unwrap()).unwrap();
        assert_eq!(decode_prefix(&raw, TOOL_RESULT_MAX_LEN), "abcd\nef中😀");
    }

    #[test]
    fn decode_prefix_caps_length_for_escape_free_text() {
        let body = "中".repeat(TOOL_RESULT_MAX_LEN);
        let raw = RawValue::from_string(serde_json::to_string(&body).unwrap()).unwrap();
        let prefix = decode_prefix(&raw, TOOL_RESULT_MAX_LEN);
        assert!(prefix.len() > TOOL_RESULT_MAX_LEN);
        assert!(prefix.len() <= TOOL_RESULT_MAX_LEN + 4);
        assert!(prefix.chars().all(|c| c == '中'));
    }
}
