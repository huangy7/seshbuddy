/**
 * Rust 枚举 ↔ TS 联合的 **parity 提取器**（测试专用）。
 *
 * 为什么需要：这两侧的镜像**只有一侧有编译期保障**。
 * - TS → Rust：TS 联合漏了成员，`switch` 会落空；开了 `strictNullChecks` 时
 *   TS2366（缺少返回语句）会报，编译器管得住。
 * - **Rust → TS：没有任何东西管得住。** Rust 加一个变体，TS 联合本身没变、
 *   `default` 分支不可达，`vue-tsc` 不响；运行时才把字面量原样回显上屏。
 *
 * 所以每个手写的镜像枚举都要配一条 parity 测试，比对**两边的判据**。
 *
 * ⚠️ 折算口径（变体名 → snake_case）是写死在这里的：本模块**不看**
 * `#[serde(rename_all = …)]`，所以给枚举加 `rename_all = "camelCase"` 会改掉线上格式
 * 而 parity 测试仍然绿。它验的是**变体集合**，不是 serde 属性。
 */

/**
 * 取 `pub enum <name> { … }` 的变体名，按 snake_case 折算。
 *
 * 同时支持单元变体（`Single,`）与结构体变体（`Merged { count: usize },`）——
 * 变体名只取行首的那个标识符，跨行的结构体体不受影响。
 */
export function rustEnumVariants(source: string, enumName: string): string[] {
  const body = new RegExp(`pub enum ${enumName} \\{([\\s\\S]*?)\\n\\}`).exec(source)?.[1];
  if (body === undefined) return [];
  return body
    .split("\n")
    .map((line) => line.replace(/\/\/.*$/, "").trim())
    .filter((line) => /^[A-Z][A-Za-z0-9_]*(\s*[{(].*)?,?$/.test(line))
    .map((line) =>
      line
        .replace(/,?$/, "")
        .replace(/\s*[{(].*$/, "")
        .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
        .toLowerCase(),
    );
}

/**
 * 取 `export type <name> = …;` 里的**小写字面量成员**。
 *
 * 对两种形态都适用：字符串联合（`= "a" | "b"`）与带判别式的 tagged union
 * （`= \| { kind: "a" } \| { kind: "b" }`）—— 后者取到的正是 `kind` 的取值。
 *
 * ⚠️ **收尾不能用 `[^;]+`**：tagged union 的对象成员里有分号
 * （`{ kind: "merged"; count: number }`），那样会在第一个分号处截断，
 * 静默漏掉后面的成员。这里按**花括号深度**找类型体真正的收尾分号。
 */
export function tsUnionMembers(source: string, typeName: string): string[] {
  const head = new RegExp(`export type ${typeName} =`).exec(source);
  if (head === null) return [];
  let depth = 0;
  let end = head.index + head[0].length;
  for (; end < source.length; end++) {
    const ch = source[end];
    if (ch === "{") depth++;
    else if (ch === "}") depth--;
    else if (ch === ";" && depth === 0) break;
  }
  const body = source.slice(head.index + head[0].length, end);
  return [...body.matchAll(/"([a-z_]+)"/g)].map((match) => match[1]);
}

/**
 * 取 `pub fn id(self) -> &'static str { … }` 里 `=> "字面量"` 的取值集合 ——
 * **线上 id 串的唯一来源**。
 *
 * ⚠️ **不能按枚举变体名折算**（`rustEnumVariants` 的那套 snake_case 口径）：变体名与
 * 线上 id 不是同一条映射 —— `WorkBuddy` 的 id 是 `workbuddy` 而不是 `work_buddy`。
 * 按折算口径比对会在两个集合都「看起来对」时静默判错，正是本模块要防的那类缺陷。
 */
export function rustCliIds(source: string): string[] {
  const anchor = source.indexOf("pub fn id(self) -> &'static str {");
  if (anchor < 0) return [];
  const open = source.indexOf("{", anchor);
  if (open < 0) return [];
  let depth = 0;
  let end = open;
  for (; end < source.length; end++) {
    if (source[end] === "{") depth++;
    else if (source[end] === "}") {
      depth--;
      if (depth === 0) break;
    }
  }
  const body = source.slice(open, end);
  return [...body.matchAll(/=>\s*"([a-z0-9]+)"/g)].map((match) => match[1]);
}

/**
 * 取 `export const <name> = [ … ] as const;` 里的字符串成员。
 *
 * 收窄派生的清单（`type X = (typeof ARR)[number]`）不是字符串联合，`tsUnionMembers`
 * 抽不到；而它恰恰是前端唯一的手写清单，必须能被比对，否则 parity 只能覆盖一半。
 */
export function tsConstArrayMembers(source: string, constName: string): string[] {
  const body = new RegExp(`export const ${constName} = \\[([\\s\\S]*?)\\] as const;`).exec(source)?.[1];
  if (body === undefined) return [];
  return [...body.matchAll(/"([a-z0-9]+)"/g)].map((match) => match[1]);
}
