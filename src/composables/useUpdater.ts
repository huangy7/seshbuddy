import { computed, ref } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import { listen } from "@tauri-apps/api/event";
import type {
  TauriDownloadProgressEvent,
  TauriUpdateInfo,
  UpdaterSettings,
  UpdaterSettingsInput,
  UpdaterState,
} from "../types/updater";

const state = ref<UpdaterState | null>(null);
const loadingState = ref(false);
const savingSettings = ref(false);
const checking = ref(false);
const error = ref("");

const tauriUpdateInfo = ref<TauriUpdateInfo | null>(null);
const installingTauriUpdate = ref(false);
const tauriDownloadProgress = ref<{ downloaded: number; total: number | null }>({ downloaded: 0, total: null });

let loadPromise: Promise<UpdaterState> | null = null;
let autoCheckAttempted = false;
let updateEventListenerPromise: Promise<void> | null = null;

const LOAD_STATE_TIMEOUT_MS = 500;

function defaultUpdaterState(): UpdaterState {
  return {
    currentVersion: "",
    settings: defaultSettings(),
    hasToken: false,
  };
}

function defaultSettings(): UpdaterSettings {
  return {
    autoCheck: true,
    ignoredUpdateVersion: null,
  };
}

async function loadState(force = false, skipTimeout = false): Promise<UpdaterState> {
  if (!force && state.value) {
    return state.value;
  }

  if (!force && loadPromise) {
    return loadPromise;
  }

  loadingState.value = true;

  const doLoad = invokeApp<UpdaterState>("get_updater_state")
    .then((next) => {
      state.value = next;
      error.value = "";
      return next;
    })
    .catch((err) => {
      error.value = renderAppError(err);
      throw err;
    });

  const task = (
    skipTimeout
      ? doLoad
      : Promise.race([
          doLoad,
          new Promise<UpdaterState>((resolve) =>
            setTimeout(() => resolve(state.value ?? defaultUpdaterState()), LOAD_STATE_TIMEOUT_MS)
          ),
        ])
  ).finally(() => {
    loadingState.value = false;
    if (loadPromise === task) {
      loadPromise = null;
    }
  });

  loadPromise = task;
  return task;
}

async function saveSettings(input: UpdaterSettingsInput): Promise<UpdaterState> {
  savingSettings.value = true;
  try {
    const next = await invokeApp<UpdaterState>("set_updater_settings", { input });
    state.value = next;
    error.value = "";
    return next;
  } catch (err) {
    error.value = renderAppError(err);
    throw err;
  } finally {
    savingSettings.value = false;
  }
}

function ensureUpdateEventListeners(): Promise<void> {
  if (updateEventListenerPromise) return updateEventListenerPromise;

  updateEventListenerPromise = Promise.all([
    listen<TauriUpdateInfo>("update-available", (event) => {
      tauriUpdateInfo.value = event.payload;
      error.value = "";
    }),
    listen("update-cleared", () => {
      tauriUpdateInfo.value = null;
    }),
  ]).then(() => undefined);

  return updateEventListenerPromise;
}

async function maybeAutoCheck(): Promise<void> {
  if (autoCheckAttempted) {
    return;
  }
  try {
    await ensureUpdateEventListeners();
    const current = await loadState(false, true);
    autoCheckAttempted = true;
    if (!current.settings.autoCheck) {
      return;
    }
    await invokeApp<void>("check_tauri_update_bg");
  } catch (err) {
    console.error("[update-check] auto check failed:", err);
  }
}

async function checkTauriUpdate(): Promise<TauriUpdateInfo | null> {
  checking.value = true;
  try {
    await ensureUpdateEventListeners();
    const result = await invokeApp<TauriUpdateInfo | null>("check_tauri_update");
    tauriUpdateInfo.value = result;
    error.value = "";
    return result;
  } catch (err) {
    error.value = renderAppError(err);
    throw err;
  } finally {
    checking.value = false;
  }
}

async function installTauriUpdate(): Promise<void> {
  installingTauriUpdate.value = true;
  tauriDownloadProgress.value = { downloaded: 0, total: null };

  const unlisten = await listen<TauriDownloadProgressEvent>("tauri-updater-progress", (event) => {
    const { chunkLen, total } = event.payload;
    tauriDownloadProgress.value = {
      downloaded: tauriDownloadProgress.value.downloaded + chunkLen,
      total: total ?? tauriDownloadProgress.value.total,
    };
  });

  try {
    await invokeApp<void>("install_tauri_update");
  } catch (err) {
    error.value = renderAppError(err);
    throw err;
  } finally {
    unlisten();
    installingTauriUpdate.value = false;
  }
}

async function ignoreUpdateVersion(version: string): Promise<UpdaterState> {
  try {
    const next = await invokeApp<UpdaterState>("ignore_update_version", { version });
    state.value = next;
    tauriUpdateInfo.value = null;
    error.value = "";
    return next;
  } catch (err) {
    error.value = renderAppError(err);
    throw err;
  }
}

export function useUpdater() {
  return {
    state,
    settings: computed(() => state.value?.settings ?? defaultSettings()),
    currentVersion: computed(() => state.value?.currentVersion ?? ""),
    hasAvailableUpdate: computed(() => !!tauriUpdateInfo.value),
    loadingState,
    savingSettings,
    checking,
    error,
    loadState,
    ensureUpdateEventListeners,
    saveSettings,
    maybeAutoCheck,
    tauriUpdateInfo,
    tauriDownloadProgress,
    installingTauriUpdate,
    checkTauriUpdate,
    installTauriUpdate,
    ignoreUpdateVersion,
  };
}
