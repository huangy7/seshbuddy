<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { t } from "../i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ProfilePicker from "../components/assistant/ProfilePicker.vue";
import ChatMessageList from "../components/assistant/ChatMessageList.vue";
import ChatInput from "../components/assistant/ChatInput.vue";
import ConversationHistory from "../components/assistant/ConversationHistory.vue";
import { useAssistantChat } from "../composables/useAssistantChat";
import CacheBackfillBar from "../components/assistant/CacheBackfillBar.vue";

// 即开即聊：profile 是标题栏下拉 chip，默认记住上次选择（ProfilePicker 内处理）
const profile = ref("");
const selectedModel = ref("");
const engineAvailable = ref(true);
const historyVisible = ref(false);
const chatMessageListRef = ref<InstanceType<typeof ChatMessageList> | null>(null);
const chatInputRef = ref<InstanceType<typeof ChatInput> | null>(null);
const { messages, running, activity, conversationId, conversationUsage, send, cancel, loadConversation, newConversation, dispose } = useAssistantChat();

// 窗口置顶：选择持久化，开窗自动恢复
const TOP_KEY = "assistant.alwaysOnTop";
const alwaysOnTop = ref(localStorage.getItem(TOP_KEY) === "1");

async function applyOnTop(onTop: boolean) {
  try {
    await invokeApp("assistant_set_always_on_top", { onTop });
  } catch (err) {
    console.error("设置置顶失败:", err);
  }
}

function toggleOnTop() {
  alwaysOnTop.value = !alwaysOnTop.value;
  localStorage.setItem(TOP_KEY, alwaysOnTop.value ? "1" : "0");
  applyOnTop(alwaysOnTop.value);
}

let unlistenDashboardPrompt: UnlistenFn | null = null;

// ProfilePicker 异步恢复上次选择，等它就绪再发，避免空 profile 打到后端
async function waitForProfile(timeoutMs = 3000): Promise<void> {
  if (profile.value) return;
  await new Promise<void>((resolve) => {
    const timer = setTimeout(() => {
      stop();
      resolve();
    }, timeoutMs);
    const stop = watch(profile, (p) => {
      if (p) {
        clearTimeout(timer);
        stop();
        resolve();
      }
    });
  });
}

interface PendingPromptPayload {
  text: string;
  autoSend?: boolean;
}

// Dashboard「问 SeshBuddy」提交或外部预填：取走后端暂存的 pending prompt（take 语义，恰好消费一次）
async function consumeDashboardPrompt() {
  try {
    const item = await invokeApp<PendingPromptPayload | string | null>("assistant_take_pending_prompt");
    if (!item) return;

    const text = typeof item === "string" ? item : item.text;
    const autoSend = typeof item === "string" ? true : (item.autoSend ?? true);

    if (text && text.trim()) {
      if (autoSend) {
        await waitForProfile();
        send(text, profile.value, selectedModel.value || undefined);
        chatMessageListRef.value?.scrollToBottom?.(true);
      } else {
        await nextTick();
        chatInputRef.value?.setDraft(text);
      }
    }
  } catch (err) {
    console.error("读取助手待发送消息失败:", err);
  }
}

onUnmounted(() => {
  dispose();
  unlistenDashboardPrompt?.();
  unlistenDashboardPrompt = null;
});

onMounted(async () => {
  if (alwaysOnTop.value) applyOnTop(true);
  try {
    const status = await invokeApp<{ available: boolean }>("assistant_engine_status");
    engineAvailable.value = status.available;
  } catch (err) {
    console.error("检查助手引擎失败:", err);
    engineAvailable.value = false;
  }
  // 窗口由 assistant_open_with_prompt 新建：mount 后取走暂存的 prompt
  await consumeDashboardPrompt();
  // 窗口已存在时由事件通知再取
  unlistenDashboardPrompt = await listen("assistant-dashboard-prompt", () => {
    void consumeDashboardPrompt();
  });
});

function onSelectConversation(id: string, convProfile: string) {
  loadConversation(id);
  if (convProfile) profile.value = convProfile;
  historyVisible.value = false;
  chatMessageListRef.value?.scrollToBottom?.(true);
}

function onDeleteConversation(id: string) {
  // 删的是当前对话：回到新对话状态
  if (conversationId.value === id) newConversation();
}

function startNewConversation() {
  // 状态已按对话隔离：生成中也可以开新对话，后台轮次不受干扰
  newConversation();
  chatMessageListRef.value?.scrollToBottom?.(true);
}

function onPickSuggestion(text: string) {
  chatInputRef.value?.setDraft(text);
}

function onSend(text: string) {
  send(text, profile.value, selectedModel.value || undefined);
  chatMessageListRef.value?.scrollToBottom?.(true);
}
</script>

<template>
  <div class="assistant-root">
    <header class="assistant-header">
      <button class="icon-btn" :title="t('assistant.history.title')" @click="historyVisible = !historyVisible">☰</button>
      <div class="header-spacer" />
      <ProfilePicker v-model="profile" :locked="conversationId !== null" />
      <button class="icon-btn" :title="t('assistant.app.newConversation')" @click="startNewConversation">{{ t("assistant.app.newConversationGlyph") }}</button>
      <button
        class="icon-btn pin-btn"
        :class="{ active: alwaysOnTop }"
        :title="alwaysOnTop ? t('assistant.app.unpinWindow') : t('assistant.app.pinWindow')"
        @click="toggleOnTop"
      >📌</button>
    </header>
    <div v-if="!engineAvailable" class="engine-banner">
      {{ t("assistant.app.engineMissing") }}
    </div>
    <CacheBackfillBar />

    <ConversationHistory
      v-if="historyVisible"
      :current-id="conversationId"
      @select="onSelectConversation"
      @close="historyVisible = false"
      @delete="onDeleteConversation"
    />
    <ChatMessageList
      ref="chatMessageListRef"
      :messages="messages"
      :activity="activity"
      :profile="profile"
      @pick="onPickSuggestion"
    />
    <ChatInput
      ref="chatInputRef"
      v-model:model="selectedModel"
      :disabled="!engineAvailable || running || !profile"
      :running="running"
      :profile="profile"
      :usage="conversationUsage"
      @send="onSend"
      @cancel="cancel()"
    />
  </div>
</template>

<style scoped>
.assistant-root {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--color-bg);
  color: var(--color-text);
}
.assistant-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.header-spacer {
  flex: 1;
}
.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 15px;
  cursor: pointer;
}
.icon-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.pin-btn {
  font-size: 12px;
  opacity: 0.55;
}
.pin-btn.active {
  opacity: 1;
  background: var(--color-primary-light);
}
.engine-banner {
  padding: var(--space-2) var(--space-3);
  background: var(--color-danger-bg, rgba(220, 60, 60, 0.12));
  color: var(--color-danger);
  font-size: var(--text-sm);
  flex-shrink: 0;
}
</style>
