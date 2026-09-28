<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import { t } from "../i18n";
import { message } from "@tauri-apps/plugin-dialog";
import { useProxy, compressionNotice, errorNotice } from "../composables/useProxy";
import type { TrafficDetail, SessionTrafficSummary, SessionTrafficFilter } from "../composables/useProxy";
import { useSessions } from "../composables/useSessions";
import { resolveFeatureCliId } from "../composables/cliFilter";
import { isCliId, type CliId } from "../types/cli";
import ApiLogClearConfirm from "./api-log/ApiLogClearConfirm.vue";
import ApiLogFooter from "./api-log/ApiLogFooter.vue";
import ApiLogRequestDetail from "./api-log/ApiLogRequestDetail.vue";
import ApiLogTrafficPanel from "./api-log/ApiLogTrafficPanel.vue";
import ApiLogToolbar from "./api-log/ApiLogToolbar.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { isSseBody, assembleSseMessage } from "../utils/sse";
import { hasSkillInfo, parseSkillInfo } from "../utils/reqSkills";
import { parseRequestMessages } from "../utils/reqMessages";
import { copyToClipboard } from "../utils/clipboard";

const props = withDefaults(defineProps<{ active?: boolean; initialCliId?: CliId; initialSessionId?: string }>(), {
  active: true,
});
const emit = defineEmits<{ "update:cliId": [cliId: CliId] }>();

const { cliOptions } = useSessions();
const FEATURE_CLI_STORAGE_KEY = "seshbuddy-feature-cli-api-log";
const persistedCliId = localStorage.getItem(FEATURE_CLI_STORAGE_KEY);
const featureCliOptions = computed(() =>
  cliOptions.value.filter((cli) => cli.supportsApiLogs),
);
const cliSelectOptions = computed<SelectOption[]>(() =>
  featureCliOptions.value.map((cli) => ({
    value: cli.id,
    label: cli.name,
    cliId: cli.id,
  })),
);
const featureCliId = ref<CliId | undefined>(resolveFeatureCliId(
  featureCliOptions.value,
  props.initialCliId,
  persistedCliId && isCliId(persistedCliId) ? persistedCliId : undefined,
));
const currentCli = computed(() =>
  featureCliOptions.value.find((cli) => cli.id === featureCliId.value)
    ?? featureCliOptions.value[0],
);

const {
  status,
  pending,
  sessions,
  sessionsTotal,
  expandedSessionId,
  expandedSessionTraffic,
  expandedSessionTotal,
  dbSize,
  loading,
  error,
  loadStatus,
  loadSessions,
  loadSessionTraffic,
  getDetail,
  clearTraffic,
  loadDbSize,
  connectWs,
  disconnectWs,
} = useProxy(() => featureCliId.value ?? "claude");

const proxyLoading = ref(false);
const proxyError = ref("");

// Session loading state
const sessionLoading = ref(false);

// Expanded card detail state
const expandedId = ref<string | null>(null);
const expandedDetail = ref<TrafficDetail | null>(null);
const detailLoading = ref(false);
const detailTab = ref<"request" | "response">("request");

// Clear confirmation
const showClearConfirm = ref(false);
const clearDays = ref<number | undefined>(undefined);
const clearLoading = ref(false);
const initialLoading = ref(false);
const loadedCliId = ref<string | null>(null);
let refreshToken = 0;

// 从会话头部「N 条代理记录」跳入时待定位的 session：加载完成后自动展开并滚动到该行
const pendingInitialSessionId = ref<string | undefined>(props.initialSessionId);

function tryApplyInitialSession() {
  const sid = pendingInitialSessionId.value;
  if (!sid) return;
  const target = sessions.value.find((s) => s.session_id === sid);
  if (!target) return; // 列表尚未加载到时保留，下次加载完成再试
  pendingInitialSessionId.value = undefined;
  if (expandedSessionId.value !== sid) {
    void toggleSession(target);
  }
  void nextTick(() => {
    document
      .querySelector(`[data-session-id="${CSS.escape(sid)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  });
}

watch(
  () => props.initialSessionId,
  (sid) => {
    pendingInitialSessionId.value = sid;
    tryApplyInitialSession();
  },
);

const proxyActive = computed(() => !!status.value?.enabled && !!status.value?.running);
const proxyTargetFallback = computed(() => (
  featureCliId.value === "codex" ? "api.openai.com/v1" : "api.anthropic.com"
));
const proxyStatusText = computed(() => (
  proxyActive.value
    ? `127.0.0.1:${status.value?.port} → ${status.value?.target_url || proxyTargetFallback.value}`
    : t("api-log.view.disabledStatus")
));
const logBodyState = computed(() => (initialLoading.value ? "loading" : "content"));

async function waitForPaint() {
  await nextTick();
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

// ─── 会话内流量过滤（全文搜索） ───
// 注意：这些状态必须声明在 resetExpandedState / refreshProxyState 之前 ——
// props.active 的 immediate watch 会在 setup 期间同步调用它们，后置声明会触发 TDZ
const sessionSearch = ref("");

// 详情全屏模式
const detailFullscreen = ref(false);
const expandedSummary = computed(() =>
  expandedSessionTraffic.value.find((t) => t.id === expandedId.value) ?? null,
);

function resetExpandedState() {
  expandedSessionId.value = undefined;
  expandedSessionTraffic.value = [];
  expandedSessionTotal.value = 0;
  expandedId.value = null;
  expandedDetail.value = null;
  detailFullscreen.value = false;
  sessionSearch.value = "";
  detailLoading.value = false;
  sessionLoading.value = false;
}

async function refreshProxyState(options: { defer?: boolean } = {}) {
  const token = ++refreshToken;

  if (options.defer) {
    initialLoading.value = true;
    pending.value = [];
    sessions.value = [];
    resetExpandedState();
    await waitForPaint();
    if (token !== refreshToken) return;
  }

  await loadStatus();
  if (token !== refreshToken) return;

  disconnectWs();
  if (status.value?.running) {
    connectWs(status.value.ws_port);
  } else {
    pending.value = [];
  }

  await Promise.all([loadSessions(), loadDbSize()]);

  if (token === refreshToken) {
    tryApplyInitialSession();
    loadedCliId.value = featureCliId.value ?? null;
    initialLoading.value = false;
  }
}

watch(
  () => props.active,
  (active) => {
    if (active) {
      if (loadedCliId.value === featureCliId.value) {
        void refreshProxyState();
      } else {
        void refreshProxyState({ defer: true });
      }
      return;
    }
    disconnectWs();
  },
  { immediate: true },
);

watch(featureCliId, (cliId) => {
  if (cliId) {
    localStorage.setItem(FEATURE_CLI_STORAGE_KEY, cliId);
    emit("update:cliId", cliId);
  }
  loadedCliId.value = null;
  resetExpandedState();
  if (!props.active) return;
  void refreshProxyState({ defer: true });
});

watch(
  () => props.initialCliId,
  (entryCliId) => {
    featureCliId.value = resolveFeatureCliId(
      featureCliOptions.value,
      entryCliId,
      featureCliId.value,
    );
  },
);

async function startProxy() {
  proxyLoading.value = true;
  proxyError.value = "";
  try {
    if (!featureCliId.value) return;
    await invokeApp("proxy_enable", { cliId: featureCliId.value });
    await refreshProxyState();
  } catch (e: any) {
    proxyError.value = renderAppError(e);
    await message(t("api-log.view.startFailed", { error: renderAppError(e) }), {
      title: t("app.dialog.error"),
      kind: "error",
    });
  } finally {
    proxyLoading.value = false;
  }
}

async function toggleProxy(checked: boolean) {
  proxyError.value = "";
  if (checked) {
    await startProxy();
    return;
  }

  proxyLoading.value = true;
  try {
    if (!featureCliId.value) return;
    await invokeApp("proxy_disable", { cliId: featureCliId.value });
    await refreshProxyState();
  } catch (e: any) {
    proxyError.value = renderAppError(e);
    await refreshProxyState();
    await message(t("api-log.view.stopFailed", { error: renderAppError(e) }), {
      title: t("app.dialog.error"),
      kind: "error",
    });
  } finally {
    proxyLoading.value = false;
  }
}

async function toggleSession(session: SessionTrafficSummary) {
  const sid = session.session_id;
  if (expandedSessionId.value === sid) {
    expandedSessionId.value = undefined;
    expandedSessionTraffic.value = [];
    expandedSessionTotal.value = 0;
    expandedId.value = null;
    expandedDetail.value = null;
    detailFullscreen.value = false;
    return;
  }
  expandedSessionId.value = sid;
  sessionLoading.value = true;
  expandedId.value = null;
  expandedDetail.value = null;
  detailFullscreen.value = false;
  const result = await loadSessionTraffic(sid, 50, 0, currentFilter());
  // 代际守卫：加载期间用户可能已切换/收起会话，迟到的响应不得覆盖新选中项
  if (expandedSessionId.value !== sid) return;
  if (result) {
    expandedSessionTraffic.value = result.items;
    expandedSessionTotal.value = result.total;
    // 立即直接加载首条请求详情，避免 watch 异步延迟产生瀑布流卡顿
    if (result.items.length > 0) {
      void toggleExpand(result.items[0].id);
    }
  }
  sessionLoading.value = false;
}

async function loadMoreSessionTraffic() {
  if (expandedSessionId.value === undefined) return;
  const sid = expandedSessionId.value;
  sessionLoading.value = true;
  const result = await loadSessionTraffic(sid ?? null, 50, expandedSessionTraffic.value.length, currentFilter());
  if (expandedSessionId.value !== sid) return;
  if (result) {
    expandedSessionTraffic.value = [...expandedSessionTraffic.value, ...result.items];
    expandedSessionTotal.value = result.total;
  }
  sessionLoading.value = false;
}

function currentFilter(): SessionTrafficFilter | undefined {
  const search = sessionSearch.value.trim();
  if (!search) return undefined;
  return { search };
}

// 过滤条件变化：重新加载当前展开会话的第一页
async function applySessionFilter() {
  if (expandedSessionId.value === undefined) return;
  const sid = expandedSessionId.value;
  sessionLoading.value = true;
  expandedId.value = null;
  expandedDetail.value = null;
  detailFullscreen.value = false;
  const result = await loadSessionTraffic(sid ?? null, 50, 0, currentFilter());
  if (expandedSessionId.value !== sid) return;
  if (result) {
    expandedSessionTraffic.value = result.items;
    expandedSessionTotal.value = result.total;
  }
  sessionLoading.value = false;
}

let searchDebounce: ReturnType<typeof setTimeout> | null = null;
watch(sessionSearch, () => {
  if (searchDebounce) clearTimeout(searchDebounce);
  searchDebounce = setTimeout(() => void applySessionFilter(), 300);
});

async function toggleExpand(id: string) {
  if (expandedId.value === id && !expandedDetail.value) {
    return;
  }
  expandedId.value = id;
  detailFullscreen.value = false;
  detailTab.value = "request";
  detailLoading.value = true;
  const detail = await getDetail(id);
  // 代际守卫：慢速详情（大报文 SQLite 读取）返回时，用户可能已选中其他请求，
  // 迟到的响应不得覆盖新选中项的详情，否则头部路径与左侧高亮行错配
  if (expandedId.value !== id) return;
  expandedDetail.value = detail;
  if (detail && parseRequestMessages(detail.req_body).hasMessages) {
    detailSection.value = "messages";
  } else {
    detailSection.value = "body";
  }
  detailLoading.value = false;
}

function requestClear(days?: number) {
  if (clearLoading.value) return;
  clearDays.value = days;
  showClearConfirm.value = true;
}

async function confirmClear() {
  if (clearLoading.value) return;

  clearLoading.value = true;
  await waitForPaint();

  try {
    await clearTraffic(clearDays.value);
    resetExpandedState();
    showClearConfirm.value = false;
  } finally {
    clearLoading.value = false;
  }
}

const totalRecords = computed(() => {
  return sessions.value.reduce((sum, s) => sum + s.request_count, 0);
});

// 内存美化缓存，避免同一大报文反复 parse / stringify
const prettyCache = new Map<string, string>();

function prettyJson(raw: string | null): string {
  if (!raw) return "";
  const cached = prettyCache.get(raw);
  if (cached) return cached;
  // 超过 400KB 的超大报文跳过昂贵的美化，直接原样呈现
  if (raw.length > 400_000) {
    return raw;
  }
  try {
    const formatted = JSON.stringify(JSON.parse(raw), null, 2);
    if (prettyCache.size > 20) {
      const firstKey = prettyCache.keys().next().value;
      if (firstKey) prettyCache.delete(firstKey);
    }
    prettyCache.set(raw, formatted);
    return formatted;
  } catch {
    return raw;
  }
}

function langOf(raw: string | null): string {
  if (!raw) return "plaintext";
  if (raw.length > 400_000) return "json"; // 大文件快速认定为 json，免去 JSON.parse 性能损耗
  try {
    JSON.parse(raw);
    return "json";
  } catch {
    return "plaintext";
  }
}

// SSE 重组结果按原始报文做一层 memo：body 文本与语言判定两个 computed
// 在同一渲染周期都要用到组装结果，共享避免对 MB 级流逐行 parse 两遍
let sseAssemblyMemo: { resBody: string; assembled: unknown | null } | null = null;

function assembledSse(res: string): unknown | null {
  if (sseAssemblyMemo?.resBody === res) return sseAssemblyMemo.assembled;
  const assembled = assembleSseMessage(res);
  sseAssemblyMemo = { resBody: res, assembled };
  return assembled;
}

// body 区：惰性计算，仅在激活 body 视图时计算
const detailBodyText = computed(() => {
  if (!expandedDetail.value || detailSection.value !== "body") return "";
  if (detailTab.value === "request") return prettyJson(expandedDetail.value.req_body);
  // 压缩响应没有可展示的正文（后端只回 compression 与 res_size），提示句由前端渲染。
  // 放在正文管线的 SSE/JSON 两个分支之前：提示句不是正文，不该进那条管线
  // （改前它根本不存在——它是本任务新造的前端句子，不是从 res_body 里读出来的）。
  const notice = compressionNotice(expandedDetail.value);
  if (notice) return notice;
  // 上游失败时正文区显示提示句，同样放在 SSE/JSON 管线之前：提示句不是正文。
  const errNotice = errorNotice(expandedDetail.value);
  if (errNotice) return errNotice;
  const res = expandedDetail.value.res_body ?? "";
  if (isSseBody(res)) {
    // 超大 SSE 流（>2MB）跳过重组直接展示原始流，避免逐行 split/parse 卡死渲染
    if (res.length > 2_000_000) return res;
    const assembled = assembledSse(res);
    return assembled ? JSON.stringify(assembled, null, 2) : res;
  }
  return prettyJson(res);
});
const detailBodyLanguage = computed(() => {
  if (!expandedDetail.value || detailSection.value !== "body") return "plaintext";
  if (detailTab.value === "response" && isSseBody(expandedDetail.value.res_body)) {
    return assembledSse(expandedDetail.value.res_body ?? "") ? "json" : "plaintext";
  }
  return langOf(detailTab.value === "request"
    ? expandedDetail.value.req_body
    : expandedDetail.value.res_body);
});
const detailHeadersText = computed(() => {
  if (!expandedDetail.value || detailSection.value !== "headers") return "";
  return prettyJson(detailTab.value === "request"
    ? expandedDetail.value.req_headers
    : expandedDetail.value.res_headers);
});
const detailHeadersLanguage = computed(() => {
  if (!expandedDetail.value || detailSection.value !== "headers") return "plaintext";
  return langOf(detailTab.value === "request"
    ? expandedDetail.value.req_headers
    : expandedDetail.value.res_headers);
});

// 请求是否携带工具定义（决定 Tools 区是否出现）；与 ApiLogToolInspector 口径一致，
// 同时兼容新版 tools 与旧版 functions 两种形态
const detailHasTools = computed(() => {
  try {
    const body = JSON.parse(expandedDetail.value?.req_body ?? "");
    if (Array.isArray(body?.tools) && body.tools.length > 0) return true;
    return Array.isArray(body?.functions) && body.functions.length > 0;
  } catch {
    return false;
  }
});
// 响应是否为 SSE 流（决定「解析」区是否出现）
const detailIsSse = computed(() => isSseBody(expandedDetail.value?.res_body));
// 请求是否含 Skill 信息（可用清单或调用记录，决定 Skills 区是否出现）
const detailHasSkills = computed(() => hasSkillInfo(expandedDetail.value?.req_body));

type DetailSection = "messages" | "body" | "headers" | "tools" | "skills" | "parsed" | "diff";
const detailSection = ref<DetailSection>("body");

// 切换 Request/Response 时选择最合适的默认区
watch(detailTab, (tab) => {
  if (!expandedDetail.value) return;
  if (tab === "response") {
    detailSection.value = detailIsSse.value ? "parsed" : "body";
  } else {
    detailSection.value = parseRequestMessages(expandedDetail.value?.req_body).hasMessages
      ? "messages"
      : "body";
  }
});

// Copy current detail content to clipboard
const copyState = ref<"idle" | "copied">("idle");
const copyToast = ref<string | null>(null);
let toastTimer: ReturnType<typeof setTimeout> | null = null;

function showCopyToast(text: string) {
  copyToast.value = text;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { copyToast.value = null; }, 1600);
}

async function copyDetail() {
  if (!expandedDetail.value) return;
  let text: string;
  let label: string;
  if (detailSection.value === "messages") {
    text = expandedDetail.value.req_body ?? "";
    label = t("api-log.view.copiedMessages");
  } else if (detailSection.value === "tools") {
    // 复制工具定义数组（美化后）
    try {
      const body = JSON.parse(expandedDetail.value.req_body ?? "");
      text = JSON.stringify(body?.tools ?? [], null, 2);
    } catch {
      text = "";
    }
    label = t("api-log.view.copiedTools");
  } else if (detailSection.value === "skills") {
    const info = parseSkillInfo(expandedDetail.value.req_body);
    const lines = [
      ...info.invoked.map((s) => t("api-log.view.invokedSkill", { count: s.count, name: s.name })),
      ...info.available.map((s) => s.name),
    ];
    text = lines.join("\n");
    label = t("api-log.view.copiedSkills");
  } else if (detailSection.value === "parsed") {
    text = expandedDetail.value.res_body ?? "";
    label = t("api-log.view.copiedSse");
  } else if (detailSection.value === "body") {
    // 复制屏上那条提示，而不是空串：与 `detailBodyText` 同序取（压缩 → 上游失败），
    // 否则才是正文。两处不同序就会出现「屏上一句、复制到另一句」。
    text = detailTab.value === "request"
      ? (expandedDetail.value.req_body ?? "")
      : (compressionNotice(expandedDetail.value)
        || errorNotice(expandedDetail.value)
        || (expandedDetail.value.res_body ?? ""));
    label = detailTab.value === "request"
      ? t("api-log.view.copiedRequestBody")
      : t("api-log.view.copiedResponseBody");
  } else if (detailSection.value === "diff") {
    // Diff 视图复制当前请求的完整报文（此前会错误地落到 Headers 分支）
    text = expandedDetail.value.req_body ?? "";
    label = t("api-log.view.copiedRequestPayload");
  } else {
    text = detailTab.value === "request"
      ? (expandedDetail.value.req_headers ?? "")
      : (expandedDetail.value.res_headers ?? "");
    label = detailTab.value === "request"
      ? t("api-log.view.copiedRequestHeaders")
      : t("api-log.view.copiedResponseHeaders");
  }
  if (await copyToClipboard(text)) {
    showCopyToast(label);
    copyState.value = "copied";
    setTimeout(() => { copyState.value = "idle"; }, 1500);
  }
}
</script>

<template>
  <div class="log-embedded">
    <Transition name="toast-fade">
      <div v-if="copyToast" class="copy-toast">
        <SvgIcon name="check" :size="14" />
        <span>{{ copyToast }}</span>
      </div>
    </Transition>
    <ApiLogToolbar
      :active="proxyActive"
      :loading="proxyLoading"
      :status-text="proxyStatusText"
      :total-records="totalRecords"
      :db-size="dbSize"
      :error="proxyError"
      @toggle="toggleProxy"
    >
      <div class="cli-select-inline">
        <ElegantSelect
          v-model="featureCliId"
          :options="cliSelectOptions"
          size="small"
          data-testid="api-log-cli-select"
        />
      </div>
    </ApiLogToolbar>

    <!-- Error -->
    <div v-if="error" class="log-error">{{ error }}</div>

    <!-- Body -->
    <div class="log-body">
      <Transition name="log-body-swap" mode="out-in">
        <div :key="logBodyState" class="log-body-stage">
          <div v-if="logBodyState === 'loading'" class="log-panel-loading">
            <span class="proxy-spinner"></span>
            <span>{{ t("api-log.view.loading", { cli: currentCli?.name ?? "CLI" }) }}</span>
          </div>

          <template v-else>
            <ApiLogTrafficPanel
              :sessions="sessions"
              :sessions-total="sessionsTotal"
              :expanded-session-id="expandedSessionId"
              :expanded-session-traffic="expandedSessionTraffic"
              :expanded-session-total="expandedSessionTotal"
              :expanded-id="expandedId"
              :expanded-detail="expandedDetail"
              :detail-loading="detailLoading"
              :detail-tab="detailTab"
              :detail-section="detailSection"
              :detail-body-text="detailBodyText"
              :detail-body-language="detailBodyLanguage"
              :detail-headers-text="detailHeadersText"
              :detail-headers-language="detailHeadersLanguage"
              :detail-has-tools="detailHasTools"
              :detail-has-skills="detailHasSkills"
              :detail-is-sse="detailIsSse"
              :copy-state="copyState"
              :session-loading="sessionLoading"
              :loading="loading"
              :get-detail-fn="getDetail"
              v-model:session-search="sessionSearch"
              @toggle-session="toggleSession"
              @toggle-expand="toggleExpand"
              @load-more="loadMoreSessionTraffic"
              @update:detail-tab="detailTab = $event"
              @update:detail-section="detailSection = $event"
              @reset-copy-state="copyState = 'idle'"
              @copy-detail="copyDetail"
              @copied="showCopyToast"
              @toggle-fullscreen="detailFullscreen = !detailFullscreen"
            />
          </template>
        </div>
      </Transition>
    </div>

    <ApiLogFooter :loading="clearLoading" @clear="requestClear" />

    <!-- 请求详情全屏模式：覆盖整个视图，Esc/按钮退出 -->
    <Transition name="fade">
      <div v-if="detailFullscreen && expandedDetail" class="detail-fullscreen-overlay">
        <ApiLogRequestDetail
          fullscreen
          :detail="expandedDetail"
          :summary="expandedSummary"
          :traffic-list="expandedSessionTraffic"
          :traffic-has-more="expandedSessionTraffic.length < expandedSessionTotal"
          :get-detail-fn="getDetail"
          :detail-tab="detailTab"
          :detail-section="detailSection"
          :body-text="detailBodyText"
          :body-language="detailBodyLanguage"
          :headers-text="detailHeadersText"
          :headers-language="detailHeadersLanguage"
          :has-tools="detailHasTools"
          :has-skills="detailHasSkills"
          :is-sse="detailIsSse"
          :copy-state="copyState"
          @update:detail-tab="detailTab = $event"
          @update:detail-section="detailSection = $event"
          @copy-detail="copyDetail"
          @reset-copy-state="copyState = 'idle'"
          @toggle-fullscreen="detailFullscreen = false"
          @copied="showCopyToast"
        />
      </div>
    </Transition>

    <!-- Clear confirmation -->
    <Transition name="fade">
      <ApiLogClearConfirm
        v-if="showClearConfirm"
        :days="clearDays"
        :loading="clearLoading"
        @cancel="showClearConfirm = false"
        @confirm="confirmClear"
      />
    </Transition>
  </div>
</template>

<style scoped>
.log-embedded {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  position: relative;
}
.detail-fullscreen-overlay {
  position: absolute;
  inset: 0;
  z-index: 10;
  background: var(--color-bg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.copy-toast {
  position: absolute;
  top: 14px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  background-color: color-mix(in srgb, var(--color-success) 15%, var(--color-bg));
  color: var(--color-success);
  border: 1px solid color-mix(in srgb, var(--color-success) 30%, var(--color-border));
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-weight: 500;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  pointer-events: none;
  white-space: nowrap;
}
.copy-toast svg {
  color: var(--color-success);
}
[data-theme="dark"] .copy-toast {
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
}
.toast-fade-enter-active,
.toast-fade-leave-active {
  transition: all var(--transition-slow);
}
.toast-fade-enter-from,
.toast-fade-leave-to {
  opacity: 0;
  transform: translate(-50%, -10px);
}
.cli-select-inline {
  min-width: 120px;
  flex-shrink: 0;
}

.proxy-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: proxy-spin 0.8s linear infinite;
}
@keyframes proxy-spin {
  to { transform: rotate(360deg); }
}

.log-error {
  padding: var(--space-2) var(--space-5);
  font-size: var(--text-xs);
  color: var(--color-danger);
  background: rgba(220, 38, 38, 0.06);
}

/* Body */
.log-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: var(--space-2) var(--space-4) var(--space-3);
}
.log-body-stage {
  flex: 1;
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.log-body-swap-enter-active,
.log-body-swap-leave-active {
  transition: opacity 180ms ease, transform 220ms ease;
}
.log-body-swap-enter-from,
.log-body-swap-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.log-panel-loading {
  min-height: 240px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
}
</style>
