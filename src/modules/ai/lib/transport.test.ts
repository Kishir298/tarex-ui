import { beforeEach, describe, expect, it, vi } from "vitest";

const readFileMock = vi.hoisted(() => vi.fn());

vi.mock("./native", () => ({
  native: { readFile: readFileMock },
}));
vi.mock("./agent", () => ({
  runAgentStream: vi.fn(),
}));

import { readProjectMemory } from "./transport";

function text(content: string) {
  return { kind: "text" as const, content, size: content.length };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("readProjectMemory", () => {
  it("returns null without touching the filesystem when there is no workspace root", async () => {
    expect(await readProjectMemory(null)).toBeNull();
    expect(readFileMock).not.toHaveBeenCalled();
  });

  it("reads TAREX.md first", async () => {
    readFileMock.mockResolvedValueOnce(text("tarex content"));
    expect(await readProjectMemory("/w-tarex-first")).toBe("tarex content");
    expect(readFileMock).toHaveBeenCalledWith("/w-tarex-first/TAREX.md");
    expect(readFileMock).toHaveBeenCalledTimes(1);
  });

  it("falls back to the legacy TERAX.md when TAREX.md is missing", async () => {
    readFileMock.mockImplementation(async (path: string) => {
      if (path.endsWith("TAREX.md")) throw new Error("ENOENT");
      return text("legacy content");
    });
    expect(await readProjectMemory("/w-terax-fallback")).toBe("legacy content");
    expect(readFileMock).toHaveBeenLastCalledWith("/w-terax-fallback/TERAX.md");
    expect(readFileMock).toHaveBeenCalledTimes(2);
  });

  it("falls back when TAREX.md exists but is not readable text", async () => {
    readFileMock.mockImplementation(async (path: string) => {
      if (path.endsWith("TAREX.md"))
        return { kind: "binary" as const, size: 10 };
      return text("legacy content");
    });
    expect(await readProjectMemory("/w-binary-tarex")).toBe("legacy content");
    expect(readFileMock).toHaveBeenCalledTimes(2);
  });

  it("returns null when neither file exists", async () => {
    readFileMock.mockRejectedValue(new Error("ENOENT"));
    expect(await readProjectMemory("/w-neither")).toBeNull();
    expect(readFileMock).toHaveBeenCalledTimes(2);
  });

  it("caches the resolved content for repeat reads", async () => {
    readFileMock.mockResolvedValueOnce(text("cached"));
    expect(await readProjectMemory("/w-cached")).toBe("cached");
    expect(await readProjectMemory("/w-cached")).toBe("cached");
    expect(readFileMock).toHaveBeenCalledTimes(1);
  });

  it("caches a negative result so a missing file is not re-probed", async () => {
    readFileMock.mockRejectedValue(new Error("ENOENT"));
    expect(await readProjectMemory("/w-cached-null")).toBeNull();
    expect(await readProjectMemory("/w-cached-null")).toBeNull();
    expect(readFileMock).toHaveBeenCalledTimes(2);
  });

  it("truncates oversized memory files to 32 KiB", async () => {
    readFileMock.mockResolvedValueOnce(text("x".repeat(40 * 1024)));
    const content = await readProjectMemory("/w-truncate");
    expect(content).toHaveLength(32 * 1024);
  });

  it("strips a trailing slash from the workspace root", async () => {
    readFileMock.mockResolvedValueOnce(text("ok"));
    expect(await readProjectMemory("/w-slash/")).toBe("ok");
    expect(readFileMock).toHaveBeenCalledWith("/w-slash/TAREX.md");
  });
});
