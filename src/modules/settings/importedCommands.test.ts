import {
  applyImportedCommand,
  coerceServers,
  formatCommandValue,
  isApplyable,
} from "@/modules/settings/importedCommands";
import { beforeEach, describe, expect, it, vi } from "vitest";

const store = vi.hoisted(() => ({
  setTerminalShell: vi.fn(async () => {}),
  setEditorCustomFormatCommand: vi.fn(async () => {}),
  setAgentLaunchCommands: vi.fn(async () => {}),
  setLspCustomServers: vi.fn(async () => {}),
}));

vi.mock("@/modules/settings/store", () => store);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("isApplyable", () => {
  it("accepts exactly the four command-bearing keys", () => {
    expect(isApplyable("terminalShell")).toBe(true);
    expect(isApplyable("editorCustomFormatCommand")).toBe(true);
    expect(isApplyable("agentLaunchCommands")).toBe(true);
    expect(isApplyable("lspCustomServers")).toBe(true);
  });

  it("refuses anything else", () => {
    expect(isApplyable("customInstructions")).toBe(false);
    expect(isApplyable("__proto__")).toBe(false);
    expect(isApplyable("")).toBe(false);
  });
});

describe("coerceServers", () => {
  it("keeps a well-formed entry", () => {
    const servers = coerceServers([
      {
        id: "weird",
        name: "Weird LSP",
        command: "weird-lsp",
        args: ["--stdio"],
        languages: { ts: "typescript" },
        rootMarkers: ["package.json"],
      },
    ]);
    expect(servers).toEqual([
      {
        id: "weird",
        name: "Weird LSP",
        command: "weird-lsp",
        args: ["--stdio"],
        languages: { ts: "typescript" },
        rootMarkers: ["package.json"],
      },
    ]);
  });

  it("drops entries with no usable command", () => {
    expect(coerceServers([{}, { command: "  " }, { command: 42 }, null, "x"])).toEqual([]);
    expect(coerceServers("not-an-array")).toEqual([]);
    expect(coerceServers(undefined)).toEqual([]);
  });

  it("repairs missing fields instead of trusting them", () => {
    const [server] = coerceServers([{ command: "only-command" }]);
    expect(server.id).toBe("imported-0");
    expect(server.name).toBe("only-command");
    expect(server.args).toEqual([]);
    expect(server.languages).toEqual({});
    expect(server.rootMarkers).toEqual([]);
  });

  it("filters non-string members out of list fields", () => {
    const [server] = coerceServers([
      { command: "c", args: ["ok", 7, null, false], languages: { a: 1 }, rootMarkers: ["r", 2] },
    ]);
    expect(server.args).toEqual(["ok"]);
    expect(server.rootMarkers).toEqual(["r"]);
    expect(server.languages).toEqual({});
  });
});

describe("formatCommandValue", () => {
  it("returns strings untouched", () => {
    expect(formatCommandValue("/bin/zsh")).toBe("/bin/zsh");
  });

  it("pretty-prints structured values", () => {
    expect(formatCommandValue({ claude: "claude" })).toBe(
      '{\n  "claude": "claude"\n}',
    );
  });

  it("truncates very long previews", () => {
    const text = formatCommandValue("x".repeat(500));
    expect(text.length).toBeLessThanOrEqual(221);
    expect(text.endsWith("…")).toBe(true);
  });

  it("handles nullish values", () => {
    expect(formatCommandValue(null)).toBe("");
    expect(formatCommandValue(undefined)).toBe("");
  });
});

describe("applyImportedCommand", () => {
  it("routes each key to its settings setter", async () => {
    await applyImportedCommand("terminalShell", "/bin/fish");
    expect(store.setTerminalShell).toHaveBeenCalledWith("/bin/fish");

    await applyImportedCommand("editorCustomFormatCommand", "prettier -w");
    expect(store.setEditorCustomFormatCommand).toHaveBeenCalledWith("prettier -w");

    await applyImportedCommand("agentLaunchCommands", { claude: "claude" });
    expect(store.setAgentLaunchCommands).toHaveBeenCalledWith({ claude: "claude" });

    await applyImportedCommand("lspCustomServers", [{ command: "ls" }]);
    expect(store.setLspCustomServers).toHaveBeenCalledWith([
      {
        id: "imported-0",
        name: "ls",
        command: "ls",
        args: [],
        languages: {},
        rootMarkers: [],
      },
    ]);
  });

  it("refuses unknown keys rather than guessing", async () => {
    await expect(
      applyImportedCommand("customInstructions", "ignore me"),
    ).rejects.toThrow(/unknown setting/);
    expect(store.setTerminalShell).not.toHaveBeenCalled();
  });

  it("coerces non-string shell values to an empty string", async () => {
    await applyImportedCommand("terminalShell", { evil: true });
    expect(store.setTerminalShell).toHaveBeenCalledWith("");
  });
});
