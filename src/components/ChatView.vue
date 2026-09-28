<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted, shallowRef } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import type { ChatMessage, ContentPart, BookmarkInfo, SessionLoadResult } from "../types/session";
import { resolveCliDefinition, type CliId } from "../types/cli";
import { formatTimestamp } from "../utils/format";
import { handleMarkdownLinkClick } from "../utils/markdownLinks";
import { highlightJson, highlightSseResponse } from "../utils/json";
import { isMessageVisible } from "../utils/messageFilter";
import LoadingSkeleton from "./LoadingSkeleton.vue";
import ScrollToBottom from "./ScrollToBottom.vue";
import ChatSearchBar from "./ChatSearchBar.vue";
import ChatApiDetailPanel from "./chat/ChatApiDetailPanel.vue";
import ChatImageFullscreenModal from "./chat/ChatImageFullscreenModal.vue";
import ChatMessageBody from "./chat/ChatMessageBody.vue";
import ChatMessageHeader from "./chat/ChatMessageHeader.vue";
import ChatSelectionPill from "./chat/ChatSelectionPill.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { addProxyStatusChangedListener, compressionNotice, errorNotice } from "../composables/useProxy";
import { useChatScroll, type IndexedChatMessage } from "../composables/useChatScroll";
import { useChatSearch } from "../composables/useChatSearch";
import { useSessions } from "../composables/useSessions";
import { useStreamingLoad } from "../composables/useStreamingLoad";
import { useMessageTurns } from "../composables/useMessageTurns";
import ChatToolGroup from "./chat/ChatToolGroup.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import ChatTimeDivider from "./chat/ChatTimeDivider.vue";
import ChatSvgWidgetCard from "./chat/ChatSvgWidgetCard.vue";
import type { TurnNode, TextSegment, WidgetSegment } from "../types/chatTurn";
import { setToolResultSessionPath } from "../composables/useToolResultFullContent";
import { t } from "../i18n";
import "../styles/markdown.css";

const props = defineProps<{
  sessionPath: string;
  encodedDir: string;
  active: boolean;
  showSearch: boolean;
  bookmarks: BookmarkInfo[];
  sessionId: string;
  projectRoot?: string;
  autoFollow?: boolean;
  filterUser: boolean;
  filterAssistant: boolean;
  filterTool: boolean;
  filterThinking: boolean;
  searchHostId?: string;
  isSubagent?: boolean;
  cliId: CliId;
}>();

const emit = defineEmits<{
  closeSearch: [];
  toggleBookmark: [messageIndex: number];
  forkFromHere: [uuid: string];
  activeMessageChange: [messageIndex: number | null];
  openFile: [path: string, projectRoot: string];
  openSubagent: [path: string, label: string];
  proxyTrafficCountChange: [count: number];
}>();

const effectiveCliId = computed(() => props.cliId);

// tool_result 懒加载：全文缓存按会话文件隔离（source_offset 只对当前文件有意义）
watch(() => props.sessionPath, (path) => setToolResultSessionPath(path), { immediate: true });

// Fork 能力：仅当前 Tab 归属 CLI 支持任意消息分叉时展示入口
const canFork = computed(() => resolveCliDefinition(effectiveCliId.value).supportsInPlaceFork);

// 流式加载状态
const sessionStream = useStreamingLoad<ChatMessage, {
  offset: number;
  subagent_map: Record<string, import("../types/session").SubagentInfo>;
}>();
const messages = sessionStream.items;
const loading = sessionStream.loading;
// 全量到齐之前需要门控的操作（滚动到底/搜索/书签跳转）以此为闸
const fullyLoaded = computed(() => sessionStream.done.value !== null);
const subagentMap = shallowRef<Record<string, import("../types/session").SubagentInfo>>({});
let currentOffset = 0;
let identityGeneration = 0;
const unreadCount = ref(0);

const sessionIdentityKey = computed(() => `${effectiveCliId.value}\u0000${props.sessionPath}`);

interface SessionLoadTarget {
  key: string;
  filePath: string;
  cliId: CliId;
  isSubagent: boolean;
}

function currentSessionLoadTarget(): SessionLoadTarget {
  return {
    key: sessionIdentityKey.value,
    filePath: props.sessionPath,
    cliId: effectiveCliId.value,
    isSubagent: props.isSubagent === true,
  };
}

const searchTeleportTarget = computed(() => `#${props.searchHostId || "session-toolbar-search"}`);
const isRefreshingSession = ref(false);
const showInitialLoading = computed(() => loading.value && messages.value.length === 0);
// 首 chunk 到达后、done 之前：消息已经在渲染但还在持续流入，给用户一个明确的进度提示（刷新已有会话时保持静默）
const streamingProgress = computed(
  () => !isRefreshingSession.value && loading.value && messages.value.length > 0 && !fullyLoaded.value
);

async function loadSession(
  target = currentSessionLoadTarget(),
  generation = identityGeneration,
): Promise<boolean> {
  subagentMap.value = {};
  pendingScrollToEnd.value = false;
  unreadCount.value = 0;
  stopLiveWatch();

  if (generation !== identityGeneration || target.key !== sessionIdentityKey.value) {
    return false;
  }

  try {
    await sessionStream.start("load_session", "session", {
      filePath: target.filePath,
      cliId: target.cliId,
      skipSidechainFilter: target.isSubagent,
    });
    // start() 返回时只代表 invoke 调用回来了，真正完成要等 done 事件
    // —— 见下方 watch(sessionStream.done)
    return generation === identityGeneration && target.key === sessionIdentityKey.value;
  } catch (e) {
    console.error("load_session_stream failed:", e);
    // 复位刷新滚动意图：流未启动，fullyLoaded 不会再触发，否则残留意图会在下次无关加载时误滚到底部
    shouldScrollToBottomOnRefresh.value = false;
    pendingScrollToEnd.value = false;
    return false;
  }
}

// done 事件到达时同步 offset 与 subagent_map（增量轮询要用）
watch(
  () => sessionStream.done.value,
  (done) => {
    if (!done) return;
    currentOffset = done.offset;
    if (done.subagent_map) {
      subagentMap.value = { ...done.subagent_map };
    }
  }
);

// error 事件
watch(
  () => sessionStream.error.value,
  (err) => {
    if (!err) return;
    console.error("load_session_stream failed:", err);
    // 与改造前的 catch 块保持一致：错误时回到空状态，避免半截 chunk 被误认为加载成功
    sessionStream.items.value = [];
    subagentMap.value = {};
    // 复位刷新滚动意图：加载失败不会有 fullyLoaded，残留意图会在下次无关加载时误滚到底部
    shouldScrollToBottomOnRefresh.value = false;
  }
);

let liveWatchInterval: ReturnType<typeof setInterval> | null = null;

async function loadIncremental() {
  const generation = identityGeneration;
  const target = currentSessionLoadTarget();
  const offset = currentOffset;
  try {
    const result = await invokeApp<SessionLoadResult>("load_session_incremental", {
      filePath: target.filePath,
      offset,
      skipSidechainFilter: target.isSubagent,
    });
    if (generation !== identityGeneration || target.key !== sessionIdentityKey.value) return;
    if (result.messages.length > 0) {
      const addedCount = result.messages.length;
      messages.value = [...messages.value, ...result.messages];
      if (!isAtBottom.value) {
        unreadCount.value += addedCount;
      }
    }
    if (result.subagent_map && Object.keys(result.subagent_map).length > 0) {
      subagentMap.value = { ...subagentMap.value, ...result.subagent_map };
    }
    currentOffset = result.offset;
  } catch {
    // File may be temporarily locked, ignore
  }
}

function startLiveWatch() {
  stopLiveWatch();
  liveWatchInterval = setInterval(loadIncremental, 2000);
}

function stopLiveWatch() {
  if (liveWatchInterval) {
    clearInterval(liveWatchInterval);
    liveWatchInterval = null;
  }
}

watch(() => props.autoFollow, (val) => {
  if (val && fullyLoaded.value) {
    nextTick(() => scrollToAbsoluteEnd());
  }
});

const filteredMessages = computed<IndexedChatMessage[]>(() => {
  const filters = {
    user: props.filterUser,
    assistant: props.filterAssistant,
    tool: props.filterTool,
    thinking: props.filterThinking,
  };
  const list = messages.value;
  const len = list.length;
  const result: IndexedChatMessage[] = [];
  for (let i = 0; i < len; i++) {
    const msg = list[i];
    if (isMessageVisible(msg, filters)) {
      result.push({ msg, originalIndex: i });
    }
  }
  return result;
});

const chatContainer = ref<HTMLElement | null>(null);
const searchBarRef = ref<InstanceType<typeof ChatSearchBar> | null>(null);
const fullScreenImageUrl = ref<string | null>(null);

function closeFullscreenImage() {
  fullScreenImageUrl.value = null;
}

async function downloadImage(dataOrUrl: string) {
  const isSvg = dataOrUrl.startsWith("data:image/svg+xml") || dataOrUrl.trim().startsWith("<svg");
  const ext = isSvg ? "svg" : "png";
  const filterName = isSvg ? t("common.chatView.svgImage") : t("common.chatView.pngImage");

  try {
    const { save } = await import('@tauri-apps/plugin-dialog');
    const savePath = await save({
      filters: [{ name: filterName, extensions: [ext] }],
      defaultPath: `image-${Date.now()}.${ext}`,
    });
    if (!savePath) return;

    if (isSvg) {
      const { writeTextFile } = await import('@tauri-apps/plugin-fs');
      let svgText = dataOrUrl;
      if (dataOrUrl.startsWith("data:image/svg+xml")) {
        if (dataOrUrl.includes(";base64,")) {
          svgText = atob(dataOrUrl.split(";base64,")[1]);
        } else {
          const commaIdx = dataOrUrl.indexOf(",");
          svgText = decodeURIComponent(dataOrUrl.slice(commaIdx + 1));
        }
      }
      await writeTextFile(savePath, svgText);
      return;
    }

    const { writeFile } = await import('@tauri-apps/plugin-fs');
    const resp = await fetch(dataOrUrl);
    const buf = await resp.arrayBuffer();
    await writeFile(savePath, new Uint8Array(buf));
  } catch {
    const a = document.createElement("a");
    if (isSvg && dataOrUrl.trim().startsWith("<svg")) {
      const blob = new Blob([dataOrUrl], { type: "image/svg+xml;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      a.href = url;
      a.download = `image-${Date.now()}.${ext}`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } else {
      a.href = dataOrUrl;
      a.download = `image-${Date.now()}.${ext}`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
    }
  }
}

function getAssistantTurnText(turn: TurnNode): string {
  if (!turn.assistantTurn) return "";
  const texts = turn.assistantTurn.segments
    .filter((s): s is TextSegment => s.type === "text")
    .map((s) => s.text);
  if (texts.length > 0) return texts.join("\n");
  const widgets = turn.assistantTurn.segments
    .filter((s): s is WidgetSegment => s.type === "widget")
    .map((s) => t("common.chatView.widgetExportLabel", { title: s.title }));
  if (widgets.length > 0) return widgets.join("\n");
  return "";
}

const pendingJumpIndex = ref<number | null>(null);

const {
  displayMessages,
  hasMoreUp,
  hasMoreDown,
  sentinelUp,
  sentinelDown,
  disableObserver,
  disconnectObserver,
  onChatScroll,
  isAtBottom,
  updateActiveMessageFromScroll,
  saveScrollAnchor,
  restoreScrollAnchor,
  ensureMessageVisible,
  scrollToMessage: scrollToLoadedMessage,
  scrollToAbsoluteEnd,
  scrollToAbsoluteStart,
  loadMoreUp,
  loadMoreDown,
  refreshObserverIfEnabled,
  enableObserver,
} = useChatScroll({
  chatContainer,
  messages,
  filteredMessages,
  fullyLoaded,
  autoFollow: computed(() => props.autoFollow ?? false),
  onActiveMessageChange: (index) => emit("activeMessageChange", index),
});

function handleChatScroll() {
  onChatScroll();
  if (isAtBottom.value) {
    unreadCount.value = 0;
  }
}

const { turns: displayTurns } = useMessageTurns(displayMessages);

let initializedIdentityKey: string | null = null;
let initPromise: Promise<void> | null = null;

async function initSession() {
  const target = currentSessionLoadTarget();
  if (initializedIdentityKey === target.key) return initPromise ?? Promise.resolve();
  initializedIdentityKey = target.key;
  const generation = identityGeneration;
  const promise = (async () => {
    disableObserver();

    const loaded = await loadSession(target, generation);
    if (!loaded || generation !== identityGeneration) return;
    await nextTick();
    if (generation !== identityGeneration) return;
    const el = chatContainer.value;
    if (!el) return;
    el.scrollTop = 0;
    el.removeEventListener("scroll", handleChatScroll);
    el.addEventListener("scroll", handleChatScroll, { passive: true });
    updateActiveMessageFromScroll();
  })();
  initPromise = promise;
  await promise;
  if (initPromise === promise) initPromise = null;
}

onMounted(() => {
  if (props.active) initSession();
});

watch(() => props.active, (active) => {
  if (active) {
    initSession();
    if (fullyLoaded.value) startLiveWatch();
  } else {
    stopLiveWatch();
  }
});

onUnmounted(() => {
  identityGeneration += 1;
  stopLiveWatch();
  disconnectObserver();
  chatContainer.value?.removeEventListener("scroll", handleChatScroll);
  removeProxyStatusListener?.();
});

function isText(part: ContentPart): part is { type: "text"; text: string } {
  return part.type === "text";
}

const isSelectionMode = ref(false);
const showExportDropdown = ref(false);

function onDocumentClick() {
  showExportDropdown.value = false;
}
onMounted(() => document.addEventListener('click', onDocumentClick));
onUnmounted(() => document.removeEventListener('click', onDocumentClick));

const selectedIndexes = ref(new Set<number>());
const lastClickedTurnIdx = ref<number | null>(null);

function enterSelectionMode() {
  isSelectionMode.value = true;
}

function toggleSelectionMode() {
  isSelectionMode.value = !isSelectionMode.value;
  if (!isSelectionMode.value) {
    selectedIndexes.value.clear();
    lastClickedTurnIdx.value = null;
  }
}

// 统计当前选中的完整回合/消息数量（符合用户心智：1个发问或1个AI回答计为1项）
const selectedTurnCount = computed(() => {
  let count = 0;
  for (const turn of displayTurns.value) {
    if (turn.userMessage && selectedIndexes.value.has(turn.userMessage.originalIndex)) {
      count++;
    }
    if (turn.assistantTurn && turn.assistantTurn.originalIndexes.some((idx: number) => selectedIndexes.value.has(idx))) {
      count++;
    }
  }
  return count;
});

function toggleSelectAll() {
  const isAllSelected = selectedTurnCount.value > 0 && selectedTurnCount.value === displayTurns.value.length;
  const newSet = new Set<number>();
  if (!isAllSelected) {
    for (const turn of displayTurns.value) {
      if (turn.userMessage) newSet.add(turn.userMessage.originalIndex);
      if (turn.assistantTurn) turn.assistantTurn.originalIndexes.forEach((idx) => newSet.add(idx));
    }
  }
  selectedIndexes.value = newSet;
  lastClickedTurnIdx.value = null;
}

function onTurnClick(event: MouseEvent, tIdx: number, turn: TurnNode) {
  if (!isSelectionMode.value) return;
  event.stopPropagation();
  const newSet = new Set(selectedIndexes.value);

  // Shift + Click 范围快速连选
  if (event.shiftKey && lastClickedTurnIdx.value !== null) {
    const start = Math.min(lastClickedTurnIdx.value, tIdx);
    const end = Math.max(lastClickedTurnIdx.value, tIdx);
    for (let i = start; i <= end; i++) {
      const targetTurn = displayTurns.value[i];
      if (targetTurn.userMessage) {
        newSet.add(targetTurn.userMessage.originalIndex);
      }
      if (targetTurn.assistantTurn) {
        targetTurn.assistantTurn.originalIndexes.forEach((idx) => newSet.add(idx));
      }
    }
  } else {
    // 单击切换当前 Turn 状态
    if (turn.userMessage) {
      if (newSet.has(turn.userMessage.originalIndex)) {
        newSet.delete(turn.userMessage.originalIndex);
      } else {
        newSet.add(turn.userMessage.originalIndex);
      }
    } else if (turn.assistantTurn) {
      const isSelected = turn.assistantTurn.originalIndexes.some((idx) => newSet.has(idx));
      if (isSelected) {
        turn.assistantTurn.originalIndexes.forEach((idx) => newSet.delete(idx));
      } else {
        turn.assistantTurn.originalIndexes.forEach((idx) => newSet.add(idx));
      }
    }
    lastClickedTurnIdx.value = tIdx;
  }
  selectedIndexes.value = newSet;
}

function exportSelected(format: "txt" | "markdown" | "json" | "jsonl") {
  const { exportSession } = useSessions();
  const sorted = Array.from(selectedIndexes.value).sort((a, b) => a - b);
  exportSession(
    { cliId: effectiveCliId.value, filePath: props.sessionPath },
    format,
    props.projectRoot,
    props.sessionId,
    sorted,
  );
  toggleSelectionMode();
}

async function exportSelectedAsImage() {
  const sorted = Array.from(selectedIndexes.value).sort((a, b) => a - b);
  if (sorted.length === 0) return;

  document.body.style.cursor = 'wait';

  let container: HTMLDivElement | null = null;
  let overlay: HTMLDivElement | null = null;

  try {
    const { toPng } = await import('html-to-image');
    const { save } = await import('@tauri-apps/plugin-dialog');
    const { writeFile } = await import('@tauri-apps/plugin-fs');

    container = document.createElement('div');
    container.className = 'chat-view';
    container.style.backgroundColor = getComputedStyle(chatContainer.value!).getPropertyValue('background-color') || 'var(--color-bg)';
    container.style.position = 'fixed';
    container.style.left = '0';
    container.style.top = '0';
    container.style.width = chatContainer.value?.clientWidth + 'px';
    container.style.height = 'auto';
    container.style.zIndex = '99997';

    if (chatContainer.value) {
      for (const attr of chatContainer.value.attributes) {
        if (attr.name.startsWith('data-v-')) {
          container.setAttribute(attr.name, attr.value);
        }
      }
    }

    const messagesDiv = document.createElement('div');
    messagesDiv.className = 'messages';
    messagesDiv.style.padding = '20px';
    const originalMessagesContainer = chatContainer.value?.querySelector('.messages');
    if (originalMessagesContainer) {
      for (const attr of originalMessagesContainer.attributes) {
        if (attr.name.startsWith('data-v-')) {
          messagesDiv.setAttribute(attr.name, attr.value);
        }
      }
    }

    container.appendChild(messagesDiv);

    const originalSelectedMessages = chatContainer.value?.querySelectorAll('.message.is-selected');
    if (!originalSelectedMessages || originalSelectedMessages.length === 0) return;

    originalSelectedMessages.forEach((msg) => {
      const clone = msg.cloneNode(true) as HTMLElement;
      const checkbox = clone.querySelector('.selection-checkbox-wrapper');
      if (checkbox) checkbox.remove();
      clone.classList.remove('is-selected', 'is-selectable');
      messagesDiv.appendChild(clone);
    });

    chatContainer.value?.parentElement?.appendChild(container);

    overlay = document.createElement('div');
    overlay.style.position = 'fixed';
    overlay.style.inset = '0';
    overlay.style.zIndex = '99998';
    overlay.style.backgroundColor = 'rgba(0, 0, 0, 0.4)';
    overlay.style.backdropFilter = 'blur(8px)';
    (overlay.style as any).WebkitBackdropFilter = 'blur(8px)';
    overlay.style.display = 'flex';
    overlay.style.flexDirection = 'column';
    overlay.style.alignItems = 'center';
    overlay.style.justifyContent = 'center';
    overlay.style.color = '#ffffff';

    overlay.innerHTML = `
      <style>
        .capture-spinner {
          width: 36px;
          height: 36px;
          border: 3px solid rgba(255,255,255,0.2);
          border-radius: 50%;
          border-top-color: #fff;
          animation: capture-spin 1s cubic-bezier(0.4, 0, 0.2, 1) infinite;
          margin-bottom: 16px;
          box-shadow: 0 0 10px rgba(0,0,0,0.1);
        }
        @keyframes capture-spin {
          to { transform: rotate(360deg); }
        }
        .capture-text {
          font-size: 15px;
          font-weight: 500;
          letter-spacing: 0.5px;
          text-shadow: 0 1px 3px rgba(0,0,0,0.3);
        }
      </style>
      <div class="capture-spinner"></div>
      <div class="capture-text">${t("common.chatView.capturingImage")}</div>
    `;
    document.body.appendChild(overlay);

    await new Promise(resolve => setTimeout(resolve, 50));

    const contentHeight = container.scrollHeight;
    const contentWidth = container.scrollWidth;
    let pr = window.devicePixelRatio || 2;
    if (contentHeight * pr > 16384) {
      pr = 16384 / contentHeight;
    }

    const dataUrl = await toPng(container, {
      pixelRatio: pr,
      width: contentWidth,
      height: contentHeight,
      skipFonts: true,
      backgroundColor: getComputedStyle(container).backgroundColor || '#ffffff',
    });

    container.remove();
    overlay.remove();

    const base64Data = dataUrl.replace(/^data:image\/png;base64,/, "");
    const binaryString = window.atob(base64Data);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
        bytes[i] = binaryString.charCodeAt(i);
    }

    const { projects } = useSessions();
    let sessionName = "";
    if (props.sessionId) {
      for (const p of projects.value) {
        const s = p.sessions.find((sess) => sess.session_id === props.sessionId);
        if (s && s.display_name) {
          sessionName = s.display_name;
          break;
        }
      }
    }

    if (sessionName && sessionName.length > 15) {
      sessionName = sessionName.slice(0, 15);
    }

    const projectName = props.projectRoot
      ? props.projectRoot.split(/[\\/]/).filter(Boolean).pop() ?? "session"
      : "session";

    const baseName = sessionName ? `${projectName}_${sessionName}` : projectName;
    const safeName = baseName.replace(/[<>:"/\\|?* ]/g, "_");
    const now = new Date();
    const dateStr = `${now.getFullYear()}${(now.getMonth() + 1).toString().padStart(2, "0")}${now.getDate().toString().padStart(2, "0")}_${now.getHours().toString().padStart(2, "0")}${now.getMinutes().toString().padStart(2, "0")}${now.getSeconds().toString().padStart(2, "0")}`;

    const defaultPath = `${safeName}_${dateStr}_selected.png`;

    const savePath = await save({
      filters: [{ name: t("common.chatView.pngImage"), extensions: ["png"] }],
      defaultPath,
    });

    if (savePath) {
      await writeFile(savePath, bytes);
      toggleSelectionMode();
    }
  } catch (err) {
    console.error("Screenshot failed", err);
    container?.remove();
    overlay?.remove();
    import('@tauri-apps/plugin-dialog').then(({ message }) => {
      message(renderAppError(err), { title: t("common.chatView.exportImageFailed"), kind: "error" });
    });
  } finally {
    document.body.style.cursor = '';
  }
}

const {
  expandedTools,
  searchQuery,
  searchOptions,
  currentSearchIndex,
  totalSearchMatches,
  toggleTool,
  renderTextPart,
  renderToolUseSummary,
  renderToolUseInput,
  renderToolUseMarkdown,
  getWriteMarkdownContent,
  renderToolResultSummary,
  renderToolResultContent,
  syncSearchExpandedTools,
  focusNextSearchMatch,
  focusPrevSearchMatch,
  onSearchInput,
  onToggleCaseSensitive,
  onToggleWholeWord,
  onCloseSearch,
  focusSearch,
  openSearchAt,
  clearRenderedCache,
} = useChatSearch({
  messages,
  chatContainer,
  searchBarRef,
  showSearch: computed(() => props.showSearch),
  filterUser: computed(() => props.filterUser),
  filterAssistant: computed(() => props.filterAssistant),
  filterTool: computed(() => props.filterTool),
  fullyLoaded,
  initSession,
  ensureMessageVisible,
  emitCloseSearch: () => emit("closeSearch"),
});

function onMarkdownClick(event: MouseEvent) {
  void handleMarkdownLinkClick(event, {
    projectRoot: props.projectRoot,
    onOpenFile: ({ path }) => {
      if (props.projectRoot) {
        emit("openFile", path, props.projectRoot);
      }
    },
    onFailure: (failure) => console.warn("Markdown link was not opened:", failure),
  });
}

function getMessageText(msg: ChatMessage): string {
  return msg.content_parts
    .filter(isText)
    .map((p) => p.text)
    .join("\n");
}

// Fork 锚点：assistant turn 由多条原始消息合并而成，锚定最后一条才能保住整轮输出
function assistantTurnForkUuid(turn: TurnNode): string | undefined {
  const idxs = turn.assistantTurn?.originalIndexes;
  if (!idxs?.length) return undefined;
  return messages.value[idxs[idxs.length - 1]]?.uuid;
}

// ─── 段落渲染入参 ───────────────────────────────────

/**
 * 文本段 → ChatMessageBody 的 `parts` 入参。
 *
 * 一个 text 段可能是同一条消息内**连续**多段文本合并的结果（见 useMessageTurns）。
 * 渲染缓存以 (originalIndex, partIndex) 为键，所以必须逐 part 回查，只按首段的
 * partIndex 取一次会让后续正文静默消失（搜索高亮也会只覆盖首段）。
 */
function textSegmentParts(seg: TextSegment): Array<{ type: "text"; text: string }> {
  return seg.textParts.map((p) => ({ type: "text" as const, text: p.text }));
}

// ─── 气泡式会话：时间分隔条 ─────────────────────────
// 相邻消息间隔超过 1 小时才插分隔条，只在跨明显时段（午休/隔夜等）时标记
const DIVIDER_GAP_MS = 60 * 60 * 1000;

function turnTimestamp(turn: TurnNode): string | null {
  return turn.userMessage?.msg.timestamp ?? turn.assistantTurn?.timestamp ?? null;
}

/** 相邻 turn 时间间隔超过阈值时在中间插时间分隔条 */
function shouldShowDivider(turn: TurnNode, index: number): boolean {
  if (index === 0) return false;
  const prev = turnTimestamp(displayTurns.value[index - 1]);
  const cur = turnTimestamp(turn);
  if (!prev || !cur) return false;
  const prevTime = new Date(prev).getTime();
  const curTime = new Date(cur).getTime();
  if (isNaN(prevTime) || isNaN(curTime)) return false;
  return curTime - prevTime > DIVIDER_GAP_MS;
}

/** 分隔条文案：当天只显示 HH:MM，跨天加 MM-DD */
function dividerLabel(ts: string): string {
  const d = new Date(ts);
  if (isNaN(d.getTime())) return ts;
  const now = new Date();
  const h = String(d.getHours()).padStart(2, "0");
  const min = String(d.getMinutes()).padStart(2, "0");
  const sameDay = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  if (sameDay) return `${h}:${min}`;
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${m}-${day} ${h}:${min}`;
}

const sessionIdentityReady = computed(() => !!props.sessionId);

function isBookmarked(idx: number): boolean {
  if (!sessionIdentityReady.value) return false;
  return props.bookmarks.some(
    (b) => b.sessionId === props.sessionId && b.messageIndex === idx
  );
}

function scrollToMessage(index: number) {
  if (!scrollToLoadedMessage(index)) {
    pendingJumpIndex.value = index;
  }
}

const shouldScrollToBottomOnRefresh = ref(false);
const pendingScrollToEnd = ref(false);

function handleScrollToBottom() {
  if (!fullyLoaded.value) {
    pendingScrollToEnd.value = true;
  }
  unreadCount.value = 0;
  scrollToAbsoluteEnd();
}

async function refreshSession(): Promise<boolean> {
  if (isRefreshingSession.value || sessionStream.loading.value) return false;
  isRefreshingSession.value = true;

  // 1. 精准记录视口滚动意图：
  //    - 用户当前在底部（或 autoFollow）：刷新完成后平滑贴底
  //    - 用户正在浏览历史消息：记录当前可视元素锚点，刷新完成后精准还原视口偏移，避免产生任何跳动
  const wasAtBottom = isAtBottom.value || (props.autoFollow ?? false);
  if (!wasAtBottom) {
    saveScrollAnchor();
  }
  unreadCount.value = 0;
  stopLiveWatch();

  const target = currentSessionLoadTarget();
  const generation = identityGeneration;

  try {
    // 2. SWR 原子刷新模式：保留旧消息正常展示，不触发骨架屏与 DOM 销毁
    await sessionStream.start(
      "load_session",
      "session",
      {
        filePath: target.filePath,
        cliId: target.cliId,
        skipSidechainFilter: target.isSubagent,
      },
      { atomicSwap: true }
    );

    const donePayload = await sessionStream.waitForCompletion(8000);
    if (!donePayload || generation !== identityGeneration || target.key !== sessionIdentityKey.value) {
      return false;
    }

    // 刷新可能包含新生成的消息修改，清空按序号索引的渲染缓存，确保显示最新内容
    clearRenderedCache();

    // 3. 数据到齐且 DOM patch 之后，平滑恢复滚动位置
    await nextTick();
    if (wasAtBottom) {
      scrollToAbsoluteEnd();
    } else {
      restoreScrollAnchor();
    }
    return true;
  } catch (e) {
    console.error("refreshSession failed:", e);
    return false;
  } finally {
    isRefreshingSession.value = false;
    if (props.active && fullyLoaded.value) {
      startLiveWatch();
    }
  }
}

watch(fullyLoaded, (loaded) => {
  if (!loaded) return;
  // done 之后才允许 live-watch 启动：避免流式加载窗口内 currentOffset=0 被 setInterval 用作起点
  // 触发 load_session_incremental 返回全量消息、再 append 一次，造成消息重复
  if (props.active && !isRefreshingSession.value) startLiveWatch();
  // 启用观察器：哨兵进入视口附近即静默自动加载，不再停一个大按钮
  enableObserver();

  // 刷新会话由 refreshSession 自身在流完成时平滑还原滚动锚点/贴底，不在此重置视口
  if (isRefreshingSession.value) return;

  if (pendingJumpIndex.value !== null) {
    const i = pendingJumpIndex.value;
    pendingJumpIndex.value = null;
    nextTick(() => scrollToMessage(i));
  } else if (shouldScrollToBottomOnRefresh.value || props.autoFollow || pendingScrollToEnd.value) {
    shouldScrollToBottomOnRefresh.value = false;
    pendingScrollToEnd.value = false;
    nextTick(() => scrollToAbsoluteEnd());
  } else {
    // 非跟随（历史）进入：窗口可能在流式首包时被钉死在小值上，
    // 复位到首屏整页，避免进页面就露"加载更多"哨兵
    scrollToAbsoluteStart();
  }
});

const filteredMessageCount = computed(() => filteredMessages.value.length);

defineExpose({ scrollToMessage, openSearchAt, focusSearch, messages, filteredMessageCount, subagentMap, loading, saveScrollAnchor, restoreScrollAnchor, enterSelectionMode, refresh: refreshSession });

watch(
  () => messages.value.length,
  async (newCount, oldCount) => {
    if (newCount === 0 || oldCount === undefined) return;
    await nextTick();
    const el = chatContainer.value;
    if (!el) return;

    // 流式加载中（done 之前）每个 chunk 都会触发，这里不抢滚动权
    if (!fullyLoaded.value) return;

    if (newCount > oldCount && (isAtBottom.value || props.autoFollow)) {
      scrollToAbsoluteEnd();
      unreadCount.value = 0;
    }

    if (searchQuery.value.trim()) {
      syncSearchExpandedTools();
    }

    refreshObserverIfEnabled();
  },
);

// ─── API request association ─────────────────────
interface TrafficDetail {
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
   * 与 `useProxy.ts` 的那份 `TrafficDetail` 是**两份独立声明**（各自贴着 IPC 的一端），
   * 加字段时两处都要改：只改一处不会编译报错，只会让提示句在这里静默不出现。
   */
  error_kind: string | null;
  res_size: number;
  duration_ms: number;
}

// 当前会话的流量记录时间戳（升序 epoch ms），用于按消息精确判断是否显示 ⚡ 按钮
const proxyTrafficTs = ref<number[]>([]);
const apiExpandedIndex = ref<number | null>(null);
const apiDetail = ref<TrafficDetail | null>(null);
const apiLoading = ref(false);
const apiNotFound = ref(false);
const apiDetailTab = ref<"request" | "response">("request");
const apiDetailSection = ref<"body" | "headers">("body");

let removeProxyStatusListener: (() => void) | null = null;

function resetApiDetailState() {
  apiExpandedIndex.value = null;
  apiDetail.value = null;
  apiLoading.value = false;
  apiNotFound.value = false;
}

async function refreshProxyTimestamps() {
  if (props.sessionId) {
    try {
      const timestamps = await invokeApp<string[]>("proxy_session_traffic_timestamps", {
        cliId: effectiveCliId.value,
        sessionId: props.sessionId,
      });
      proxyTrafficTs.value = timestamps
        .map((t) => Date.parse(t))
        .filter((t) => !Number.isNaN(t))
        .sort((a, b) => a - b);
    } catch {
      proxyTrafficTs.value = [];
    }
  } else {
    proxyTrafficTs.value = [];
  }

  if (proxyTrafficTs.value.length === 0) {
    resetApiDetailState();
  }
}

// 与后端 proxy_find_by_timestamp 一致：120s 窗口内的最近邻记录即视为该消息有对应流量
function hasProxyTraffic(timestamp: string): boolean {
  const list = proxyTrafficTs.value;
  if (list.length === 0) return false;
  const target = Date.parse(timestamp);
  if (Number.isNaN(target)) return false;
  let lo = 0;
  let hi = list.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid] < target) lo = mid + 1;
    else hi = mid;
  }
  const WINDOW_MS = 120_000;
  return (
    (lo < list.length && Math.abs(list[lo] - target) < WINDOW_MS) ||
    (lo > 0 && Math.abs(list[lo - 1] - target) < WINDOW_MS)
  );
}

// 向 SessionHeader 上报当前会话的代理记录数；仅 active 时上报，切回 Tab 时重新上报
watch([proxyTrafficTs, () => props.active], () => {
  if (props.active) emit("proxyTrafficCountChange", proxyTrafficTs.value.length);
}, { immediate: true });

onMounted(async () => {
  await refreshProxyTimestamps();
  // 代理运行期间可能产生新的流量记录，状态变化时刷新一次
  removeProxyStatusListener = addProxyStatusChangedListener(({ cliId }) => {
    if (cliId !== effectiveCliId.value) return;
    refreshProxyTimestamps();
  });
});

watch(effectiveCliId, () => {
  resetApiDetailState();
  refreshProxyTimestamps();
});

function resetForSessionIdentityChange() {
  identityGeneration += 1;
  initializedIdentityKey = null;
  sessionStream.cancel();
  messages.value = [];
  sessionStream.done.value = null;
  sessionStream.error.value = null;
  currentOffset = 0;
  subagentMap.value = {};
  pendingJumpIndex.value = null;
  shouldScrollToBottomOnRefresh.value = false;
  stopLiveWatch();
  disableObserver();
  clearRenderedCache();
  searchQuery.value = "";
  currentSearchIndex.value = 0;
  searchBarRef.value?.setQuery("");
  for (const key of Object.keys(expandedTools)) delete expandedTools[key];
  isSelectionMode.value = false;
  selectedIndexes.value = new Set();
  lastClickedTurnIdx.value = null;
  fullScreenImageUrl.value = null;
  resetApiDetailState();
  refreshProxyTimestamps();
  if (chatContainer.value) chatContainer.value.scrollTop = 0;
  emit("activeMessageChange", null);
}

watch(
  sessionIdentityKey,
  (nextIdentity, previousIdentity) => {
    if (nextIdentity === previousIdentity) return;
    resetForSessionIdentityChange();
    if (props.active) void initSession();
  },
  { flush: "sync" },
);

async function toggleApiDetail(index: number, timestamp: string) {
  if (!sessionIdentityReady.value) return;
  if (apiExpandedIndex.value === index) {
    apiExpandedIndex.value = null;
    apiDetail.value = null;
    apiNotFound.value = false;
    return;
  }
  apiExpandedIndex.value = index;
  apiDetail.value = null;
  apiNotFound.value = false;
  apiDetailTab.value = "request";
  apiDetailSection.value = "body";
  apiLoading.value = true;
  try {
    const detail = await invokeApp<TrafficDetail | null>("proxy_find_by_timestamp", {
      cliId: effectiveCliId.value,
      sessionId: props.sessionId,
      timestamp,
    });
    if (detail) {
      apiDetail.value = detail;
    } else {
      apiNotFound.value = true;
    }
  } catch {
    apiNotFound.value = true;
  } finally {
    apiLoading.value = false;
  }
}

function apiBodyHtml(): string {
  if (!apiDetail.value) return "";
  if (apiDetailTab.value === "request") {
    return highlightJson(apiDetail.value.req_body);
  }
  // 压缩响应没有可高亮的正文，直接给出提示句。
  // 这一支今天是**冗余的**：`highlightSseResponse` 对以 `[` 开头的串本就转给
  // `highlightJson`，而后者对非 JSON 原样转义，四语提示句的首字符恰好都是 `[`。
  // 留着它是为了不把「压缩响应显示什么」绑在提示句的首字符形状上——那是排版，
  // 不是契约，改个标点就会掉进 SSE 聚合分支渲染成 `{}`。
  const notice = compressionNotice(apiDetail.value);
  if (notice) return highlightJson(notice);
  // 上游失败时正文区显示提示句，与压缩那一支同序（压缩 → 上游失败）、同样放在
  // SSE/JSON 管线之前：提示句不是正文。
  // 这一支是承重的：拿掉它，下面的回退会把第三方原文直接送进管线，提示句根本不出现。
  // 提示句里含第三方原文，`highlightJson` 对非 JSON 走 `escapeHtml`，故不会被当成 HTML 解释。
  const errNotice = errorNotice(apiDetail.value);
  if (errNotice) return highlightJson(errNotice);
  return highlightSseResponse(apiDetail.value.res_body);
}

function apiHeadersHtml(): string {
  if (!apiDetail.value) return "";
  const raw = apiDetailTab.value === "request"
    ? apiDetail.value.req_headers
    : apiDetail.value.res_headers;
  return highlightJson(raw);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
</script>

<template>
  <Teleport v-if="showSearch" :to="searchTeleportTarget" :disabled="!active">
    <ChatSearchBar
      ref="searchBarRef"
      toolbar
      :currentIndex="currentSearchIndex"
      :totalMatches="totalSearchMatches"
      :caseSensitive="searchOptions.caseSensitive"
      :wholeWord="searchOptions.wholeWord"
      @close="onCloseSearch"
      @search="onSearchInput"
      @toggle-case-sensitive="onToggleCaseSensitive"
      @toggle-whole-word="onToggleWholeWord"
      @next="focusNextSearchMatch"
      @prev="focusPrevSearchMatch"
    />
  </Teleport>
  <div class="chat-wrapper">
    <div class="chat-view" ref="chatContainer">
      <div class="chat-state-stage">
        <LoadingSkeleton v-if="showInitialLoading" />
        <div v-else-if="displayMessages.length === 0" class="messages-empty">{{ t("common.chatView.allTypesHidden") }}</div>
        <div v-else class="messages">
            <div v-if="streamingProgress" class="streaming-indicator">
              <SvgIcon name="loader" :size="14" class="spinning" />
              <span>{{ t("common.chatView.streamingProgress", { count: messages.length }) }}</span>
            </div>
            <div v-if="hasMoreUp" ref="sentinelUp" class="sentinel" @click="loadMoreUp">
              <span class="load-more-text">{{ t("common.chatView.loadMore") }}</span>
            </div>
            <!-- 注意：不要为此列表引入 v-memo。曾实现过（含完整依赖列表），
                 但 150 行窗口切片使行频繁进出，缓存 vnode 维护成本高、风险大；
                 窗口化本身已限制渲染规模，常规补丁路径足够。 -->
            <template v-for="(turn, tIdx) in displayTurns" :key="turn.id">
              <ChatTimeDivider
                v-if="shouldShowDivider(turn, tIdx)"
                :label="dividerLabel(turnTimestamp(turn) || '')"
              />

              <!-- USER MESSAGE -->
              <div
                v-if="turn.userMessage"
                :data-msg-index="turn.userMessage.originalIndex"
                class="message message-user"
                :class="{
                  'is-selectable': isSelectionMode,
                  'is-selected': selectedIndexes.has(turn.userMessage.originalIndex)
                }"
                @click="onTurnClick($event, tIdx, turn)"
              >
                <div v-if="isSelectionMode" class="selection-checkbox-wrapper">
                  <div class="selection-checkbox">
                    <SvgIcon v-if="selectedIndexes.has(turn.userMessage.originalIndex)" name="check" :size="12" />
                  </div>
                </div>
                <div class="msg-column">
                  <div class="message-card bubble bubble-user">
                    <ChatMessageBody
                      :parts="turn.userMessage.msg.content_parts"
                      :message-index="turn.userMessage.originalIndex"
                      :filter-tool="props.filterTool"
                      :subagent-map="subagentMap"
                      :expanded-tools="expandedTools"
                      :render-text-part="renderTextPart"
                      :render-tool-use-summary="renderToolUseSummary"
                      :render-tool-use-input="renderToolUseInput"
                      :render-tool-use-markdown="renderToolUseMarkdown"
                      :render-tool-result-summary="renderToolResultSummary"
                      :render-tool-result-content="renderToolResultContent"
                      :get-write-markdown-content="getWriteMarkdownContent"
                      @toggle-tool="toggleTool"
                      @markdown-click="onMarkdownClick"
                      @open-subagent="(filePath, label) => $emit('openSubagent', filePath, label)"
                      @preview-image="fullScreenImageUrl = $event"
                      @download-image="downloadImage"
                    />
                  </div>
                  <ChatMessageHeader
                    :role="turn.userMessage.msg.role"
                    :timestamp="formatTimestamp(turn.userMessage.msg.timestamp)"
                    :model="turn.userMessage.msg.model"
                    :token-usage="turn.userMessage.msg.token_usage"
                    :message-text="getMessageText(turn.userMessage.msg)"
                    :bookmarked="isBookmarked(turn.userMessage.originalIndex)"
                    :bookmark-disabled="!sessionIdentityReady"
                    :fork-available="canFork && !!turn.userMessage.msg.uuid"
                    :proxy-available="hasProxyTraffic(turn.userMessage.msg.timestamp)"
                    :api-active="apiExpandedIndex === turn.userMessage.originalIndex"
                    :api-disabled="!sessionIdentityReady"
                    @toggle-bookmark="$emit('toggleBookmark', turn.userMessage.originalIndex)"
                    @fork-from-here="$emit('forkFromHere', turn.userMessage.msg.uuid!)"
                    @toggle-api-detail="toggleApiDetail(turn.userMessage.originalIndex, turn.userMessage.msg.timestamp)"
                  />
                  <div class="panel-slot">
                    <Transition name="api-detail-expand">
                      <ChatApiDetailPanel
                        v-if="apiExpandedIndex === turn.userMessage.originalIndex"
                        :loading="apiLoading"
                        :not-found="apiNotFound"
                        :detail="apiDetail"
                        :detail-tab="apiDetailTab"
                        :detail-section="apiDetailSection"
                        :body-html="apiBodyHtml()"
                        :headers-html="apiHeadersHtml()"
                        :format-bytes="formatBytes"
                        @update-detail-tab="apiDetailTab = $event"
                        @update-detail-section="apiDetailSection = $event"
                      />
                    </Transition>
                  </div>
                </div>
                <div class="avatar-col">
                  <ChatAvatar role="user" />
                </div>
              </div>

              <!-- ASSISTANT TURN -->
              <div
                v-if="turn.assistantTurn"
                :data-msg-index="turn.assistantTurn.originalIndexes.join(' ')"
                class="message message-assistant"
                :class="{
                  'is-selectable': isSelectionMode,
                  'is-selected': turn.assistantTurn.originalIndexes.some(i => selectedIndexes.has(i))
                }"
                @click="onTurnClick($event, tIdx, turn)"
              >
                <div v-if="isSelectionMode" class="selection-checkbox-wrapper">
                  <div class="selection-checkbox">
                    <SvgIcon v-if="turn.assistantTurn.originalIndexes.some(i => selectedIndexes.has(i))" name="check" :size="12" />
                  </div>
                </div>
                <div class="avatar-col">
                  <ChatAvatar role="assistant" :model="turn.assistantTurn.model" :cli-id="effectiveCliId" />
                </div>
                <div class="msg-column">
                  <template v-for="(seg, sIdx) in turn.assistantTurn.segments" :key="sIdx">
                    <div
                      v-if="seg.type === 'text'"
                      class="message-card bubble bubble-assistant"
                    >
                      <ChatMessageBody
                        :parts="textSegmentParts(seg)"
                        :message-index="seg.originalIndex"
                        :part-index-base="seg.textParts[0].index"
                        :filter-tool="props.filterTool"
                        :subagent-map="subagentMap"
                        :expanded-tools="expandedTools"
                        :render-text-part="renderTextPart"
                        :render-tool-use-summary="renderToolUseSummary"
                        :render-tool-use-input="renderToolUseInput"
                        :render-tool-use-markdown="renderToolUseMarkdown"
                        :render-tool-result-summary="renderToolResultSummary"
                        :render-tool-result-content="renderToolResultContent"
                        :get-write-markdown-content="getWriteMarkdownContent"
                        @toggle-tool="toggleTool"
                        @markdown-click="onMarkdownClick"
                        @open-subagent="(filePath, label) => $emit('openSubagent', filePath, label)"
                        @preview-image="fullScreenImageUrl = $event"
                        @download-image="downloadImage"
                      />
                    </div>
                    <div
                      v-else-if="seg.type === 'thinking' && filterThinking"
                      class="card-outside"
                    >
                      <ChatMessageBody
                        :parts="[{ type: 'thinking', thinking: seg.thinking }]"
                        :message-index="seg.originalIndex"
                        :filter-tool="props.filterTool"
                        :subagent-map="subagentMap"
                        :expanded-tools="expandedTools"
                        :render-text-part="renderTextPart"
                        :render-tool-use-summary="renderToolUseSummary"
                        :render-tool-use-input="renderToolUseInput"
                        :render-tool-use-markdown="renderToolUseMarkdown"
                        :render-tool-result-summary="renderToolResultSummary"
                        :render-tool-result-content="renderToolResultContent"
                        :get-write-markdown-content="getWriteMarkdownContent"
                        @toggle-tool="toggleTool"
                        @markdown-click="onMarkdownClick"
                        @open-subagent="(filePath, label) => $emit('openSubagent', filePath, label)"
                        @preview-image="fullScreenImageUrl = $event"
                        @download-image="downloadImage"
                      />
                    </div>
                    <div
                      v-else-if="seg.type === 'widget'"
                      class="card-outside"
                    >
                      <ChatSvgWidgetCard
                        :title="seg.title"
                        :svg-code="seg.code"
                        @preview="fullScreenImageUrl = $event"
                        @download="downloadImage"
                      />
                    </div>
                    <div
                      v-else-if="seg.type === 'tool_group' && filterTool"
                      class="card-outside"
                    >
                      <ChatToolGroup
                        :segment="seg"
                        :subagent-map="subagentMap"
                        :expanded-tools="expandedTools"
                        :render-tool-use-summary="renderToolUseSummary"
                        :render-tool-use-input="renderToolUseInput"
                        :render-tool-use-markdown="renderToolUseMarkdown"
                        :render-tool-result-summary="renderToolResultSummary"
                        :render-tool-result-content="renderToolResultContent"
                        :get-write-markdown-content="getWriteMarkdownContent"
                        @toggle-tool="toggleTool"
                        @markdown-click="onMarkdownClick"
                        @open-subagent="(filePath, label) => $emit('openSubagent', filePath, label)"
                      />
                    </div>
                  </template>
                  <ChatMessageHeader
                    role="assistant"
                    :timestamp="formatTimestamp(turn.assistantTurn.timestamp)"
                    :model="turn.assistantTurn.model"
                    :token-usage="turn.assistantTurn.token_usage"
                    :message-text="getAssistantTurnText(turn)"
                    :bookmarked="isBookmarked(turn.assistantTurn.originalIndexes[0])"
                    :bookmark-disabled="!sessionIdentityReady"
                    :fork-available="canFork && !!assistantTurnForkUuid(turn)"
                    :proxy-available="hasProxyTraffic(turn.assistantTurn.timestamp)"
                    :api-active="apiExpandedIndex === turn.assistantTurn.originalIndexes[0]"
                    :api-disabled="!sessionIdentityReady"
                    @toggle-bookmark="$emit('toggleBookmark', turn.assistantTurn.originalIndexes[0])"
                    @fork-from-here="$emit('forkFromHere', assistantTurnForkUuid(turn)!)"
                    @toggle-api-detail="toggleApiDetail(turn.assistantTurn.originalIndexes[0], turn.assistantTurn.timestamp)"
                  />
                  <div class="panel-slot">
                    <Transition name="api-detail-expand">
                      <ChatApiDetailPanel
                        v-if="apiExpandedIndex === turn.assistantTurn.originalIndexes[0]"
                        :loading="apiLoading"
                        :not-found="apiNotFound"
                        :detail="apiDetail"
                        :detail-tab="apiDetailTab"
                        :detail-section="apiDetailSection"
                        :body-html="apiBodyHtml()"
                        :headers-html="apiHeadersHtml()"
                        :format-bytes="formatBytes"
                        @update-detail-tab="apiDetailTab = $event"
                        @update-detail-section="apiDetailSection = $event"
                      />
                    </Transition>
                  </div>
                </div>
              </div>
            </template>
            <div v-if="hasMoreDown" ref="sentinelDown" class="sentinel" @click="loadMoreDown">
              <span class="load-more-text">{{ t("common.chatView.loadMore") }}</span>
            </div>
          </div>
        </div>
      </div>
    <ScrollToBottom
      :container="chatContainer"
      :unread-count="unreadCount"
      @scrollToEnd="handleScrollToBottom"
      @scrollToTop="scrollToAbsoluteStart"
    />
    <ChatSelectionPill
      v-if="isSelectionMode"
      :selected-count="selectedTurnCount"
      :total-count="displayTurns.length"
      :export-dropdown-visible="showExportDropdown"
      @toggle-select-all="toggleSelectAll"
      @toggle-export-dropdown="showExportDropdown = !showExportDropdown"
      @export-selected="exportSelected"
      @export-image="exportSelectedAsImage"
      @cancel="toggleSelectionMode"
    />

    <ChatImageFullscreenModal
      :image-url="fullScreenImageUrl"
      @close="closeFullscreenImage"
      @download="downloadImage"
    />
  </div>
</template>

<style scoped>
.fade-in {
  animation: fadeIn 0.2s ease;
}
@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}
.chat-wrapper {
  flex: 1;
  position: relative;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.chat-view {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-4);
}
.chat-state-stage {
  min-height: 100%;
}
.messages {
  max-width: 800px;
  margin: 0 auto;
  box-sizing: border-box;
}
.messages-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 240px;
  font-size: var(--text-sm);
  color: var(--color-text-muted);
}
.streaming-indicator {
  position: sticky;
  top: 0;
  z-index: 5;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  margin-bottom: var(--space-3);
  background: color-mix(in srgb, var(--color-primary) 12%, var(--color-bg));
  color: var(--color-primary);
  border: 1px solid color-mix(in srgb, var(--color-primary) 28%, transparent);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  -webkit-user-select: none;
  user-select: none;
  backdrop-filter: blur(4px);
}
.streaming-indicator .spinning {
  animation: streamSpin 1s linear infinite;
}
@keyframes streamSpin {
  to { transform: rotate(360deg); }
}
.message {
  margin-bottom: var(--space-5);
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
.message-user {
  justify-content: flex-end;
}
.message-assistant {
  justify-content: flex-start;
}
.avatar-col {
  flex-shrink: 0;
  margin-top: 2px;
}
.msg-column {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.message-user .msg-column {
  align-items: flex-end;
  flex: 0 1 auto;
  max-width: 85%;
}
.message-assistant .msg-column {
  align-items: flex-start;
  flex: 1 1 auto;
  max-width: calc(100% - 44px);
  width: 100%;
}
/* 思考/工具等富内容块：全宽卡片 */
.card-outside {
  width: 100%;
}
.panel-slot {
  width: 100%;
}
.message.message-flash .bubble,
.message.message-flash .message-card {
  animation: messageFlash 2s ease;
}
@keyframes messageFlash {
  0% {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-warning) 42%, transparent);
    background: color-mix(in srgb, var(--color-warning) 14%, var(--color-bg));
  }
  100% {
    box-shadow: none;
  }
}
.bubble,
.message-card {
  border-radius: var(--radius-xl);
  overflow: hidden;
  box-sizing: border-box;
}
.bubble-user,
.message-user .message-card {
  width: fit-content;
  max-width: 100%;
  background: var(--color-role-user-bg);
  border: 1px solid color-mix(in srgb, var(--color-primary) 20%, transparent);
  padding: 10px 16px;
  border-radius: 18px;
  border-bottom-right-radius: 4px;
  box-shadow: var(--shadow-sm);
}
.bubble-assistant,
.message-assistant .message-card {
  width: 100%;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border-light);
  padding: 14px 18px;
  border-radius: 16px;
  border-bottom-left-radius: 4px;
  box-shadow: var(--shadow-sm);
}
.sentinel {
  text-align: center;
  padding: var(--space-4);
  min-height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-fast);
  -webkit-user-select: none;
  user-select: none;
}
.sentinel:hover {
  background: var(--color-bg-hover);
}
.sentinel:hover .load-more-text {
  color: var(--color-primary);
}
.load-more-text {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
/* API request association */
</style>

<style scoped>
.message {
  position: relative;
  transition: padding-left var(--transition-base), background-color var(--transition-fast);
  border-radius: var(--radius-lg);
}

/* 经典多选模式：整行可点，平滑左滑让出 Checkbox 空间 */
.message.is-selectable {
  cursor: pointer;
  padding: var(--space-2) var(--space-3) var(--space-2) 42px;
  margin-left: -8px;
  margin-right: -8px;
}

.message.is-selectable:hover {
  background-color: var(--color-surface-hover, rgba(125, 125, 125, 0.05));
}

.message.is-selected {
  background-color: color-mix(in srgb, var(--color-primary) 7%, transparent);
}

/* 左侧精致极简的 Checkbox 圆标 */
.selection-checkbox-wrapper {
  position: absolute;
  left: 12px;
  top: 14px;
  width: 22px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 2;
  pointer-events: none;
}

.selection-checkbox {
  width: 19px;
  height: 19px;
  border-radius: 50%;
  border: 1.5px solid var(--color-border-hover, rgba(140, 140, 140, 0.45));
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: var(--color-surface, var(--color-bg));
  color: #ffffff;
  transition: all var(--transition-fast);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
}

.message.is-selectable:hover .selection-checkbox {
  border-color: var(--color-primary);
  transform: scale(1.08);
}

.message.is-selected .selection-checkbox {
  background-color: var(--color-primary);
  border-color: var(--color-primary);
  box-shadow: 0 2px 8px color-mix(in srgb, var(--color-primary) 35%, transparent);
}
</style>
