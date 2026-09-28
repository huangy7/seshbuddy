/**
 * 内置「官方配置」的身份值与判据。
 *
 * `OFFICIAL_PROFILE_NAME` 是**用户数据里真实存在的配置名**——后端用同一个名字建出这条内置
 * profile，前端拿它当守卫条件与去重键。显示它就是显示数据，故它不进语言包、不翻译：
 * 官方徽章的文案另有键 `api-profile.list.officialBadge`。
 *
 * **这个字面量必须只有一个来源。** 它曾经在 `ApiProfileManager.vue` 与 `ApiProfileList.vue`
 * 里各硬编码了一份，两份独立漂移时官方配置的守卫（不可删、不可改名、不可复制、不可编辑）
 * 会只在一侧生效——另一侧照常放行，而两侧各自看都是对的。`officialProfile.test.ts` 的
 * 单一来源断言把这条钉住：在别处再抄一份会让它变红。
 */
export const OFFICIAL_PROFILE_NAME = "Codex Official";

/**
 * 这个配置名是不是内置的官方配置。
 *
 * 入参收 `null`：编辑器的「当前正在编辑哪条」在没打开编辑器时就是 `null`，
 * 而「没在编辑官方配置」同样是假——判据本身对 `null` 已有确定的答案。
 */
export function isOfficialProfile(name: string | null): boolean {
  return name === OFFICIAL_PROFILE_NAME;
}
