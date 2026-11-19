import { beforeEach, describe, expect, it, vi } from "vitest";

const keyringMock = vi.hoisted(() => ({
  clearCredentialCache: vi.fn(),
  getAllKeys: vi.fn<() => Promise<Record<string, string>>>(
    async () => ({ openai: "sk-test" }),
  ),
  getAllCustomEndpointKeys: vi.fn(
    async (): Promise<Record<string, string | null>> => ({ ep1: "ep-key" }),
  ),
}));

vi.mock("../lib/keyring", () => keyringMock);
vi.mock("@/modules/agents/lib/review", () => ({
  firePendingReviewForSession: vi.fn(),
}));
vi.mock("@/modules/settings/preferences", () => ({
  usePreferencesStore: Object.assign(vi.fn(), {
    getState: vi.fn(),
  }),
}));
vi.mock("@/modules/settings/store", () => ({
  onKeysChanged: vi.fn(async () => () => {}),
}));
vi.mock("../store/agentsStore", () => ({ useAgentsStore: { getState: vi.fn() } }));
vi.mock("../store/chatStore", () => ({ useChatStore: { getState: vi.fn() } }));
vi.mock("../store/snippetsStore", () => ({
  useSnippetsStore: { getState: vi.fn() },
}));

import { fetchProviderKeys, resolveInitialModelId } from "./useAiBootstrap";
import { DEFAULT_MODEL_ID, type CustomEndpoint } from "../config";

const EP: CustomEndpoint = {
  id: "ep1",
  name: "Local EP",
  baseURL: "http://ep:1",
  modelId: "ep-model",
  contextLimit: 32_000,
};

beforeEach(() => vi.clearAllMocks());

describe("fetchProviderKeys", () => {
  it("clears the credential cache before reading the keyring", async () => {
    const r = await fetchProviderKeys([EP]);
    expect(keyringMock.clearCredentialCache).toHaveBeenCalledTimes(1);
    expect(
      keyringMock.clearCredentialCache.mock.invocationCallOrder[0],
    ).toBeLessThan(keyringMock.getAllKeys.mock.invocationCallOrder[0]);
    expect(r.keys).toEqual({ openai: "sk-test" });
    expect(r.endpointKeys).toEqual({ ep1: "ep-key" });
  });

  it("reads endpoint keys for the given endpoints", async () => {
    await fetchProviderKeys([]);
    expect(keyringMock.getAllCustomEndpointKeys).toHaveBeenCalledWith([]);
    await fetchProviderKeys([EP]);
    expect(keyringMock.getAllCustomEndpointKeys).toHaveBeenCalledWith([EP]);
  });

  it("propagates keyring failures to the caller", async () => {
    keyringMock.getAllKeys.mockRejectedValueOnce(new Error("keyring down"));
    await expect(fetchProviderKeys([])).rejects.toThrow("keyring down");
  });
});

describe("resolveInitialModelId", () => {
  it("keeps the stored default when its provider is configured", () => {
    expect(
      resolveInitialModelId("openai/gpt-4o-mini", [], () => true),
    ).toBe("openai/gpt-4o-mini");
  });

  it("maps an unusable compat default onto its endpoint model id", () => {
    const compatId = "compat-ep1";
    expect(
      resolveInitialModelId(compatId, [EP], () => false),
    ).toBe(compatId);
  });

  it("falls back to the first configured endpoint when the default is unusable", () => {
    expect(
      resolveInitialModelId("openai/gpt-4o", [EP], () => false),
    ).toBe("compat-ep1");
  });

  it("keeps a known built-in default when unusable (amber hint) and no endpoint is configured", () => {
    expect(resolveInitialModelId("claude-sonnet-5", [], () => false)).toBe(
      "claude-sonnet-5",
    );
  });

  it("falls back to the default model for an unknown preference with no endpoints", () => {
    expect(resolveInitialModelId("totally/bogus", [], () => false)).toBe(
      DEFAULT_MODEL_ID,
    );
  });

  it("prefers a matching endpoint model over the built-in fallback", () => {
    const other: CustomEndpoint = {
      id: "ep2",
      name: "Other",
      baseURL: "http://ep2:1",
      modelId: "other-model",
      contextLimit: 8_000,
    };
    // Unusable compat id for a *second* endpoint resolves to that endpoint.
    expect(
      resolveInitialModelId("compat-ep2", [EP, other], () => false),
    ).toBe("compat-ep2");
    expect(
      resolveInitialModelId("compat-ep2", [other], () => false),
    ).toBe("compat-ep2");
    // Unusable non-compat id falls back to the first endpoint of the list.
    expect(resolveInitialModelId("openai/gpt-4o", [other, EP], () => false)).toBe(
      "compat-ep2",
    );
  });
});
