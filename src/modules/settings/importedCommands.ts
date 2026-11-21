import type { AgentLaunchCommands } from "@/modules/agents/lib/launcher";
import {
  setAgentLaunchCommands,
  setEditorCustomFormatCommand,
  setLspCustomServers,
  setTerminalShell,
  type LspCustomServer,
} from "@/modules/settings/store";

/** One command-bearing preference the legacy import held back for review. */
export type ImportedCommand = {
  key: string;
  label: string;
  value: unknown;
};

export type ImportedCommands = {
  source: string;
  commands: ImportedCommand[];
};

const PREVIEW_LIMIT = 220;

/** Keys this review flow is allowed to write, and only those. */
export const APPLYABLE_KEYS = [
  "terminalShell",
  "editorCustomFormatCommand",
  "agentLaunchCommands",
  "lspCustomServers",
] as const;

export function isApplyable(key: string): key is (typeof APPLYABLE_KEYS)[number] {
  return (APPLYABLE_KEYS as readonly string[]).includes(key);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * Rebuilds a language-server entry from untrusted imported JSON. Anything
 * missing a usable command is dropped: a half-formed server would otherwise
 * reach `lsp_spawn` and fail there instead of here.
 */
export function coerceServers(value: unknown): LspCustomServer[] {
  if (!Array.isArray(value)) return [];
  const servers: LspCustomServer[] = [];
  for (const raw of value) {
    const entry = asRecord(raw);
    if (!entry) continue;
    const command = typeof entry.command === "string" ? entry.command : "";
    if (!command.trim()) continue;
    const args = Array.isArray(entry.args)
      ? entry.args.filter((arg): arg is string => typeof arg === "string")
      : [];
    const languages: Record<string, string> = {};
    if (entry.languages !== null && typeof entry.languages === "object" && !Array.isArray(entry.languages)) {
      for (const [id, language] of Object.entries(entry.languages as Record<string, unknown>)) {
        if (typeof language === "string") languages[id] = language;
      }
    }
    const rootMarkers = Array.isArray(entry.rootMarkers)
      ? entry.rootMarkers.filter((m): m is string => typeof m === "string")
      : [];
    servers.push({
      id: typeof entry.id === "string" && entry.id ? entry.id : `imported-${servers.length}`,
      name: typeof entry.name === "string" && entry.name ? entry.name : command,
      command,
      args,
      languages,
      rootMarkers,
    });
  }
  return servers;
}

export function formatCommandValue(value: unknown): string {
  let text: string;
  if (typeof value === "string") {
    text = value;
  } else if (value === null || value === undefined) {
    text = "";
  } else {
    try {
      text = JSON.stringify(value, null, 2) ?? "";
    } catch {
      text = String(value);
    }
  }
  return text.length > PREVIEW_LIMIT ? `${text.slice(0, PREVIEW_LIMIT)}…` : text;
}

/**
 * Writes one accepted value through the normal settings setters, so every open
 * window sees the change through the usual preference event. Throws on an
 * unknown key rather than guessing at one.
 */
export async function applyImportedCommand(
  key: string,
  value: unknown,
): Promise<void> {
  switch (key) {
    case "terminalShell":
      await setTerminalShell(typeof value === "string" ? value : "");
      return;
    case "editorCustomFormatCommand":
      await setEditorCustomFormatCommand(typeof value === "string" ? value : "");
      return;
    case "agentLaunchCommands":
      await setAgentLaunchCommands(
        (asRecord(value) ?? {}) as AgentLaunchCommands,
      );
      return;
    case "lspCustomServers":
      await setLspCustomServers(coerceServers(value));
      return;
    default:
      throw new Error(`refusing to apply unknown setting "${key}"`);
  }
}
