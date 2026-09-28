import { computed, nextTick, reactive, ref, shallowRef, watch, type ComputedRef, type Ref } from "vue";
import type { ChatMessage } from "../types/session";
import { renderMarkdown } from "../utils/markdown";
import { cleanUserText } from "../utils/textClean";

type SearchSegmentType =
  | "text"
  | "tool_use_summary"
  | "tool_use_input"
  | "tool_use_markdown"
  | "tool_result_summary"
  | "tool_result_content";

interface SearchMatch {
  id: string;
  segmentKey: string;
  messageIndex: number;
  partIndex: number;
  segmentType: SearchSegmentType;
  start: number;
  end: number;
  toolKey?: string;
}

interface TextNodeSegment {
  key: string;
  text: string;
}

interface SearchState {
  matches: SearchMatch[];
  segmentMatches: Map<string, SearchMatch[]>;
  matchedToolKeys: Set<string>;
}

interface SearchOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
}

interface SearchRange {
  start: number;
  end: number;
}

interface SearchBarLike {
  focusInput: () => void;
  setQuery: (query: string) => void;
}

interface UseChatSearchOptions {
  messages: Ref<ChatMessage[]>;
  chatContainer: Ref<HTMLElement | null>;
  searchBarRef: Ref<SearchBarLike | null>;
  showSearch: ComputedRef<boolean>;
  filterUser: ComputedRef<boolean>;
  filterAssistant: ComputedRef<boolean>;
  filterTool: ComputedRef<boolean>;
  fullyLoaded: ComputedRef<boolean>;
  initSession: () => Promise<void>;
  ensureMessageVisible: (index: number) => Promise<void>;
  emitCloseSearch: () => void;
}

const CHAT_SEARCH_OPTIONS_STORAGE_KEY = "seshbuddy-chat-search-options";

function loadPersistedSearchOptions(): SearchOptions {
  try {
    const raw = localStorage.getItem(CHAT_SEARCH_OPTIONS_STORAGE_KEY);
    if (!raw) {
      return {
        caseSensitive: false,
        wholeWord: false,
      };
    }

    const parsed = JSON.parse(raw) as Partial<SearchOptions>;
    return {
      caseSensitive: parsed.caseSensitive === true,
      wholeWord: parsed.wholeWord === true,
    };
  } catch {
    return {
      caseSensitive: false,
      wholeWord: false,
    };
  }
}

function persistSearchOptions(options: SearchOptions) {
  try {
    localStorage.setItem(CHAT_SEARCH_OPTIONS_STORAGE_KEY, JSON.stringify(options));
  } catch {
    // ignore persistence failures
  }
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function isAsciiWordChar(char: string | undefined): boolean {
  return !!char && /[0-9A-Za-z_]/.test(char);
}

function isWholeWordRange(text: string, start: number, end: number): boolean {
  const prevChar = start > 0 ? text[start - 1] : undefined;
  const nextChar = end < text.length ? text[end] : undefined;
  return !isAsciiWordChar(prevChar) && !isAsciiWordChar(nextChar);
}

function findMatchRanges(text: string, query: string, options: SearchOptions): SearchRange[] {
  if (!text || !query) {
    return [];
  }

  const haystack = options.caseSensitive ? text : text.toLowerCase();
  const needle = options.caseSensitive ? query : query.toLowerCase();
  const ranges: SearchRange[] = [];
  let startIndex = 0;

  while (startIndex <= haystack.length - needle.length) {
    const matchIndex = haystack.indexOf(needle, startIndex);
    if (matchIndex === -1) {
      break;
    }

    const endIndex = matchIndex + needle.length;
    if (!options.wholeWord || isWholeWordRange(text, matchIndex, endIndex)) {
      ranges.push({ start: matchIndex, end: endIndex });
    }
    startIndex = matchIndex + Math.max(needle.length, 1);
  }

  return ranges;
}

export function useChatSearch(options: UseChatSearchOptions) {
  const expandedTools: Record<string, boolean> = reactive({});
  const searchExpandedKeys = new Set<string>();
  const searchQuery = ref("");
  const searchOptions = ref<SearchOptions>(loadPersistedSearchOptions());
  const currentSearchIndex = ref(0);
  const renderedCache = shallowRef(new Map<string, any>());
  const pendingSearch = ref<string | null>(null);

  watch(
    searchOptions,
    (value) => {
      persistSearchOptions(value);
    },
    { deep: true }
  );

  function toggleTool(key: string) {
    expandedTools[key] = !expandedTools[key];
  }

  function makeToolKey(messageIndex: number, partIndex: number): string {
    return `${messageIndex}-${partIndex}`;
  }

  function makeSegmentKey(
    messageIndex: number,
    partIndex: number,
    segmentType: SearchSegmentType,
    nodeIndex?: number
  ): string {
    return nodeIndex === undefined
      ? `search-${messageIndex}-${partIndex}-${segmentType}`
      : `search-${messageIndex}-${partIndex}-${segmentType}-${nodeIndex}`;
  }

  function extractHtmlTextNodes(
    html: string,
    messageIndex: number,
    partIndex: number,
    segmentType: SearchSegmentType = "text"
  ): TextNodeSegment[] {
    const template = document.createElement("template");
    template.innerHTML = html;

    const walker = document.createTreeWalker(template.content, NodeFilter.SHOW_TEXT, null);
    const segments: TextNodeSegment[] = [];
    let node: Text | null;
    let nodeIndex = 0;

    while ((node = walker.nextNode() as Text | null)) {
      segments.push({
        key: makeSegmentKey(messageIndex, partIndex, segmentType, nodeIndex),
        text: node.nodeValue || "",
      });
      nodeIndex += 1;
    }

    return segments;
  }

  function getWriteMarkdownContent(part: { tool_name: string; input: string }): string | null {
    if (part.tool_name !== "Write") return null;
    try {
      const parsed = JSON.parse(part.input);
      if (
        typeof parsed.file_path === "string" &&
        parsed.file_path.endsWith(".md") &&
        typeof parsed.content === "string"
      ) {
        return parsed.content;
      }
    } catch {
      // not valid JSON
    }
    return null;
  }

  function getOrRenderPart(messageIndex: number, partIndex: number): any {
    const msg = options.messages.value[messageIndex];
    if (!msg) return undefined;
    const part = msg.content_parts[partIndex];
    if (!part) return undefined;

    const cacheKey = `${messageIndex}-${partIndex}`;
    const existing = renderedCache.value.get(cacheKey);

    const toolKey = makeToolKey(messageIndex, partIndex);
    let data: any;

    if (part.type === "text") {
      const rawText = msg.role === "user" ? cleanUserText(part.text) : part.text;
      if (existing && existing.kind === "text" && existing.rawText === rawText) {
        return existing;
      }
      const renderedHtml = renderMarkdown(rawText);
      data = {
        kind: "text",
        rawText,
        renderedHtml,
        textNodes: extractHtmlTextNodes(renderedHtml, messageIndex, partIndex),
        toolKey,
      };
    } else if (part.type === "tool_use") {
      const markdownContent = getWriteMarkdownContent(part);
      if (
        existing &&
        existing.kind === "tool_use" &&
        existing.summary === part.summary &&
        existing.input === part.input &&
        existing.markdownContent === markdownContent
      ) {
        return existing;
      }
      const renderedMarkdown = markdownContent ? renderMarkdown(markdownContent) : null;
      data = {
        kind: "tool_use",
        summary: part.summary,
        input: part.input,
        markdownContent,
        markdownPreview: renderedMarkdown
          ? {
              renderedHtml: renderedMarkdown,
              textNodes: extractHtmlTextNodes(
                renderedMarkdown,
                messageIndex,
                partIndex,
                "tool_use_markdown"
              ),
            }
          : undefined,
        toolKey,
      };
    } else if (part.type === "tool_result") {
      if (
        existing &&
        existing.kind === "tool_result" &&
        existing.summary === part.summary &&
        existing.content === part.content
      ) {
        return existing;
      }
      data = {
        kind: "tool_result",
        summary: part.summary,
        content: part.content,
        toolKey,
      };
    } else {
      if (existing && existing.kind === part.type) {
        return existing;
      }
      data = { kind: part.type, toolKey };
    }

    renderedCache.value.set(cacheKey, data);
    return data;
  }

  function collectSegmentMatches(
    matches: SearchMatch[],
    segmentMatches: Map<string, SearchMatch[]>,
    globalIndex: { value: number },
    query: string,
    searchOptionsValue: SearchOptions,
    text: string,
    descriptor: Omit<SearchMatch, "id" | "start" | "end">
  ): boolean {
    const ranges = findMatchRanges(text, query, searchOptionsValue);
    if (ranges.length === 0) return false;

    const segmentItems = ranges.map((range) => {
      const match: SearchMatch = {
        id: `search-match-${globalIndex.value}`,
        ...descriptor,
        start: range.start,
        end: range.end,
      };
      globalIndex.value += 1;
      matches.push(match);
      return match;
    });

    segmentMatches.set(descriptor.segmentKey, segmentItems);
    return true;
  }

  const trimmedSearchQuery = computed(() => searchQuery.value.trim());

  const searchState = computed<SearchState>(() => {
    const queryText = trimmedSearchQuery.value;
    if (!queryText) {
      return {
        matches: [],
        segmentMatches: new Map<string, SearchMatch[]>(),
        matchedToolKeys: new Set<string>(),
      };
    }

    const searchOptionsValue = searchOptions.value;
    const matches: SearchMatch[] = [];
    const segmentMatches = new Map<string, SearchMatch[]>();
    const matchedToolKeys = new Set<string>();
    const globalIndex = { value: 0 };

    options.messages.value.forEach((msg, messageIndex) => {
      msg.content_parts.forEach((part, partIndex) => {
        const toolKey = makeToolKey(messageIndex, partIndex);
        // Exclude tool outputs (command line results) from search matching completely per user Option 1
        if (part.type !== "text") return;

        if (msg.is_meta) return;
        if (msg.role === "user" && !options.filterUser.value) return;
        if (msg.role === "assistant" && !options.filterAssistant.value) return;
        collectSegmentMatches(matches, segmentMatches, globalIndex, queryText, searchOptionsValue, part.text, {
          segmentKey: makeSegmentKey(messageIndex, partIndex, "text", -1),
          messageIndex,
          partIndex,
          segmentType: "text",
          toolKey,
        });
      });
    });

    return {
      matches,
      segmentMatches,
      matchedToolKeys,
    };
  });

  const totalSearchMatches = computed(() => searchState.value.matches.length);
  const activeSearchMatch = computed(() => searchState.value.matches[currentSearchIndex.value] ?? null);

  function getSegmentMatches(segmentKey: string, text?: string, globalOffset = 0): SearchMatch[] {
    const matches = searchState.value.segmentMatches.get(segmentKey);
    if (matches && matches.length > 0) return matches;

    if (
      text &&
      trimmedSearchQuery.value &&
      (segmentKey.includes("-text-") || segmentKey.includes("-tool_use_markdown-"))
    ) {
      const parts = segmentKey.split("-");
      const m = parseInt(parts[1]);
      const p = parseInt(parts[2]);
      const type = segmentKey.includes("-text-") ? "text" : "tool_use_markdown";

      if (isNaN(m) || isNaN(p)) return [];

      const markerKey = makeSegmentKey(m, p, type, -1);
      const markerMatches = searchState.value.segmentMatches.get(markerKey);

      if (markerMatches && markerMatches.length > 0) {
        const query = trimmedSearchQuery.value;
        const searchOptionsValue = searchOptions.value;
        const localRanges = findMatchRanges(text, query, searchOptionsValue);

        return localRanges.map((range, i) => {
          const matchIndex = globalOffset + i;
          const realMatch = markerMatches[matchIndex];
          if (realMatch) {
            return {
              ...realMatch,
              segmentKey,
              start: range.start,
              end: range.end,
            };
          }
          return {
            id: `${segmentKey}-local-${i}`,
            segmentKey,
            messageIndex: m,
            partIndex: p,
            segmentType: type,
            start: range.start,
            end: range.end,
          };
        });
      }
    }
    return [];
  }

  function highlightPlainText(text: string, segmentKey: string, globalOffset = 0): string {
    const segmentItems = getSegmentMatches(segmentKey, text, globalOffset);
    if (!trimmedSearchQuery.value || segmentItems.length === 0) {
      return escapeHtml(text);
    }

    let html = "";
    let lastIndex = 0;

    for (const match of segmentItems) {
      if (match.start > lastIndex) {
        html += escapeHtml(text.slice(lastIndex, match.start));
      }

      const active = activeSearchMatch.value;
      const isActuallyActive = active !== null && active.id === match.id;

      const activeClass = isActuallyActive ? " search-highlight-active" : "";
      const dataAttr = ` data-search-match-id="${match.id}"`;
      html += `<mark class="search-highlight${activeClass}"${dataAttr}>${escapeHtml(
        text.slice(match.start, match.end)
      )}</mark>`;
      lastIndex = match.end;
    }

    if (lastIndex < text.length) {
      html += escapeHtml(text.slice(lastIndex));
    }

    return html;
  }

  function highlightRenderedHtml(renderedHtml: string, textNodes: TextNodeSegment[]): string {
    if (!trimmedSearchQuery.value) {
      return renderedHtml;
    }

    const template = document.createElement("template");
    template.innerHTML = renderedHtml;

    const walker = document.createTreeWalker(template.content, NodeFilter.SHOW_TEXT, null);
    const nodes: Text[] = [];
    let node: Text | null;

    while ((node = walker.nextNode() as Text | null)) {
      nodes.push(node);
    }

    let runningOffset = 0;
    nodes.forEach((textNode, index) => {
      const segment = textNodes[index];
      if (!segment) {
        return;
      }

      const fragmentTemplate = document.createElement("template");
      fragmentTemplate.innerHTML = highlightPlainText(segment.text, segment.key, runningOffset);
      const numMatches = findMatchRanges(segment.text, trimmedSearchQuery.value, searchOptions.value).length;
      runningOffset += numMatches;

      textNode.parentNode?.replaceChild(fragmentTemplate.content, textNode);
    });

    return template.innerHTML;
  }

  function getPartData(messageIndex: number, partIndex: number): any {
    return getOrRenderPart(messageIndex, partIndex);
  }

  function renderTextPart(messageIndex: number, partIndex: number): string {
    const partData = getPartData(messageIndex, partIndex);
    if (!partData || partData.kind !== "text") {
      return "";
    }
    return highlightRenderedHtml(partData.renderedHtml, partData.textNodes);
  }

  function renderToolUseSummary(messageIndex: number, partIndex: number, text: string): string {
    return highlightPlainText(text, makeSegmentKey(messageIndex, partIndex, "tool_use_summary"));
  }

  function renderToolUseInput(messageIndex: number, partIndex: number, text: string): string {
    return highlightPlainText(text, makeSegmentKey(messageIndex, partIndex, "tool_use_input"));
  }

  function renderToolUseMarkdown(messageIndex: number, partIndex: number): string {
    const partData = getPartData(messageIndex, partIndex);
    if (!partData || partData.kind !== "tool_use" || !partData.markdownPreview) {
      return "";
    }
    return highlightRenderedHtml(partData.markdownPreview.renderedHtml, partData.markdownPreview.textNodes);
  }

  function renderToolResultSummary(messageIndex: number, partIndex: number, text: string): string {
    return highlightPlainText(text, makeSegmentKey(messageIndex, partIndex, "tool_result_summary"));
  }

  function renderToolResultContent(messageIndex: number, partIndex: number, text: string): string {
    return highlightPlainText(text, makeSegmentKey(messageIndex, partIndex, "tool_result_content"));
  }

  function collapseSearchExpandedTools() {
    for (const key of searchExpandedKeys) {
      expandedTools[key] = false;
    }
    searchExpandedKeys.clear();
  }

  function syncSearchExpandedTools() {
    collapseSearchExpandedTools();
    if (!trimmedSearchQuery.value) return;

    for (const key of searchState.value.matchedToolKeys) {
      if (!expandedTools[key]) {
        expandedTools[key] = true;
        searchExpandedKeys.add(key);
      }
    }
  }

  async function revealCurrentSearchMatch() {
    const match = activeSearchMatch.value;
    const container = options.chatContainer.value;
    if (!match || !container) return;

    await options.ensureMessageVisible(match.messageIndex);
    await nextTick();

    if (match.toolKey && searchState.value.matchedToolKeys.has(match.toolKey) && !expandedTools[match.toolKey]) {
      expandedTools[match.toolKey] = true;
      searchExpandedKeys.add(match.toolKey);
      await nextTick();
    }

    const activeMark = container.querySelector(`[data-search-match-id="${match.id}"]`) as HTMLElement | null;
    if (activeMark) {
      activeMark.scrollIntoView({ behavior: "smooth", block: "center" });
      return;
    }

    const messageEl = (container.querySelector(`[data-msg-index~="${match.messageIndex}"]`) ||
                       container.querySelector(`[data-msg-index="${match.messageIndex}"]`)) as HTMLElement | null;
    if (messageEl) {
      messageEl.scrollIntoView({ behavior: "smooth", block: "center" });
      messageEl.classList.remove("message-flash");
      void messageEl.offsetWidth;
      messageEl.classList.add("message-flash");
      setTimeout(() => messageEl.classList.remove("message-flash"), 2200);
    }
  }

  async function focusNextSearchMatch() {
    if (totalSearchMatches.value === 0) return;
    currentSearchIndex.value = (currentSearchIndex.value + 1) % totalSearchMatches.value;
    await nextTick();
    await revealCurrentSearchMatch();
  }

  async function focusPrevSearchMatch() {
    if (totalSearchMatches.value === 0) return;
    currentSearchIndex.value =
      (currentSearchIndex.value - 1 + totalSearchMatches.value) % totalSearchMatches.value;
    await nextTick();
    await revealCurrentSearchMatch();
  }

  async function applySearchBehavior() {
    if (!trimmedSearchQuery.value) {
      currentSearchIndex.value = 0;
      collapseSearchExpandedTools();
      return;
    }

    currentSearchIndex.value = 0;
    syncSearchExpandedTools();
    await nextTick();
    await revealCurrentSearchMatch();
  }

  async function onSearchInput(query: string) {
    searchQuery.value = query;
    await applySearchBehavior();
  }

  async function onToggleCaseSensitive() {
    searchOptions.value = {
      ...searchOptions.value,
      caseSensitive: !searchOptions.value.caseSensitive,
    };
    await applySearchBehavior();
  }

  async function onToggleWholeWord() {
    searchOptions.value = {
      ...searchOptions.value,
      wholeWord: !searchOptions.value.wholeWord,
    };
    await applySearchBehavior();
  }

  function onCloseSearch() {
    searchQuery.value = "";
    currentSearchIndex.value = 0;
    collapseSearchExpandedTools();
    options.emitCloseSearch();
  }

  function focusSearch() {
    options.searchBarRef.value?.focusInput();
  }

  async function openSearchAt(query: string) {
    await options.initSession();

    if (!options.fullyLoaded.value) {
      pendingSearch.value = query;
      return;
    }

    searchQuery.value = query;
    options.searchBarRef.value?.setQuery(query);

    await nextTick();

    if (!query.trim()) {
      return;
    }

    // 定位不消费后端给的 first_match_message_index：搜索索引的 message_index 只数
    // 「含 text 的消息」，而消息数组下标含纯工具消息，两者系统性错位，拿它当下标必然
    // 跳到偏早的错误匹配。后端 first_match 的语义 ≡ 数组顺序第一个匹配，且两边内容
    // 口径一致（都只搜 text、都排除工具输出），因此取会话内搜索的首个匹配即等价。
    currentSearchIndex.value = 0;
    await revealCurrentSearchMatch();
  }

  watch(options.fullyLoaded, (loaded) => {
    if (!loaded || !pendingSearch.value) return;
    const pending = pendingSearch.value;
    pendingSearch.value = null;
    nextTick(() => openSearchAt(pending));
  });

  watch(
    options.showSearch,
    async (show) => {
      if (show) {
        await nextTick();
        options.searchBarRef.value?.focusInput();
        if (searchQuery.value.trim()) {
          syncSearchExpandedTools();
          await nextTick();
          await revealCurrentSearchMatch();
        }
      } else {
        searchQuery.value = "";
        currentSearchIndex.value = 0;
        collapseSearchExpandedTools();
      }
    }
  );

  watch(totalSearchMatches, (count) => {
    if (count === 0) {
      currentSearchIndex.value = 0;
      return;
    }

    if (currentSearchIndex.value >= count) {
      currentSearchIndex.value = count - 1;
    }
  });

  // 会话刷新/重载时调用：渲染缓存以 (messageIndex, partIndex) 为键，
  // 内容变化后序号不变会命中旧 HTML，必须整体失效
  function clearRenderedCache() {
    renderedCache.value = new Map();
  }

  return {
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
  };
}
