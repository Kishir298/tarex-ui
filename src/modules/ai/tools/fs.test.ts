import type { ToolExecutionOptions } from "ai";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ToolContext } from "./context";

const nativeMock = vi.hoisted(() => ({
  canonicalize: vi.fn(async (path: string) => path),
  readFile: vi.fn(),
  /** The user-facing write surface. Agent tools must never reach it. */
  writeFile: vi.fn(async () => undefined),
  createDir: vi.fn(async () => undefined),
  aiSessionCreate: vi.fn(async () => "test-session"),
  aiSetPlanMode: vi.fn(async (active: boolean) => active),
  aiWriteFile: vi.fn(async () => 1),
  aiCreateDir: vi.fn(async () => undefined),
}));

const planMock = vi.hoisted(() => ({
  active: false,
  enqueue: vi.fn(),
}));

vi.mock("../lib/native", () => ({ native: nativeMock }));

vi.mock("../lib/security", () => ({
  checkReadableCanonical: vi.fn(async (path: string) => ({
    ok: true as const,
    canonical: path,
  })),
  checkWritableCanonical: vi.fn(async (path: string) => ({
    ok: true as const,
    canonical: path,
  })),
}));

vi.mock("../store/planStore", () => ({
  newQueuedEditId: () => "queued-edit",
  usePlanStore: {
    getState: () => ({ active: planMock.active, enqueue: planMock.enqueue }),
  },
}));

import { resetAiSession } from "../lib/agentSession";
import { buildFsTools } from "./fs";

const toolOptions: ToolExecutionOptions = {
  toolCallId: "tool-call",
  messages: [],
};

const FILE = "/workspace/a.txt";
const DIR = "/workspace/new-dir";

function makeContext() {
  return {
    getCwd: () => "/workspace",
    getWorkspaceRoot: () => "/workspace",
    getTerminalContext: () => null,
    isActiveTerminalPrivate: () => false,
    injectIntoActivePty: () => false,
    openPreview: () => false,
    spawnAgent: () => null,
    readAgentOutput: () => null,
    readCache: new Map<string, { size: number; hash: number }>(),
    getSessionId: () => "session",
  } as unknown as ToolContext;
}

function tools() {
  return buildFsTools(makeContext());
}

async function runWrite(input: { path: string; content: string }) {
  const execute = tools().write_file.execute;
  if (!execute) throw new Error("write_file tool has no execute");
  return execute(input as never, toolOptions);
}

async function runCreateDir(input: { path: string }) {
  const execute = tools().create_directory.execute;
  if (!execute) throw new Error("create_directory tool has no execute");
  return execute(input as never, toolOptions);
}

function expectNoWrites() {
  expect(nativeMock.aiWriteFile).not.toHaveBeenCalled();
  expect(nativeMock.aiCreateDir).not.toHaveBeenCalled();
  expect(nativeMock.writeFile).not.toHaveBeenCalled();
  expect(nativeMock.createDir).not.toHaveBeenCalled();
}

beforeEach(() => {
  vi.clearAllMocks();
  resetAiSession();
  nativeMock.canonicalize.mockImplementation(async (p: string) => p);
  nativeMock.readFile.mockRejectedValue(new Error("ENOENT"));
  nativeMock.aiSessionCreate.mockResolvedValue("test-session");
  nativeMock.aiWriteFile.mockResolvedValue(1);
  planMock.active = false;
});

describe("write_file", () => {
  it("writes through the gated agent surface with the session token", async () => {
    const result = await runWrite({ path: FILE, content: "hi" });
    expect(result).toMatchObject({ ok: true, bytesWritten: 2 });
    expect(nativeMock.aiWriteFile).toHaveBeenCalledWith(
      FILE,
      "hi",
      "test-session",
    );
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });

  it("queues instead of writing while plan mode is on", async () => {
    planMock.active = true;
    const result = await runWrite({ path: FILE, content: "hi" });
    expect(result).toMatchObject({ queued_for_plan_review: true, ok: true });
    expectNoWrites();
    expect(planMock.enqueue).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "queued-edit",
        kind: "write_file",
        path: FILE,
        proposedContent: "hi",
        isNewFile: true,
      }),
    );
  });

  it("refuses to write when no session token can be minted", async () => {
    nativeMock.aiSessionCreate.mockRejectedValue("ipc down");
    const result = await runWrite({ path: FILE, content: "hi" });
    expect(result).toMatchObject({ error: "agent session unavailable" });
    expectNoWrites();
  });

  it("surfaces a native plan-mode refusal without falling back to writeFile", async () => {
    nativeMock.aiWriteFile.mockRejectedValue(
      "plan mode is active: the change was queued for review instead",
    );
    const result = await runWrite({ path: FILE, content: "hi" });
    expect(result).toMatchObject({
      error: "plan mode is active: the change was queued for review instead",
    });
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });
});

describe("create_directory", () => {
  it("creates through the gated agent surface", async () => {
    const result = await runCreateDir({ path: DIR });
    expect(result).toMatchObject({ ok: true });
    expect(nativeMock.aiCreateDir).toHaveBeenCalledWith(DIR, "test-session");
    expect(nativeMock.createDir).not.toHaveBeenCalled();
  });

  it("queues instead of creating while plan mode is on", async () => {
    planMock.active = true;
    const result = await runCreateDir({ path: DIR });
    expect(result).toMatchObject({ queued_for_plan_review: true, ok: true });
    expectNoWrites();
    expect(planMock.enqueue).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "create_directory", path: DIR }),
    );
  });
});
