import { computed, nextTick, ref, watch, type ComputedRef, type Ref } from "vue";
import type { ChatMessage } from "../types/session";

export interface IndexedChatMessage {
  msg: ChatMessage;
  originalIndex: number;
}

const PAGE_SIZE = 150;

interface UseChatScrollOptions {
  chatContainer: Ref<HTMLElement | null>;
  messages: Ref<ChatMessage[]>;
  filteredMessages: ComputedRef<IndexedChatMessage[]>;
  fullyLoaded: ComputedRef<boolean>;
  autoFollow?: ComputedRef<boolean>;
  onActiveMessageChange: (index: number | null) => void;
}

function findFilteredIndex(items: readonly IndexedChatMessage[], targetOriginalIndex: number): number {
  let lo = 0;
  let hi = items.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const midIdx = items[mid].originalIndex;
    if (midIdx === targetOriginalIndex) return mid;
    if (midIdx < targetOriginalIndex) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

export function useChatScroll(options: UseChatScrollOptions) {
  const windowStart = ref(0);
  const windowEnd = ref(PAGE_SIZE);
  const sentinelUp = ref<HTMLElement | null>(null);
  const sentinelDown = ref<HTMLElement | null>(null);
  const isJumping = ref(false);
  const isAtBottom = ref(false);

  function checkIfAtBottom(): boolean {
    const el = options.chatContainer.value;
    if (!el) return false;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    return distanceFromBottom <= 60;
  }

  let observer: IntersectionObserver | null = null;
  let observerEnabled = false;
  let loadingMore = false;
  let savedAnchor: { el: HTMLElement; offset: number } | null = null;

  const displayMessages = computed(() => {
    const start = Math.max(0, windowStart.value);
    const end = Math.min(windowEnd.value, options.filteredMessages.value.length);
    return options.filteredMessages.value.slice(start, end);
  });

  const hasMoreDown = computed(() => windowEnd.value < options.filteredMessages.value.length);
  const hasMoreUp = computed(() => windowStart.value > 0);

  function disconnectObserver() {
    observer?.disconnect();
    observer = null;
  }

  function disableObserver() {
    observerEnabled = false;
    disconnectObserver();
  }

  /** 加载完成后启用观察器：哨兵进入视口附近即自动加载，避免进页面就露"加载更多"按钮 */
  function enableObserver() {
    observerEnabled = true;
    setupObserver();
  }

  function setupObserver() {
    disconnectObserver();
    if (!options.chatContainer.value) return;

    observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting && !loadingMore && !isJumping.value) {
            if (entry.target === sentinelDown.value && hasMoreDown.value) {
              loadMoreDown();
            } else if (entry.target === sentinelUp.value && hasMoreUp.value) {
              loadMoreUp();
            }
          }
        });
      },
      {
        root: options.chatContainer.value,
        threshold: 0,
        rootMargin: "250px 0px",
      }
    );

    if (sentinelUp.value) observer.observe(sentinelUp.value);
    if (sentinelDown.value) observer.observe(sentinelDown.value);
  }

  watch([sentinelUp, sentinelDown], () => {
    if (observerEnabled) {
      nextTick(() => setupObserver());
    }
  });

  function loadMoreDown() {
    if (loadingMore) return;
    loadingMore = true;
    windowEnd.value = Math.min(windowEnd.value + PAGE_SIZE, options.filteredMessages.value.length);

    nextTick(() => {
      setTimeout(() => {
        loadingMore = false;
      }, 200);
    });
  }

  function loadMoreUp() {
    if (loadingMore) return;
    loadingMore = true;
    const container = options.chatContainer.value;
    const oldScrollHeight = container?.scrollHeight ?? 0;
    const oldScrollTop = container?.scrollTop ?? 0;

    windowStart.value = Math.max(0, windowStart.value - PAGE_SIZE);

    nextTick(() => {
      if (container) {
        const newScrollHeight = container.scrollHeight;
        const heightDiff = newScrollHeight - oldScrollHeight;
        if (heightDiff > 0) {
          container.scrollTop = oldScrollTop + heightDiff;
        }
      }
      setTimeout(() => {
        loadingMore = false;
      }, 200);
    });
  }

  function saveScrollAnchor() {
    const container = options.chatContainer.value;
    if (!container) return;
    const nodes = Array.from(container.querySelectorAll<HTMLElement>("[data-msg-index]"));
    const containerRect = container.getBoundingClientRect();
    for (const node of nodes) {
      const nodeTop = node.getBoundingClientRect().top - containerRect.top;
      if (nodeTop >= 0) {
        savedAnchor = { el: node, offset: nodeTop };
        return;
      }
    }
    if (nodes.length > 0) {
      const last = nodes[nodes.length - 1];
      savedAnchor = { el: last, offset: last.getBoundingClientRect().top - containerRect.top };
    }
  }

  function restoreScrollAnchor() {
    const container = options.chatContainer.value;
    if (!container || !savedAnchor) return;
    const { el, offset } = savedAnchor;
    const containerRect = container.getBoundingClientRect();
    const drift = el.getBoundingClientRect().top - containerRect.top - offset;
    if (Math.abs(drift) > 1) {
      container.scrollTop += drift;
    }
    savedAnchor = null;
  }

  function updateActiveMessageFromScroll() {
    const container = options.chatContainer.value;
    if (!container) {
      options.onActiveMessageChange(null);
      return;
    }
    const nodes = Array.from(container.querySelectorAll<HTMLElement>("[data-msg-index]"));
    if (nodes.length === 0) {
      options.onActiveMessageChange(null);
      return;
    }

    const containerTop = container.getBoundingClientRect().top;
    const targetTop = containerTop + 96;
    // data-msg-index 可能是空格分隔的多个序号（assistant 回合合并多条消息），取首个
    const firstIndexOf = (el: HTMLElement): number =>
      Number((el.dataset.msgIndex ?? "").split(" ")[0]);
    let activeIndex = firstIndexOf(nodes[0]);
    if (!Number.isFinite(activeIndex)) activeIndex = 0;
    for (const node of nodes) {
      if (node.getBoundingClientRect().top <= targetTop) {
        const idx = firstIndexOf(node);
        if (Number.isFinite(idx)) activeIndex = idx;
      } else {
        break;
      }
    }
    options.onActiveMessageChange(Number.isFinite(activeIndex) ? activeIndex : null);
  }

  function onChatScroll() {
    if (!observerEnabled) {
      observerEnabled = true;
      setupObserver();
    }
    isAtBottom.value = checkIfAtBottom();
    updateActiveMessageFromScroll();
  }

  async function ensureMessageVisible(index: number) {
    if (index < 0 || index >= options.messages.value.length) return;
    const filteredIndex = findFilteredIndex(options.filteredMessages.value, index);
    if (filteredIndex === -1) return;

    if (filteredIndex < windowStart.value || filteredIndex >= windowEnd.value) {
      isJumping.value = true;
      windowStart.value = Math.max(0, Math.floor(filteredIndex - PAGE_SIZE / 2));
      windowEnd.value = Math.min(
        options.filteredMessages.value.length,
        Math.floor(filteredIndex + PAGE_SIZE / 2)
      );
      await nextTick();
      setTimeout(() => {
        isJumping.value = false;
      }, 100);
    }
  }

  function scrollToMessage(index: number) {
    if (!options.fullyLoaded.value) {
      return false;
    }
    if (index < 0 || index >= options.messages.value.length) {
      return true;
    }

    const attemptScroll = (retries = 8) => {
      const container = options.chatContainer.value;
      if (!container) {
        if (retries > 0) setTimeout(() => attemptScroll(retries - 1), 40);
        return;
      }
      // ~= 匹配空格分隔列表（assistant 回合的 data-msg-index 含多个序号）
      const el = container.querySelector(
        `[data-msg-index~="${index}"]`
      ) as HTMLElement | null;
      if (el) {
        el.scrollIntoView({ behavior: "auto", block: "start" });
        el.classList.remove("message-flash");
        void el.offsetWidth;
        el.classList.add("message-flash");
        window.setTimeout(() => el.classList.remove("message-flash"), 2200);
        options.onActiveMessageChange(index);
      } else if (retries > 0) {
        setTimeout(() => attemptScroll(retries - 1), 50);
      }
    };

    ensureMessageVisible(index).then(() => {
      nextTick(() => {
        attemptScroll();
      });
    });
    return true;
  }

  function scrollToAbsoluteEnd() {
    if (isJumping.value) return;
    isJumping.value = true;
    windowEnd.value = options.filteredMessages.value.length;
    windowStart.value = Math.max(0, options.filteredMessages.value.length - PAGE_SIZE);

    nextTick(() => {
      requestAnimationFrame(() => {
        const el = options.chatContainer.value;
        if (el) {
          el.scrollTo({ top: el.scrollHeight, behavior: "auto" });
          setTimeout(() => {
            el.scrollTo({ top: el.scrollHeight, behavior: "auto" });
            isJumping.value = false;
            isAtBottom.value = true;
          }, 100);
        } else {
          isJumping.value = false;
        }
      });
    });
  }

  function scrollToAbsoluteStart() {
    if (isJumping.value) return;
    isJumping.value = true;
    windowStart.value = 0;
    windowEnd.value = PAGE_SIZE;

    nextTick(() => {
      requestAnimationFrame(() => {
        const el = options.chatContainer.value;
        if (el) {
          el.scrollTo({ top: 0, behavior: "auto" });
          setTimeout(() => {
            el.scrollTo({ top: 0, behavior: "auto" });
            isJumping.value = false;
            isAtBottom.value = checkIfAtBottom();
          }, 100);
        } else {
          isJumping.value = false;
        }
      });
    });
  }

  watch(
    () => options.filteredMessages.value.length,
    (newLen) => {
      const shouldFollow = isAtBottom.value || (options.autoFollow?.value ?? false);
      if (shouldFollow) {
        windowEnd.value = newLen;
        windowStart.value = Math.max(0, newLen - PAGE_SIZE);
      } else if (windowEnd.value > newLen) {
        windowEnd.value = newLen;
        windowStart.value = Math.max(0, newLen - PAGE_SIZE);
      } else if (windowStart.value >= newLen) {
        windowStart.value = Math.max(0, newLen - PAGE_SIZE);
        windowEnd.value = newLen;
      } else {
        const currentSize = windowEnd.value - windowStart.value;
        if (currentSize <= 0 && newLen > 0) {
          windowEnd.value = Math.min(newLen, windowStart.value + PAGE_SIZE);
        }
      }
    }
  );

  function refreshObserverIfEnabled() {
    if (observerEnabled) {
      nextTick(() => setupObserver());
    }
  }

  return {
    PAGE_SIZE,
    windowStart,
    windowEnd,
    displayMessages,
    hasMoreUp,
    hasMoreDown,
    sentinelUp,
    sentinelDown,
    isJumping,
    isAtBottom,
    checkIfAtBottom,
    disableObserver,
    enableObserver,
    disconnectObserver,
    setupObserver,
    onChatScroll,
    updateActiveMessageFromScroll,
    saveScrollAnchor,
    restoreScrollAnchor,
    ensureMessageVisible,
    scrollToMessage,
    scrollToAbsoluteEnd,
    scrollToAbsoluteStart,
    loadMoreUp,
    loadMoreDown,
    refreshObserverIfEnabled,
  };
}
