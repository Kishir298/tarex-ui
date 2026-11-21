import type { ToolExecutionOptions } from "ai";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ToolContext } from "./context";

const nativeMock = vi.hoisted(() => ({
  canonicalize: vi.fn(async (path: string) => path),
  readFile: vi.fn(),
  /** The user-facing write surface. Agent tools must never reach it. */
  writeFile: vi.fn(async () => undefined),
  aiSessionCreate: vi.fn(async () => "test-session"),
  aiSetPlanMode: vi.fn(async (active: boolean) => active),
  aiWriteFile: vi.fn(async () => 1),
}));

const planMock = vi.hoisted(() => ({
  active: false,
  enqueue: vi.fn(),
}));

vi.mock("../lib/native", () => ({ native: nativeMock }));

vi.mock("../lib/security", () => ({
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
import { buildEditTools } from "./edit";

const toolOptions: ToolExecutionOptions = {
  toolCallId: "tool-call",
  messages: [],
};

const FILE = "/workspace/a.txt";

function makeContext(readCache: Map<string, { size: number; hash: number }>) {
  return {
    getCwd: () => "/workspace",
    getWorkspaceRoot: () => "/workspace",
    getTerminalContext: () => null,
    isActiveTerminalPrivate: () => false,
    injectIntoActivePty: () => false,
    openPreview: () => false,
    spawnAgent: () => null,
    readAgentOutput: () => null,
    readCache,
    getSessionId: () => "session",
  } as unknown as ToolContext;
}

/** Context with the file already marked as read, satisfying read-before-edit. */
function readContext() {
  return makeContext(new Map([[FILE, { size: 0, hash: 0 }]]));
}

function setFile(content: string) {
  nativeMock.readFile.mockResolvedValue({
    kind: "text",
    content,
    size: content.length,
  });
}

type EditResult = { error?: string; replacements?: number };

async function runEdit(
  ctx: ToolContext,
  input: Record<string, unknown>,
): Promise<EditResult> {
  const execute = buildEditTools(ctx).edit.execute;
  if (!execute) throw new Error("edit tool has no execute");
  return (await execute(input as never, toolOptions)) as unknown as EditResult;
}

async function runMultiEdit(
  ctx: ToolContext,
  input: Record<string, unknown>,
): Promise<EditResult> {
  const execute = buildEditTools(ctx).multi_edit.execute;
  if (!execute) throw new Error("multi_edit tool has no execute");
  return (await execute(input as never, toolOptions)) as unknown as EditResult;
}

beforeEach(() => {
  vi.clearAllMocks();
  resetAiSession();
  nativeMock.canonicalize.mockImplementation(async (p: string) => p);
  nativeMock.writeFile.mockResolvedValue(undefined);
  nativeMock.aiSessionCreate.mockResolvedValue("test-session");
  nativeMock.aiWriteFile.mockResolvedValue(1);
  planMock.active = false;
});

/** No path may mutate a file, through the gated or the ungated command. */
function expectNoWrites() {
  expect(nativeMock.aiWriteFile).not.toHaveBeenCalled();
  expect(nativeMock.writeFile).not.toHaveBeenCalled();
}

describe("edit tool guards", () => {
  it("refuses to edit a file that was never read (read-before-edit)", async () => {
    setFile("hello world");
    const result = await runEdit(makeContext(new Map()), {
      path: FILE,
      old_string: "hello",
      new_string: "bye",
    });
    expect(result.error).toContain("read_file");
    expectNoWrites();
  });

  it("errors when old_string is absent", async () => {
    setFile("hello world");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "missing",
      new_string: "x",
    });
    expect(result.error).toContain("not found");
    expectNoWrites();
  });

  it("errors when old_string is not unique and replace_all is off", async () => {
    setFile("a a");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "a",
      new_string: "b",
    });
    expect(result.error).toContain("not unique");
    expectNoWrites();
  });

  it("rejects an identical old and new string", async () => {
    setFile("hello");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "hello",
      new_string: "hello",
    });
    expect(result.error).toContain("identical");
    expectNoWrites();
  });

  it("rejects an empty old_string", async () => {
    setFile("hello");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "",
      new_string: "x",
    });
    expect(result.error).toContain("empty");
    expectNoWrites();
  });

  it("refuses binary and oversized files", async () => {
    nativeMock.readFile.mockResolvedValue({ kind: "binary", size: 10 });
    const binary = await runEdit(readContext(), {
      path: FILE,
      old_string: "a",
      new_string: "b",
    });
    expect(binary.error).toContain("binary");

    nativeMock.readFile.mockResolvedValue({
      kind: "toolarge",
      size: 999,
      limit: 10,
    });
    const large = await runEdit(readContext(), {
      path: FILE,
      old_string: "a",
      new_string: "b",
    });
    expect(large.error).toContain("too large");
    expectNoWrites();
  });
});

describe("edit tool replacement", () => {
  it("writes the file with a single unique replacement", async () => {
    setFile("hello world");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "world",
      new_string: "there",
    });
    expect(result.replacements).toBe(1);
    expect(nativeMock.aiWriteFile).toHaveBeenCalledWith(
      FILE,
      "hello there",
      "test-session",
    );
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });

  it("replaces every occurrence and counts them with replace_all", async () => {
    setFile("a b a b a");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "a",
      new_string: "z",
      replace_all: true,
    });
    expect(result.replacements).toBe(3);
    expect(nativeMock.aiWriteFile).toHaveBeenCalledWith(
      FILE,
      "z b z b z",
      "test-session",
    );
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });
});

describe("multi_edit atomicity", () => {
  it("applies edits in order against the running buffer", async () => {
    setFile("one two");
    const result = await runMultiEdit(readContext(), {
      path: FILE,
      edits: [
        { old_string: "one", new_string: "1" },
        { old_string: "two", new_string: "2" },
      ],
    });
    expect(result.replacements).toBe(2);
    expect(nativeMock.aiWriteFile).toHaveBeenCalledWith(
      FILE,
      "1 2",
      "test-session",
    );
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });

  it("writes nothing when a later edit in the batch fails", async () => {
    setFile("one two");
    const result = await runMultiEdit(readContext(), {
      path: FILE,
      edits: [
        { old_string: "one", new_string: "1" },
        { old_string: "absent", new_string: "x" },
      ],
    });
    expect(result.error).toContain("not found");
    expectNoWrites();
  });

  it("writes nothing when a later edit is non-unique", async () => {
    setFile("one dup dup");
    const result = await runMultiEdit(readContext(), {
      path: FILE,
      edits: [
        { old_string: "one", new_string: "1" },
        { old_string: "dup", new_string: "x" },
      ],
    });
    expect(result.error).toContain("not unique");
    expectNoWrites();
  });
});

describe("plan mode", () => {
  it("queues the edit instead of writing while plan mode is on", async () => {
    planMock.active = true;
    setFile("hello world");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "world",
      new_string: "there",
    });
    expect(result).toMatchObject({ ok: true, path: FILE });
    expectNoWrites();
    expect(planMock.enqueue).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "queued-edit",
        kind: "edit",
        path: FILE,
        proposedContent: "hello there",
      }),
    );
  });

  it("never mints a session token for an edit that is only queued", async () => {
    planMock.active = true;
    setFile("hello");
    await runEdit(readContext(), {
      path: FILE,
      old_string: "hello",
      new_string: "x",
    });
    expect(nativeMock.aiSessionCreate).not.toHaveBeenCalled();
  });

  it("falls back to a refused write when Rust still holds plan mode", async () => {
    nativeMock.aiWriteFile.mockRejectedValue(
      "plan mode is active: the change was queued for review instead",
    );
    setFile("hello world");
    const result = await runEdit(readContext(), {
      path: FILE,
      old_string: "world",
      new_string: "there",
    });
    expect(result.error).toContain("plan mode is active");
    expect(nativeMock.writeFile).not.toHaveBeenCalled();
  });
});
