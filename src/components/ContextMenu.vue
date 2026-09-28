<script setup lang="ts">
import { ref, nextTick, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import type { ContextMenuItem } from "../types/session";

const props = defineProps<{
  visible: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
}>();

const emit = defineEmits<{ close: [] }>();

const menuRef = ref<HTMLElement | null>(null);
const adjustedX = ref(0);
const adjustedY = ref(0);
const positioned = ref(false);

watch(
  () => props.visible,
  async (val) => {
    if (!val) {
      positioned.value = false;
      return;
    }

    // Start at cursor position, then clamp after render
    positioned.value = false;
    adjustedX.value = props.x;
    adjustedY.value = props.y;
    await nextTick();
    clampToViewport();
    positioned.value = true;
  }
);

function clampToViewport() {
  const el = menuRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  let posX = props.x;
  let posY = props.y;

  if (posX + rect.width > window.innerWidth) {
    posX = window.innerWidth - rect.width - 8;
  }
  if (posY + rect.height > window.innerHeight) {
    posY = window.innerHeight - rect.height - 8;
  }
  if (posX < 0) posX = 8;
  if (posY < 0) posY = 8;

  adjustedX.value = posX;
  adjustedY.value = posY;
}

async function runItemAction(item: ContextMenuItem) {
  if (item.children?.length) return;
  if (item.closeOnClick !== false) {
    emit("close");
  }
  await item.action?.();
}
</script>

<template>
  <Transition name="fade-scale">
    <div v-if="visible" class="context-menu-overlay" @click="emit('close')" @contextmenu.prevent="emit('close')">
      <div
        ref="menuRef"
        class="context-menu menu-surface"
        :class="{ 'is-positioned': positioned }"
        :style="{ left: adjustedX + 'px', top: adjustedY + 'px' }"
        role="menu"
        @click.stop
      >
        <template v-for="(item, idx) in items" :key="idx">
          <div v-if="item.separator" class="menu-divider"></div>
          <div v-else class="menu-anchor">
            <div
              class="menu-item"
              :class="{ danger: item.danger, 'has-submenu': item.children?.length }"
              role="menuitem"
              @click.stop="runItemAction(item)"
            >
              <SvgIcon v-if="item.icon" :name="item.icon" :size="14" class="menu-icon" />
              <span>{{ item.label }}</span>
              <SvgIcon v-if="item.children?.length" name="chevron-right" :size="13" class="menu-chevron" />
            </div>
            <div v-if="item.children?.length" class="submenu menu-submenu menu-submenu--right" role="menu">
              <template v-for="(child, childIdx) in item.children" :key="childIdx">
                <div v-if="child.separator" class="menu-divider"></div>
                <div v-else class="menu-anchor">
                  <div
                    class="menu-item"
                    :class="{ danger: child.danger, 'has-submenu': child.children?.length }"
                    role="menuitem"
                    @click.stop="runItemAction(child)"
                  >
                    <SvgIcon v-if="child.icon" :name="child.icon" :size="14" class="menu-icon" />
                    <span>{{ child.label }}</span>
                    <SvgIcon v-if="child.children?.length" name="chevron-right" :size="13" class="menu-chevron" />
                  </div>
                  <div v-if="child.children?.length" class="submenu menu-submenu menu-submenu--right" role="menu">
                    <template v-for="(grandchild, gcIdx) in child.children" :key="gcIdx">
                      <div v-if="grandchild.separator" class="menu-divider"></div>
                      <div
                        v-else
                        class="menu-item"
                        :class="{ danger: grandchild.danger }"
                        role="menuitem"
                        @click.stop="runItemAction(grandchild)"
                      >
                        <SvgIcon v-if="grandchild.icon" :name="grandchild.icon" :size="14" class="menu-icon" />
                        <span>{{ grandchild.label }}</span>
                      </div>
                    </template>
                  </div>
                </div>
              </template>
            </div>
          </div>
        </template>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.context-menu-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
}
.context-menu {
  position: fixed;
  min-width: 180px;
  z-index: calc(var(--z-modal) + 1);
  opacity: 0;
  visibility: hidden;
  transform: scale(0.98);
  transform-origin: top left;
}
.context-menu.is-positioned {
  opacity: 1;
  visibility: visible;
  transform: scale(1);
  transition: opacity 120ms ease, transform 140ms ease;
}
.submenu {
  min-width: 170px;
  opacity: 0;
  visibility: hidden;
  transform: translateX(-4px) scale(0.98);
  transition: opacity 120ms ease, transform 140ms ease, visibility 120ms ease;
}
.menu-anchor:hover > .submenu {
  opacity: 1;
  visibility: visible;
  transform: translateX(0) scale(1);
}
.menu-item.danger {
  color: var(--color-danger);
}
.menu-item.danger:hover {
  background: rgba(220, 38, 38, 0.08);
}
.menu-icon {
  flex-shrink: 0;
  color: var(--color-text-secondary);
}
.menu-item.danger .menu-icon {
  color: var(--color-danger);
}
</style>
