export function shouldShowTreeLoading(state: {
  projectCount: number;
  bootstrapping: boolean;
  hasCompletedInitialScan: boolean;
  isStreamingProjects: boolean;
  isRefreshing: boolean;
}): boolean {
  return state.projectCount === 0 && (
    state.bootstrapping || !state.hasCompletedInitialScan || state.isStreamingProjects || state.isRefreshing
  );
}
