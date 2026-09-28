import { invokeApp } from "../utils/invokeApp";
import type { Locale } from "../i18n/types";
import type { CliId } from "../types/cli";

export type StartupMode = "ready" | "onboarding" | "status-error";
export type FirstRunSelection = { locale: Locale; cliIds: CliId[] };

export async function loadStartupMode(
  readStatus: () => Promise<boolean> = () => invokeApp<boolean>("get_onboarding_status"),
): Promise<StartupMode> {
  try {
    return await readStatus() ? "onboarding" : "ready";
  } catch (error) {
    console.error("get_onboarding_status failed:", error);
    return "status-error";
  }
}

export async function completeFirstRun(
  selection: FirstRunSelection,
  actions: {
    saveLocale: (locale: Locale) => Promise<boolean>;
    saveFilter: (cliIds: CliId[]) => void;
    markComplete: () => Promise<void>;
  },
): Promise<void> {
  if (!await actions.saveLocale(selection.locale)) {
    throw new Error("locale-save-failed");
  }
  actions.saveFilter(selection.cliIds);
  await actions.markComplete();
}

export async function runStartup(
  mode: StartupMode,
  actions: { detectClis: () => Promise<void>; startSessions: () => Promise<void> },
): Promise<void> {
  if (mode === "onboarding") await actions.detectClis();
  else if (mode === "ready") await actions.startSessions();
}
