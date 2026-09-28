<script setup lang="ts">
import { ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { copyToClipboard, copyPromiseToClipboard } from "../utils/clipboard";
import { t } from "../i18n";

const props = withDefaults(
  defineProps<{
    text: string;
    size?: "sm" | "md";
    /**
     * 异步取文本（如 tool_result 懒加载全文）。
     * 提供时优先于 text：用 copyPromiseToClipboard 同步发起写入、数据异步到达，
     * 规避旧版 WKWebView await 后 transient activation 失效导致的剪贴板拒绝。
     */
    textPromise?: () => Promise<string>;
  }>(),
  {
    size: "sm",
    textPromise: undefined,
  }
);

const copied = ref(false);

async function doCopy() {
  const ok = props.textPromise
    ? await copyPromiseToClipboard(props.textPromise())
    : await copyToClipboard(props.text);
  if (ok) {
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1800);
  }
}
</script>

<template>
  <button
    class="copy-btn"
    :class="[`size-${size}`, { copied }]"
    :title="copied ? t('common.copyButton.copied') : t('common.copyButton.copy')"
    @click.stop="doCopy()"
  >
    <SvgIcon :name="copied ? 'check' : 'copy'" :size="size === 'sm' ? 13 : 14" />
  </button>
</template>

<style scoped>
.copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  transition: all var(--transition-fast);
  cursor: pointer;
}
.size-sm {
  width: 24px;
  height: 24px;
}
.size-md {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-md);
}
.copy-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.copy-btn.copied {
  color: var(--color-success);
  background: var(--color-bg-hover);
}
</style>
