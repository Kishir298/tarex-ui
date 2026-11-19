import type { ToolExecutionOptions } from "ai";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EMPTY_PROVIDER_KEYS } from "../lib/keyring";
import type { CustomEndpoint } from "../config";
import type { ToolContext } from "./context";

const generateTextMock = vi.hoisted(() => vi.fn());
const buildConfiguredLanguageModelMock = vi.hoisted(() => vi.fn());
const prefsState = vi.hoisted(() => ({
  lmstudioBaseURL: "http://lm:1234",
  lmstudioModelId: "lm-qwen",
  mlxBaseURL: "http://mlx:8080",
  mlxModelId: "mlx-qwen",
  ollamaBaseURL: "http://ollama:11434",
  ollamaModelId: "ollama-qwen",
  openaiCompatibleBaseURL: "http://oa:9999",
  openaiCompatibleModelId: "oa-qwen",
  openrouterModelId: "or-qwen",
  customEndpoints: [
    {
      id: "ep-1",
      name: "EP",
      baseURL: "http://ep:1",
      modelId: "ep-model",
      contextLimit: 32_000,
    },
  ] as CustomEndpoint[],
}));
const chatState = vi.hoisted(() => ({
  apiKeys: { openai: "sk-test" },
  selectedModelId: "lmstudio/lm-qwen",
  customEndpointKeys: { "ep-1": "ep-key" },
  patchAgentMeta: vi.fn(),
}));

vi.mock("ai", async (importOriginal) => ({
  ...(await importOriginal<typeof import("ai")>()),
  generateText: generateTextMock,
}));
vi.mock("../lib/agent", () => ({
  buildConfiguredLanguageModel: buildConfiguredLanguageModelMock,
}));
vi.mock("../store/chatStore", () => ({
  useChatStore: { getState: () => chatState },
}));
vi.mock("@/modules/settings/preferences", () => ({
  usePreferencesStore: { getState: () => prefsState },
}));

import { runSubagent } from "../agents/runSubagent";
import { buildSubagentTools } from "./subagent";

const toolOptions: ToolExecutionOptions = {
  toolCallId: "tool-call",
  messages: [],
};

function makeContext(): ToolContext {
  return {
    getCwd: () => "/workspace",
    getWorkspaceRoot: () => "/workspace",
    getTerminalContext: () => null,
    isActiveTerminalPrivate: () => false,
    injectIntoActivePty: () => false,
    openPreview: () => false,
    spawnAgent: () => null,
    readAgentOutput: () => null,
    readCache: new Map(),
    getSessionId: () => "session",
  } as unknown as ToolContext;
}

// biome-ignore lint/suspicious/noExplicitAny: tool results are heterogeneous.
type Result = Record<string, any>;

async function run(input: Record<string, unknown>): Promise<Result> {
  const execute = buildSubagentTools(makeContext()).run_subagent.execute;
  if (!execute) throw new Error("run_subagent has no execute");
  return (await execute(input as never, toolOptions)) as unknown as Result;
}

beforeEach(() => {
  vi.clearAllMocks();
  buildConfiguredLanguageModelMock.mockResolvedValue({ modelId: "resolved" });
  generateTextMock.mockResolvedValue({
    text: "done",
    steps: [{ toolCalls: [] }],
  });
});

describe("runSubagent", () => {
  it("resolves the selected model through buildConfiguredLanguageModel with keys and local config", async () => {
    const r = await runSubagent({
      type: "general",
      prompt: "review it",
      keys: { ...EMPTY_PROVIDER_KEYS, openai: "sk" },
      modelId: "lmstudio/lm-qwen",
      toolContext: makeContext(),
      local: {
        lmstudioBaseURL: "http://lm:1234",
        lmstudioModelId: "lm-qwen",
        customEndpoints: prefsState.customEndpoints,
        customEndpointKeys: { "ep-1": "ep-key" },
      },
    });
    expect(buildConfiguredLanguageModelMock).toHaveBeenCalledTimes(1);
    expect(buildConfiguredLanguageModelMock).toHaveBeenCalledWith(
      "lmstudio/lm-qwen",
      { ...EMPTY_PROVIDER_KEYS, openai: "sk" },
      {
        lmstudioBaseURL: "http://lm:1234",
        lmstudioModelId: "lm-qwen",
        customEndpoints: prefsState.customEndpoints,
        customEndpointKeys: { "ep-1": "ep-key" },
      },
    );
    expect(r.summary).toBe("done");
    expect(r.stepCount).toBe(1);
    expect(r.durationMs).toBeGreaterThanOrEqual(0);
  });

  it("passes a compat model id through unchanged (never cast to ModelId)", async () => {
    await runSubagent({
      type: "general",
      prompt: "p",
      keys: EMPTY_PROVIDER_KEYS,
      modelId: "compat:my-endpoint",
      toolContext: makeContext(),
    });
    expect(buildConfiguredLanguageModelMock).toHaveBeenCalledWith(
      "compat:my-endpoint",
      EMPTY_PROVIDER_KEYS,
      {},
    );
  });

  it("defaults local config to an empty object when omitted", async () => {
    await runSubagent({
      type: "general",
      prompt: "p",
      keys: EMPTY_PROVIDER_KEYS,
      modelId: "openai/gpt-4o-mini",
      toolContext: makeContext(),
    });
    expect(buildConfiguredLanguageModelMock).toHaveBeenCalledWith(
      "openai/gpt-4o-mini",
      EMPTY_PROVIDER_KEYS,
      {},
    );
  });

  it("reports per-step tool labels through onStep", async () => {
    generateTextMock.mockImplementationOnce(async (opts) => {
      opts.onStepFinish({ toolCalls: [{ toolName: "grep" }] });
      return { text: "done", steps: [{ toolCalls: [{ toolName: "grep" }] }] };
    });
    const steps: string[] = [];
    await runSubagent({
      type: "general",
      prompt: "p",
      keys: EMPTY_PROVIDER_KEYS,
      modelId: "openai/gpt-4o-mini",
      toolContext: makeContext(),
      onStep: (label) => steps.push(label),
    });
    expect(steps).toEqual(["general: grep"]);
  });

  it("throws for an unknown subagent type before touching the model", async () => {
    await expect(
      runSubagent({
        type: "nope" as never,
        prompt: "p",
        keys: EMPTY_PROVIDER_KEYS,
        modelId: "openai/gpt-4o-mini",
        toolContext: makeContext(),
      }),
    ).rejects.toThrow("unknown subagent type");
    expect(buildConfiguredLanguageModelMock).not.toHaveBeenCalled();
  });

  it("propagates generateText failures", async () => {
    generateTextMock.mockRejectedValueOnce(new Error("provider down"));
    await expect(
      runSubagent({
        type: "general",
        prompt: "p",
        keys: EMPTY_PROVIDER_KEYS,
        modelId: "openai/gpt-4o-mini",
        toolContext: makeContext(),
      }),
    ).rejects.toThrow("provider down");
  });

  it("falls back to '(no output)' when the model returns no text", async () => {
    generateTextMock.mockResolvedValueOnce({ text: "", steps: [] });
    const r = await runSubagent({
      type: "general",
      prompt: "p",
      keys: EMPTY_PROVIDER_KEYS,
      modelId: "openai/gpt-4o-mini",
      toolContext: makeContext(),
    });
    expect(r.summary).toBe("(no output)");
    expect(r.stepCount).toBe(0);
  });
});

describe("run_subagent tool", () => {
  it("maps the subagent result fields on success", async () => {
    generateTextMock.mockResolvedValueOnce({
      text: "done",
      steps: [{}, {}, {}, {}],
    });
    const r = await run({
      type: "general",
      prompt: "review it",
      description: "card",
    });
    expect(r).toEqual({
      type: "general",
      description: "card",
      summary: "done",
      stepCount: 4,
      durationMs: expect.any(Number),
    });
  });

  it("returns an error result instead of throwing when the subagent fails", async () => {
    generateTextMock.mockRejectedValueOnce(new Error("boom"));
    const r = await run({ type: "general", prompt: "review it" });
    expect(r.type).toBe("general");
    expect(r.error).toContain("boom");
  });

  it("threads chat-store model/keys and preference local config into the run", async () => {
    await run({ type: "general", prompt: "p", description: "d" });
    expect(buildConfiguredLanguageModelMock).toHaveBeenCalledWith(
      chatState.selectedModelId,
      chatState.apiKeys,
      {
        lmstudioBaseURL: prefsState.lmstudioBaseURL,
        lmstudioModelId: prefsState.lmstudioModelId,
        mlxBaseURL: prefsState.mlxBaseURL,
        mlxModelId: prefsState.mlxModelId,
        ollamaBaseURL: prefsState.ollamaBaseURL,
        ollamaModelId: prefsState.ollamaModelId,
        openaiCompatibleBaseURL: prefsState.openaiCompatibleBaseURL,
        openaiCompatibleModelId: prefsState.openaiCompatibleModelId,
        openrouterModelId: prefsState.openrouterModelId,
        customEndpoints: prefsState.customEndpoints,
        customEndpointKeys: chatState.customEndpointKeys,
      },
    );
    expect(generateTextMock).toHaveBeenCalledWith(
      expect.objectContaining({ model: { modelId: "resolved" } }),
    );
  });

  it("reports progress through chat.patchAgentMeta", async () => {
    generateTextMock.mockImplementationOnce(async (opts) => {
      opts.onStepFinish({ toolCalls: [{ toolName: "grep" }] });
      return { text: "done", steps: [{}] };
    });
    await run({ type: "general", prompt: "p" });
    expect(chatState.patchAgentMeta).toHaveBeenCalledWith({
      step: "general: grep",
    });
  });
});
