import { describe, expect, it } from "vitest";
import { shouldShowTreeLoading } from "./sessionTreeLoading";

describe("session tree loading state", () => {
  it("shows loading while the first empty list is still scanning", () => {
    expect(shouldShowTreeLoading({ projectCount: 0, bootstrapping: true, hasCompletedInitialScan: false, isStreamingProjects: true, isRefreshing: true })).toBe(true);
  });

  it("shows the empty state after an empty scan completes", () => {
    expect(shouldShowTreeLoading({ projectCount: 0, bootstrapping: false, hasCompletedInitialScan: true, isStreamingProjects: false, isRefreshing: false })).toBe(false);
  });

  it("keeps existing sessions visible during a refresh", () => {
    expect(shouldShowTreeLoading({ projectCount: 2, bootstrapping: false, hasCompletedInitialScan: true, isStreamingProjects: true, isRefreshing: true })).toBe(false);
  });
});
