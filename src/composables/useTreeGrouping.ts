import { ref } from "vue";
import type { CliId } from "../types/cli";
import { isCliId, SUPPORTED_CLIS } from "../types/cli";
import type { AggregatedProjectInfo, SessionInfo } from "../types/session";

export type TreeGrouping = "directory" | "provider";

export interface ProviderGroup {
  cliId: CliId;
  projects: AggregatedProjectInfo[];
  count: number;
}

export const TREE_GROUPING_STORAGE_KEY = "seshbuddy-tree-grouping";

function parseTimestamp(timestamp: string): number {
  const time = Date.parse(timestamp);
  return Number.isNaN(time) ? 0 : time;
}

function sortSessionsDesc(sessions: SessionInfo[]): SessionInfo[] {
  return [...sessions].sort((a, b) => {
    const aTime = parseTimestamp(a.timestamp);
    const bTime = parseTimestamp(b.timestamp);
    return bTime - aTime;
  });
}

function getLatestProjectTime(project: AggregatedProjectInfo): number {
  if (project.sessions.length === 0) return 0;
  return parseTimestamp(project.sessions[0].timestamp);
}

export function groupProjectsByProvider(
  projects: AggregatedProjectInfo[],
  cliOrder: CliId[] = SUPPORTED_CLIS.map((cli) => cli.id),
): ProviderGroup[] {
  if (!projects || projects.length === 0) return [];

  const cliProjectsMap = new Map<CliId, AggregatedProjectInfo[]>();

  for (const project of projects) {
    const sessionsByCli = new Map<CliId, SessionInfo[]>();
    for (const session of project.sessions) {
      if (isCliId(session.cli_id)) {
        const list = sessionsByCli.get(session.cli_id);
        if (list) {
          list.push(session);
        } else {
          sessionsByCli.set(session.cli_id, [session]);
        }
      }
    }

    for (const [cliId, sessions] of sessionsByCli.entries()) {
      const sortedSessions = sortSessionsDesc(sessions);
      const subProject: AggregatedProjectInfo = {
        project_key: project.project_key,
        encoded_dir: project.encoded_dir,
        original_path: project.original_path,
        cli_ids: [cliId],
        sessions: sortedSessions,
      };

      const existingProjects = cliProjectsMap.get(cliId);
      if (existingProjects) {
        existingProjects.push(subProject);
      } else {
        cliProjectsMap.set(cliId, [subProject]);
      }
    }
  }

  const result: ProviderGroup[] = [];

  for (const cliId of cliOrder) {
    const groupProjects = cliProjectsMap.get(cliId);
    if (!groupProjects || groupProjects.length === 0) continue;

    const sortedProjects = [...groupProjects].sort((a, b) => {
      return getLatestProjectTime(b) - getLatestProjectTime(a);
    });

    const totalCount = sortedProjects.reduce(
      (sum, p) => sum + p.sessions.length,
      0,
    );

    result.push({
      cliId,
      projects: sortedProjects,
      count: totalCount,
    });
  }

  return result;
}

export function useTreeGrouping(initialGrouping?: TreeGrouping) {
  let initial = initialGrouping;
  if (!initial && typeof localStorage !== "undefined") {
    try {
      const stored = localStorage.getItem(TREE_GROUPING_STORAGE_KEY);
      if (stored === "provider" || stored === "directory") {
        initial = stored;
      }
    } catch {
      // ignore
    }
  }

  const grouping = ref<TreeGrouping>(initial || "directory");

  function setGrouping(next: TreeGrouping) {
    grouping.value = next;
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.setItem(TREE_GROUPING_STORAGE_KEY, next);
      } catch {
        // ignore
      }
    }
  }

  function toggleGrouping() {
    setGrouping(grouping.value === "directory" ? "provider" : "directory");
  }

  return {
    grouping,
    setGrouping,
    toggleGrouping,
  };
}
