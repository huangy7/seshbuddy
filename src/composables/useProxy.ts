import { ref, onUnmounted, computed, toValue, type MaybeRefOrGetter } from "vue";
import { t } from "../i18n";
import { appErrorCode, invokeApp, renderAppError } from "../utils/invokeApp";
import { useStreamingCollection } from "./useStreamingCollection";

// ─── Types ───────────────────────────────────────

export interface ProxyStatusInfo {
  enabled: boolean;
  running: boolean;
  port: number;
  ws_port: number;
  target_url: string;
  cli_id: string;
}

export interface TrafficSummary {
  id: string;
  timestamp: string;
  method: string;
  path: string;
  req_size: number;
  status: number | null;
  res_size: number;
  duration_ms: number;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_tokens: number | null;
  cache_creation_tokens: number | null;
  tool_names: string[];
  has_skill_call: boolean;
}

/** 会话流量过滤：search 走 req/res body LIKE，presets 为结构化预设，toolNames 按工具定义过滤 */
export interface SessionTrafficFilter {
  search?: string;
  presets?: string[];
  toolNames?: string[];
}

export interface TrafficDetail {
  id: string;
  timestamp: string;
  method: string;
  path: string;
  req_headers: string | null;
  req_body: string | null;
  req_size: number;
  status: number | null;
  res_headers: string | null;
  res_body: string | null;
  /** 上游强制压缩时的 `content-encoding`；此时 `res_body` 为空，提示句由前端渲染 */
  compression: string | null;
  /**
   * 后端只回事实（今天只有 `"upstream"`）；句子由前端渲染，`res_body` 是第三方原文、原样展示。
   * 事实由 `src-tauri/src/proxy.rs` 的 `get_detail` / `find_traffic_by_timestamp` 从库里原样
   * 转发到 IPC——不经应用侧 Rust 那一程，字段到不了这里。
   */
  error_kind: string | null;
  res_size: number;
  duration_ms: number;
}

/** 会话在流量面板里的显示名：后端只回结构化数据，标签由前端按当前语言渲染。 */
export type TrafficSessionLabel =
  /** 会话列表索引解析出的展示名（用户重命名 / 原生标题 / 首条消息）——用户数据，原样显示 */
  | { kind: "named"; title: string }
  /** 解析不出标题的会话：渲染本地化标签 + 截断后的 id */
  | { kind: "session"; session_id: string }
  /** 没有归属会话的请求桶 */
  | { kind: "ungrouped" };

/**
 * 会话 id 在界面上截断到多少位。
 *
 * 只此一处决定长度：标签与「已复制」提示都在这里取，免得两处各截各的、
 * 同一个会话在不同位置显示成不同的前缀。
 */
const SHORT_SESSION_ID_LENGTH = 8;

export function shortSessionId(sessionId: string): string {
  return sessionId.slice(0, SHORT_SESSION_ID_LENGTH);
}

/**
 * 按变体渲染会话标签。
 *
 * `t()` 必须写在函数体里求值：写成模块级常量会把语言冻结在 import 那一刻，
 * 切换语言后不再跟随。
 */
export function sessionLabel(label: TrafficSessionLabel): string {
  switch (label.kind) {
    case "named":
      return label.title;
    case "session":
      return t("api-log.trafficPanel.sessionLabel", { id: shortSessionId(label.session_id) });
    case "ungrouped":
      return t("api-log.trafficPanel.ungroupedLabel");
  }
}

/**
 * 压缩响应未解压存储时正文区显示的提示；不是压缩响应时返回空串（调用方照常渲染正文）。
 *
 * 后端只回 `compression`（编码名）与 `res_size`（收到的字节数），句子由前端渲染——
 * 正文是被原样展示的，写中文进去就把语言定死在了库里。
 */
export function compressionNotice(
  detail: Pick<TrafficDetail, "compression" | "res_size">,
): string {
  if (!detail.compression) return "";
  return t("api-log.view.compressionNotice", {
    encoding: detail.compression,
    size: detail.res_size,
  });
}

/**
 * 上游失败时正文区显示的提示；不是这类失败时返回空串（调用方照常渲染正文）。
 *
 * ⚠️ **未知 kind 的兜底是【原样展示 `res_body`】，这是良性的**：`res_body` 装的是
 * **第三方错误的原文**，原样展示本就是它的正确去处（R3：第三方文本永不翻译）。
 * 这与「`default` 返回原值会把内部标识符粘上屏」不是一回事 —— 别把这条兜底「修掉」。
 */
export function errorNotice(
  detail: Pick<TrafficDetail, "error_kind" | "res_body">,
): string {
  if (detail.error_kind !== "upstream") return "";
  return t("api-log.view.upstreamError", { detail: detail.res_body ?? "" });
}

export interface SessionTrafficSummary {
  session_id: string | null;
  display_name: TrafficSessionLabel;
  /** `null` = 这个会话没有项目路径；流量面板据此隐藏该行（不渲染占位文案） */
  project_path: string | null;
  request_count: number;
  total_req_size: number;
  total_res_size: number;
  total_duration_ms: number;
  first_timestamp: string;
  last_timestamp: string;
  ok_count: number;
  error_count: number;
}

interface TrafficList {
  items: TrafficSummary[];
  total: number;
}

interface ClearResult {
  deleted: number;
  db_size: number;
}

const PROXY_STATUS_CHANGED_EVENT = "seshbuddy:proxy-status-changed";

export interface ProxyStatusChangedDetail {
  cliId: string;
  status: ProxyStatusInfo | null;
}

export function emitProxyStatusChanged(detail: ProxyStatusChangedDetail) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent<ProxyStatusChangedDetail>(PROXY_STATUS_CHANGED_EVENT, {
    detail,
  }));
}

export function addProxyStatusChangedListener(handler: (detail: ProxyStatusChangedDetail) => void) {
  if (typeof window === "undefined") {
    return () => {};
  }

  const listener: EventListener = (event) => {
    handler((event as CustomEvent<ProxyStatusChangedDetail>).detail);
  };

  window.addEventListener(PROXY_STATUS_CHANGED_EVENT, listener);
  return () => window.removeEventListener(PROXY_STATUS_CHANGED_EVENT, listener);
}

// WS event types from proxy
interface WsPending {
  type: "pending";
  id: string;
  timestamp: string;
  method: string;
  path: string;
  session_id: string | null;
}

interface WsComplete {
  type: "complete";
  id: string;
  status: number;
  duration_ms: number;
  req_size: number;
  res_size: number;
  session_id: string | null;
}

type WsEvent = WsPending | WsComplete;

// Pending request shown in UI
export interface PendingRequest {
  id: string;
  timestamp: string;
  method: string;
  path: string;
  session_id: string | null;
}

// ─── Composable ──────────────────────────────────

export function useProxy(resolveCliId: MaybeRefOrGetter<string>) {
  const status = ref<ProxyStatusInfo | null>(null);
  const pending = ref<PendingRequest[]>([]);

  // ─── Streaming wrappers ────
  // 在 composable 内部创建，依赖组件 setup 同步阶段调用（与现有 useProxy 调用约束一致）
  const sessionsStream = useStreamingCollection<SessionTrafficSummary, { total: number }>({
    command: "proxy_get_sessions",
    topic: "proxy_sessions",
  });

  const sessions = sessionsStream.items;
  const sessionsTotal = computed(() => sessions.value.length);
  const expandedSessionId = ref<string | null | undefined>(undefined);
  const expandedSessionTraffic = ref<TrafficSummary[]>([]);
  const expandedSessionTotal = ref(0);
  const dbSize = ref(0);
  const loading = ref(false);
  const error = ref("");

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectAttempts = 0;
  let shouldReconnect = false;

  const MAX_RECONNECT_ATTEMPTS = 10;
  const RECONNECT_BASE_DELAY = 1000; // 1s
  const RECONNECT_MAX_DELAY = 16000; // 16s

  function resolvedCliId() {
    return toValue(resolveCliId);
  }

  function cliPayload() {
    return { cliId: resolvedCliId() };
  }

  function notifyProxyStatus(nextStatus: ProxyStatusInfo | null) {
    emitProxyStatusChanged({
      cliId: nextStatus?.cli_id || resolvedCliId(),
      status: nextStatus,
    });
  }

  // ─── Status ─────────────────────

  async function loadStatus() {
    try {
      const s = await invokeApp<ProxyStatusInfo>("proxy_status", cliPayload());
      status.value = s.enabled ? s : null;
      notifyProxyStatus(status.value);
    } catch (_) {
      status.value = null;
      notifyProxyStatus(null);
    }
  }

  // ─── Sessions (grouped, streaming) ─────

  async function loadSessions() {
    loading.value = true;
    try {
      await sessionsStream.refresh({ ...cliPayload() });
    } catch (e: unknown) {
      // 命中「流量数据库还没生成」的错误码时不弹错误：那是代理还没跑过的正常状态。
      // 判据是后端 `AppError::Coded` 的 code（跨进程协议状态），不是错误文案——
      // 按文案子串匹配时，译文一变这条分支就静默失效。见 useProxy.test.ts 的绊线。
      if (appErrorCode(e) !== "proxy.traffic_db_missing") error.value = renderAppError(e);
    } finally {
      loading.value = false;
    }
  }

  // 记录各会话最近一次流量查询的搜索词，WS 实时追加时据此遵守当前过滤视图
  const sessionSearchBySid = new Map<string, string>();

  async function loadSessionTraffic(sessionId: string | null, limit = 50, offset = 0, filter?: SessionTrafficFilter): Promise<TrafficList | null> {
    sessionSearchBySid.set(sessionId ?? "", filter?.search?.trim() ?? "");
    try {
      return await invokeApp<TrafficList>("proxy_get_session_traffic", {
        sessionId,
        limit,
        offset,
        search: filter?.search ?? null,
        presets: filter?.presets?.length ? filter.presets : null,
        toolNames: filter?.toolNames?.length ? filter.toolNames : null,
        ...cliPayload(),
      });
    } catch (e: any) {
      error.value = renderAppError(e);
      return null;
    }
  }

  // ─── Detail ─────────────────────
  const MAX_DETAIL_CACHE_ENTRIES = 100;
  // 字节级上限：条数限制约束不了 MB 级报文，浏览长会话会把数百 MB 详情常驻内存。
  // 超出总字节上限时按插入顺序淘汰最旧条目；单条超限的不缓存，按需重取。
  const MAX_DETAIL_CACHE_BYTES = 48 * 1024 * 1024;
  const MAX_DETAIL_ENTRY_BYTES = 8 * 1024 * 1024;
  const detailCache = new Map<string, { detail: TrafficDetail; bytes: number }>();
  let detailCacheBytes = 0;

  function detailEntryBytes(detail: TrafficDetail): number {
    return (detail.req_body?.length ?? 0) + (detail.res_body?.length ?? 0) + 2048;
  }

  function evictDetailCache(incomingBytes: number) {
    while (
      detailCache.size > 0 &&
      (detailCache.size >= MAX_DETAIL_CACHE_ENTRIES ||
        detailCacheBytes + incomingBytes > MAX_DETAIL_CACHE_BYTES)
    ) {
      const first = detailCache.entries().next().value;
      if (!first) break;
      detailCacheBytes -= first[1].bytes;
      detailCache.delete(first[0]);
    }
  }

  async function getDetail(id: string): Promise<TrafficDetail | null> {
    const cached = detailCache.get(id);
    if (cached) return cached.detail;
    try {
      const detail = await invokeApp<TrafficDetail>("proxy_get_detail", { id });
      if (detail) {
        const bytes = detailEntryBytes(detail);
        if (bytes <= MAX_DETAIL_ENTRY_BYTES) {
          evictDetailCache(bytes);
          detailCache.set(id, { detail, bytes });
          detailCacheBytes += bytes;
        }
      }
      return detail;
    } catch (e: any) {
      error.value = renderAppError(e);
      return null;
    }
  }

  // ─── Clear ──────────────────────

  async function clearTraffic(beforeDays?: number) {
    detailCache.clear();
    detailCacheBytes = 0;
    sessionSearchBySid.clear();
    try {
      const result = await invokeApp<ClearResult>("proxy_clear_traffic", {
        beforeDays,
        ...cliPayload(),
      });
      dbSize.value = result.db_size;
      pending.value = [];
      expandedSessionId.value = undefined;
      expandedSessionTraffic.value = [];
      expandedSessionTotal.value = 0;
      // 清理后刷新会话流，让 useStreamingCollection 的 "done with !receivedAny → 清空" 分支接管
      await loadSessions();
    } catch (e: any) {
      error.value = renderAppError(e);
    }
  }

  // ─── DB Size ────────────────────

  async function loadDbSize() {
    try {
      dbSize.value = await invokeApp<number>("proxy_db_size");
    } catch (_) {
      // ignore
    }
  }

  // ─── WebSocket with reconnect ──────────────────

  function connectWs(wsPort: number) {
    disconnectWs();
    shouldReconnect = true;
    reconnectAttempts = 0;
    doConnect(wsPort);
  }

  function doConnect(wsPort: number) {
    const url = `ws://127.0.0.1:${wsPort}`;
    const socket = new WebSocket(url);
    ws = socket;

    socket.onopen = () => {
      reconnectAttempts = 0;
    };

    socket.onmessage = (event) => {
      try {
        const data: WsEvent = JSON.parse(event.data);
        if (data.type === "pending") {
          pending.value = [
            {
              id: data.id,
              timestamp: data.timestamp,
              method: data.method,
              path: data.path,
              session_id: data.session_id,
            },
            ...pending.value,
          ];
        } else if (data.type === "complete") {
          const completedPending = pending.value.find((p) => p.id === data.id);
          pending.value = pending.value.filter((p) => p.id !== data.id);

          // Update session summary if in session view
          const sid = data.session_id ?? completedPending?.session_id;
          updateSessionOnComplete(sid, data, completedPending);
        }
      } catch (_) {
        // ignore malformed messages
      }
    };

    socket.onclose = () => {
      // disconnect→connect 快速切换时，旧 socket 的 onclose 会异步触发：
      // 忽略非当前连接的事件，避免清掉新连接引用或为旧端口安排重连
      if (ws !== socket) return;
      ws = null;
      scheduleReconnect(wsPort);
    };

    socket.onerror = () => {
      // Will trigger onclose
    };
  }

  function updateSessionOnComplete(sessionId: string | null | undefined, data: WsComplete, completedPending?: PendingRequest) {
    const sid = sessionId ?? null;
    const idx = sessions.value.findIndex((s) => s.session_id === sid);
    if (idx >= 0) {
      const old = sessions.value[idx];
      const updated: SessionTrafficSummary = {
        ...old,
        request_count: old.request_count + 1,
        total_req_size: old.total_req_size + data.req_size,
        total_res_size: old.total_res_size + data.res_size,
        total_duration_ms: old.total_duration_ms + data.duration_ms,
        last_timestamp: new Date().toISOString(),
        ok_count: old.ok_count + (data.status >= 200 && data.status < 300 ? 1 : 0),
        error_count: old.error_count + (data.status >= 400 ? 1 : 0),
      };
      // Move updated session to top if it has a real session_id; keep null group at bottom
      if (sid !== null) {
        sessions.value = [
          updated,
          ...sessions.value.slice(0, idx),
          ...sessions.value.slice(idx + 1),
        ];
      } else {
        sessions.value = [
          ...sessions.value.slice(0, idx),
          updated,
          ...sessions.value.slice(idx + 1),
        ];
      }
    }
    // If session not in list yet, reload after a short delay
    if (idx < 0) {
      setTimeout(async () => {
        await loadSessions();
        // Auto-expand this session if none is currently expanded and it has a real session_id
        if (expandedSessionId.value === undefined && sid !== null) {
          expandedSessionId.value = sid;
          const result = await loadSessionTraffic(sid);
          if (result) {
            expandedSessionTraffic.value = result.items;
            expandedSessionTotal.value = result.total;
          }
        }
      }, 300);
    }

    // Auto-append to expanded session traffic list
    if (expandedSessionId.value !== undefined && expandedSessionId.value === sid) {
      // 搜索过滤生效时跳过实时追加，避免不匹配关键词的记录混入过滤视图
      // （token/工具字段由后端列表查询回填，手动刷新后完整展示）
      const activeSearch = sessionSearchBySid.get(sid ?? "") ?? "";
      if (!activeSearch) {
        const newItem: TrafficSummary = {
          id: data.id,
          timestamp: completedPending?.timestamp ?? new Date().toISOString(),
          method: completedPending?.method ?? "POST",
          path: completedPending?.path ?? "",
          req_size: data.req_size,
          status: data.status,
          res_size: data.res_size,
          input_tokens: null,
          output_tokens: null,
          cache_read_tokens: null,
          cache_creation_tokens: null,
          tool_names: [],
          has_skill_call: false,
          duration_ms: data.duration_ms,
        };
        expandedSessionTraffic.value = [newItem, ...expandedSessionTraffic.value];
        expandedSessionTotal.value++;
      }
    }
  }

  function scheduleReconnect(wsPort: number) {
    if (!shouldReconnect || reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
      return;
    }
    const delay = Math.min(
      RECONNECT_BASE_DELAY * Math.pow(2, reconnectAttempts),
      RECONNECT_MAX_DELAY,
    );
    reconnectAttempts++;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      if (shouldReconnect) {
        doConnect(wsPort);
      }
    }, delay);
  }

  function disconnectWs() {
    shouldReconnect = false;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (ws) {
      ws.close();
      ws = null;
    }
  }

  // ─── Lifecycle ──────────────────

  onUnmounted(() => {
    disconnectWs();
  });

  return {
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
  };
}
