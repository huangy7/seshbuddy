import { describe, expect, it } from "vitest";
import { shouldDismissStartupSplash } from "./startupSplash";

const base = {
  hasShownOnboarding: true,
  startupMode: "onboarding" as const,
  onboardingDetectionDone: false,
  initialScanSettled: false,
};

describe("startup splash", () => {
  it("shows the guide only after CLI detection", () => {
    expect(shouldDismissStartupSplash(base)).toBe(false);
    expect(shouldDismissStartupSplash({ ...base, onboardingDetectionDone: true })).toBe(true);
  });

  it("stays over the first scan after onboarding until the stream settles", () => {
    expect(shouldDismissStartupSplash({ ...base, startupMode: "ready" })).toBe(false);
    expect(shouldDismissStartupSplash({ ...base, startupMode: "ready", initialScanSettled: true })).toBe(true);
  });

  it("waits for the stream on an existing installation too", () => {
    expect(shouldDismissStartupSplash({ ...base, hasShownOnboarding: false, startupMode: "ready" })).toBe(false);
    expect(shouldDismissStartupSplash({ ...base, hasShownOnboarding: false, startupMode: "ready", initialScanSettled: true })).toBe(true);
  });
});
