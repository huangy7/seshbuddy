<script setup lang="ts">
import { ref, computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";

const props = defineProps<{
  mediaType?: string;
  data?: string;
  path?: string;
}>();

defineEmits<{
  preview: [url: string];
  download: [url: string];
}>();

const hasError = ref(false);

function onError() {
  hasError.value = true;
}

const imageUrl = computed(() => {
  if (props.mediaType && props.data) {
    return `data:${props.mediaType};base64,${props.data}`;
  }
  if (props.path) {
    return convertFileSrc(props.path);
  }
  return "";
});
</script>

<template>
  <div v-if="imageUrl && !hasError" class="image-block">
    <div class="image-container">
      <img :src="imageUrl" class="chat-image" @error="onError" />
      <div class="image-overlay">
        <button class="image-action-btn" :title="t('chat.svgCard.preview')" @click="$emit('preview', imageUrl)">
          <SvgIcon name="zoom-in" :size="16" />
        </button>
        <button class="image-action-btn" :title="t('chat.imagePart.downloadImage')" @click="$emit('download', imageUrl)">
          <SvgIcon name="download" :size="16" />
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.image-block {
  margin: var(--space-1) 0;
}
.chat-image {
  max-width: 100%;
  max-height: 400px;
  border-radius: var(--radius-md);
  object-fit: contain;
}
.image-container {
  position: relative;
  display: inline-block;
  max-width: 100%;
  border-radius: var(--radius-md);
  overflow: hidden;
}
.image-overlay {
  position: absolute;
  top: var(--space-2);
  right: var(--space-2);
  display: flex;
  gap: var(--space-2);
  opacity: 0;
  transition: opacity var(--transition-fast);
}
.image-container:hover .image-overlay {
  opacity: 1;
}
.image-action-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  border: 1px solid var(--color-border);
  background-color: var(--color-bg);
  color: var(--color-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-fast);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}
.image-action-btn:hover {
  background-color: var(--color-bg-hover);
  color: var(--color-primary);
  transform: scale(1.05);
}
</style>
