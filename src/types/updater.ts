export interface UpdaterSettings {
  autoCheck: boolean;
  ignoredUpdateVersion: string | null;
}

export interface UpdaterState {
  currentVersion: string;
  settings: UpdaterSettings;
  hasToken: boolean;
}


export interface UpdaterSettingsInput {
  autoCheck: boolean;
}

export interface TauriUpdateInfo {
  version: string;
  notes: string | null;
  pubDate: string | null;
  releasePageUrl: string | null;
}

export interface TauriDownloadProgressEvent {
  chunkLen: number;
  total: number | null;
}
