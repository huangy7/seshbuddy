import type { Directive } from "vue";
import { renderMermaidInElement, cleanupMermaidInElement } from "../utils/mermaid";

export const vMermaid: Directive<HTMLElement> = {
  mounted(el) {
    void renderMermaidInElement(el);
  },
  updated(el) {
    void renderMermaidInElement(el);
  },
  unmounted(el) {
    cleanupMermaidInElement(el);
  },
};
