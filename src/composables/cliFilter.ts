import type { CliId, CliOption } from "../types/cli";

export type CliFilterState =
  | { mode: "all" }
  | { mode: "custom"; cliIds: CliId[] };

/** Returns the selected CLI IDs in the display order supplied by the caller. */
export function resolveVisibleCliIds(state: CliFilterState, available: CliId[]): CliId[] {
  if (state.mode === "all") return [...available];

  const selected = new Set(state.cliIds);
  return available.filter((cliId) => selected.has(cliId));
}

export function toggleVisibleCli(
  state: CliFilterState,
  available: CliId[],
  cliId: CliId,
): CliFilterState {
  const selected = new Set(state.mode === "all" ? available : state.cliIds);

  if (selected.has(cliId)) {
    selected.delete(cliId);
  } else if (available.includes(cliId)) {
    selected.add(cliId);
  }

  return {
    mode: "custom",
    cliIds: [
      ...available.filter((availableCliId) => selected.has(availableCliId)),
      ...[...selected].filter((selectedCliId) => !available.includes(selectedCliId)),
    ],
  };
}

/**
 * Resolves a feature page's local CLI without reading or changing the history
 * filter/current CLI. Candidates must already be filtered for that feature's capability.
 */
export function resolveFeatureCliId(
  candidates: CliOption[],
  entryCliId?: CliId,
  lastUsed?: CliId,
): CliId | undefined {
  const candidateIds = new Set(candidates.map((candidate) => candidate.id));

  if (entryCliId && candidateIds.has(entryCliId)) return entryCliId;
  if (lastUsed && candidateIds.has(lastUsed)) return lastUsed;
  return candidates[0]?.id;
}
