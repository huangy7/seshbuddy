import { ref, type Ref } from "vue";
import { invokeApp } from "../utils/invokeApp";
import type { SessionIdentity } from "../types/session";

/** 后端 favorites 条目（serde snake_case），与 SessionIdentity 的 cliId/filePath 一一对应。 */
export interface FavoriteEntry {
  cli_id: string;
  path: string;
}

function toEntry(identity: SessionIdentity): FavoriteEntry {
  return { cli_id: identity.cliId, path: identity.filePath };
}

function matches(entry: FavoriteEntry, identity: SessionIdentity): boolean {
  return entry.cli_id === identity.cliId && entry.path === identity.filePath;
}

// 单例：全应用共享同一份星标列表（SessionTree 行内图标、右键菜单、SessionHeader 按钮）。
const favorites = ref<FavoriteEntry[]>([]);

export function useFavorites(): {
  favorites: Readonly<Ref<FavoriteEntry[]>>;
  loadFavorites: () => Promise<void>;
  isFavorite: (identity: SessionIdentity) => boolean;
  toggleFavorite: (identity: SessionIdentity) => Promise<void>;
} {
  async function loadFavorites(): Promise<void> {
    favorites.value = await invokeApp<FavoriteEntry[]>("read_favorite_entries");
  }

  function isFavorite(identity: SessionIdentity): boolean {
    return favorites.value.some((entry) => matches(entry, identity));
  }

  async function toggleFavorite(identity: SessionIdentity): Promise<void> {
    if (isFavorite(identity)) {
      favorites.value = favorites.value.filter((entry) => !matches(entry, identity));
    } else {
      favorites.value = [...favorites.value, toEntry(identity)];
    }
    // 后端 write 语义为全量覆盖，toggle 后写回完整列表。
    await invokeApp("write_favorite_entries", { entries: favorites.value });
  }

  return { favorites, loadFavorites, isFavorite, toggleFavorite };
}
