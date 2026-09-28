import { invokeApp, renderAppError } from "./invokeApp";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";

const ALLOWED_EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export type MarkdownLinkFailureReason =
  | "missing-context"
  | "outside-project"
  | "invalid-path"
  | "unsupported-protocol"
  | "open-failed";

export interface MarkdownLocalLink {
  path: string;
  anchor: string | null;
}

export interface MarkdownLinkFailure {
  href: string;
  reason: MarkdownLinkFailureReason;
  message?: string;
}

function getClickedAnchor(event: MouseEvent): HTMLAnchorElement | null {
  const path = event.composedPath?.() ?? [];
  for (const item of path) {
    if (item instanceof HTMLAnchorElement && item.hasAttribute("href")) {
      return item;
    }
  }

  const target = event.target;
  if (!(target instanceof Element)) return null;

  const anchor = target.closest("a[href]");
  return anchor instanceof HTMLAnchorElement ? anchor : null;
}

function normalizeSlashes(value: string): string {
  return value.replace(/\\/g, "/");
}

function stripTrailingSlash(value: string): string {
  const normalized = normalizeSlashes(value);
  if (normalized === "/" || /^[A-Za-z]:\/$/.test(normalized)) return normalized;
  return normalized.replace(/\/+$/, "");
}

function splitPathPrefix(path: string): { prefix: string; rest: string } {
  const normalized = normalizeSlashes(path);
  const driveMatch = normalized.match(/^[A-Za-z]:\//);
  if (driveMatch) {
    return { prefix: driveMatch[0], rest: normalized.slice(driveMatch[0].length) };
  }
  if (normalized.startsWith("//")) {
    return { prefix: "//", rest: normalized.slice(2) };
  }
  if (normalized.startsWith("/")) {
    return { prefix: "/", rest: normalized.slice(1) };
  }
  return { prefix: "", rest: normalized };
}

function joinPath(prefix: string, segments: string[]): string {
  const body = segments.filter(Boolean).join("/");
  if (!prefix) return body;
  if (!body) return prefix;
  return prefix === "/" ? `/${body}` : `${prefix}${body}`;
}

function dirname(path: string): string {
  const normalized = normalizeSlashes(path);
  const index = normalized.lastIndexOf("/");
  if (index <= 0) return "";
  return normalized.slice(0, index);
}

function resolvePath(basePath: string, relativePath: string): string {
  const { prefix, rest } = splitPathPrefix(basePath);
  const stack = rest ? rest.split("/").filter(Boolean) : [];

  for (const segment of normalizeSlashes(relativePath).split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (stack.length > 0) stack.pop();
      continue;
    }
    stack.push(segment);
  }

  return joinPath(prefix, stack);
}

function isExternalHref(href: string): boolean {
  return /^(https?:|mailto:|tel:)/i.test(href) || href.startsWith("//");
}

function normalizeExternalHref(href: string): string | null {
  const normalizedHref = href.startsWith("//") ? `https:${href}` : href;

  try {
    const url = new URL(normalizedHref);
    return ALLOWED_EXTERNAL_PROTOCOLS.has(url.protocol) ? url.toString() : null;
  } catch {
    return null;
  }
}

function isUnsupportedProtocolHref(href: string): boolean {
  return /^[A-Za-z][A-Za-z0-9+.-]*:/.test(href) && !isExternalHref(href) && !/^[A-Za-z]:[\\/]/.test(href);
}

function splitHrefPath(rawHref: string): { rawPath: string; anchor: string | null } | null {
  const trimmed = rawHref.trim();
  if (!trimmed || trimmed.startsWith("#")) return null;

  const hashIndex = trimmed.indexOf("#");
  const queryIndex = trimmed.indexOf("?");
  const pathEndCandidates = [hashIndex, queryIndex].filter((index) => index >= 0);
  const pathEnd = pathEndCandidates.length > 0 ? Math.min(...pathEndCandidates) : trimmed.length;
  const rawPath = trimmed.slice(0, pathEnd).trim();
  if (!rawPath) return null;

  const anchor = hashIndex >= 0 ? safeDecode(trimmed.slice(hashIndex + 1)) : null;
  return { rawPath, anchor };
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function decodePath(path: string): string {
  return normalizeSlashes(path)
    .split("/")
    .map((segment) => safeDecode(segment))
    .join("/");
}

function isWithinPath(path: string, root: string): boolean {
  const normalizedPath = stripTrailingSlash(path);
  const normalizedRoot = stripTrailingSlash(root);
  const comparePath = /^[A-Za-z]:\//.test(normalizedPath) ? normalizedPath.toLowerCase() : normalizedPath;
  const compareRoot = /^[A-Za-z]:\//.test(normalizedRoot) ? normalizedRoot.toLowerCase() : normalizedRoot;
  return comparePath === compareRoot || comparePath.startsWith(`${compareRoot}/`);
}

function isFilesystemAbsolutePath(path: string): boolean {
  const normalized = normalizeSlashes(path);
  return normalized.startsWith("/") || normalized.startsWith("//") || /^[A-Za-z]:\//.test(normalized);
}

export function resolveMarkdownLocalLink(
  href: string,
  options: { currentFilePath?: string; projectRoot?: string } = {},
): MarkdownLocalLink | MarkdownLinkFailure | null {
  const parsed = splitHrefPath(href);
  if (!parsed) return null;

  const decodedPath = decodePath(parsed.rawPath);
  const projectRoot = options.projectRoot ? stripTrailingSlash(options.projectRoot) : undefined;
  const currentFilePath = options.currentFilePath ? normalizeSlashes(options.currentFilePath) : undefined;
  let resolvedPath: string;

  if (decodedPath.startsWith("/") && projectRoot && !isWithinPath(decodedPath, projectRoot)) {
    return {
      href,
      reason: "outside-project",
      message: t("common.markdownLink.outsideProject"),
    };
  } else if (isFilesystemAbsolutePath(decodedPath)) {
    resolvedPath = normalizeSlashes(decodedPath);
  } else if (currentFilePath) {
    resolvedPath = resolvePath(dirname(currentFilePath), decodedPath);
  } else if (projectRoot) {
    resolvedPath = resolvePath(projectRoot, decodedPath);
  } else {
    return {
      href,
      reason: "missing-context",
      message: t("common.markdownLink.missingProjectPath"),
    };
  }

  if (projectRoot && !isWithinPath(resolvedPath, projectRoot)) {
    return {
      href,
      reason: "outside-project",
      message: t("common.markdownLink.outsideProject"),
    };
  }

  return { path: resolvedPath, anchor: parsed.anchor };
}

async function validateWorkspaceFile(path: string, projectRoot?: string): Promise<string> {
  if (!projectRoot) return path;
  return invokeApp<string>("validate_workspace_file", { path, projectRoot });
}

export async function handleMarkdownLinkClick(
  event: MouseEvent,
  options: {
    currentFilePath?: string;
    projectRoot?: string;
    onOpenFile?: (link: MarkdownLocalLink) => void | Promise<void>;
    onFailure?: (failure: MarkdownLinkFailure) => void;
  } = {},
): Promise<void> {
  const anchor = getClickedAnchor(event);
  if (!anchor) return;

  const rawHref = anchor.getAttribute("href")?.trim();
  if (!rawHref || rawHref.startsWith("#")) return;

  event.preventDefault();
  event.stopPropagation();

  if (isExternalHref(rawHref)) {
    const href = normalizeExternalHref(rawHref);
    if (!href) {
      options.onFailure?.({ href: rawHref, reason: "unsupported-protocol" });
      return;
    }

    try {
      await openUrl(href);
    } catch (error) {
      console.error("Failed to open external link:", error);
      options.onFailure?.({ href: rawHref, reason: "open-failed", message: renderAppError(error) });
    }
    return;
  }

  if (isUnsupportedProtocolHref(rawHref)) {
    options.onFailure?.({ href: rawHref, reason: "unsupported-protocol" });
    return;
  }

  const localLink = resolveMarkdownLocalLink(rawHref, {
    currentFilePath: options.currentFilePath,
    projectRoot: options.projectRoot,
  });
  if (!localLink) return;
  if ("reason" in localLink) {
    options.onFailure?.(localLink);
    return;
  }
  if (!options.onOpenFile) {
    options.onFailure?.({
      href: rawHref,
      reason: "missing-context",
      message: t("common.markdownLink.unsupportedView"),
    });
    return;
  }

  try {
    const validatedPath = await validateWorkspaceFile(localLink.path, options.projectRoot);
    await options.onOpenFile({ ...localLink, path: validatedPath });
  } catch (error) {
    const message = renderAppError(error);
    console.warn("Invalid markdown local link:", message);
    options.onFailure?.({ href: rawHref, reason: "invalid-path", message });
  }
}
