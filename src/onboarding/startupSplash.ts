import type { StartupMode } from "./startup";

type StartupSplashState = {
  hasShownOnboarding: boolean;
  startupMode: StartupMode;
  onboardingDetectionDone: boolean;
  initialScanSettled: boolean;
};

export function shouldDismissStartupSplash(state: StartupSplashState): boolean {
  if (state.hasShownOnboarding && state.startupMode !== "ready") {
    return state.onboardingDetectionDone;
  }
  return state.initialScanSettled;
}
