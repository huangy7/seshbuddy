/**
 * 从 Claude Code 请求体中提取 skill 信息：
 * - available：system-reminder 里「available for use with the Skill tool」的可用 skills 清单
 * - invoked：messages 历史中 Skill 工具的实际调用记录（名称 + 次数）
 */

export interface SkillEntry {
  name: string;
  description: string;
}

export interface InvokedSkill {
  name: string;
  count: number;
}

export interface SkillInfo {
  available: SkillEntry[];
  invoked: InvokedSkill[];
}

/** 可用清单的锚点文本（Claude Code 系统提示固定句式） */
const AVAILABLE_MARKER = "skills are available for use with the Skill tool";

/**
 * req_body 是 JSON 字符串，清单文本在 JSON 字符串值内部，
 * 换行是字面量的 `\` + `n` 两个字符，所以对原始文本做转义感知匹配。
 */
function parseAvailable(raw: string): SkillEntry[] {
  const markerIdx = raw.indexOf(AVAILABLE_MARKER);
  if (markerIdx < 0) return [];
  const BS_N = "\\n";
  // marker 后是 `:\n\n`，清单从第一个空行后开始，到下一个空行结束
  const rest = raw.slice(markerIdx + AVAILABLE_MARKER.length);
  const firstBlank = rest.indexOf(BS_N + BS_N);
  if (firstBlank < 0) return [];
  const after = rest.slice(firstBlank + BS_N.length * 2);
  const secondBlank = after.indexOf(BS_N + BS_N);
  const section = secondBlank >= 0 ? after.slice(0, secondBlank) : after.slice(0, 20000);
  const entries: SkillEntry[] = [];
  for (const line of section.split(BS_N)) {
    const m = line.match(/^-\s+([\w][\w:.-]*):\s+(.*)$/);
    if (m) {
      entries.push({ name: m[1], description: m[2].trim() });
    }
  }
  return entries;
}

interface JsonObject {
  [key: string]: unknown;
}

function parseInvoked(raw: string): InvokedSkill[] {
  let body: JsonObject;
  try {
    body = JSON.parse(raw);
  } catch {
    return [];
  }
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const counts = new Map<string, number>();
  for (const msg of messages) {
    const content = (msg as JsonObject)?.content;
    if (!Array.isArray(content)) continue;
    for (const block of content) {
      const b = block as JsonObject;
      if (b?.type !== "tool_use") continue;
      const name = b.name;
      if (name !== "Skill") continue;
      const input = b.input as JsonObject | undefined;
      const skill = typeof input?.skill === "string" ? input.skill : null;
      if (!skill) continue;
      counts.set(skill, (counts.get(skill) ?? 0) + 1);
    }
  }
  return [...counts.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count);
}

export function parseSkillInfo(reqBody: string | null | undefined): SkillInfo {
  if (!reqBody) return { available: [], invoked: [] };
  return {
    available: parseAvailable(reqBody),
    invoked: parseInvoked(reqBody),
  };
}

/** 快速存在性判断（避免无谓解析） */
export function hasSkillInfo(reqBody: string | null | undefined): boolean {
  if (!reqBody) return false;
  return reqBody.includes(AVAILABLE_MARKER) || reqBody.includes('"Skill"');
}
