import { useEffect, useMemo, useState } from "react";
import { firePendingReviewForSession } from "@/modules/agents/lib/review";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { onKeysChanged } from "@/modules/settings/store";
import {
  endpointModelId,
  endpointModels,
} from "@/modules/ai/lib/endpointModels";
import {
  DEFAULT_MODEL_ID,
  getModel,
  isCompatModelId,
  isKnownModelId,
  PROVIDERS,
  providerNeedsKey,
  type CustomEndpoint,
  type ProviderId,
} from "../config";
import {
  clearCredentialCache,
  getAllCustomEndpointKeys,
  getAllKeys,
  type ProviderKeys,
} from "../lib/keyring";
import { useAgentsStore } from "../store/agentsStore";
import { useChatStore } from "../store/chatStore";
import { useSnippetsStore } from "../store/snippetsStore";

type LocalPrefs = {
  lmstudioModelId: string;
  mlxModelId: string;
  ollamaModelId: string;
  openaiCompatibleBaseURL: string;
  openaiCompatibleModelId: string;
  openrouterModelId: string;
};

/** Mirror of Settings → Models `isConfigured`, shared by the onboarding flag
 *  and the initial model selection. */
function isProviderConfigured(
  id: ProviderId,
  keys: ProviderKeys,
  local: LocalPrefs,
): boolean {
  switch (id) {
    case "openrouter":
      return !!keys.openrouter && !!local.openrouterModelId.trim();
    case "lmstudio":
      return !!local.lmstudioModelId.trim();
    case "mlx":
      return !!local.mlxModelId.trim();
    case "ollama":
      return !!local.ollamaModelId.trim();
    case "openai-compatible":
      return (
        !!local.openaiCompatibleBaseURL.trim() &&
        !!local.openaiCompatibleModelId.trim()
      );
    default:
      return providerNeedsKey(id) ? !!keys[id] : false;
  }
}

function isModelUsable(
  modelId: string,
  endpoints: readonly CustomEndpoint[],
  keys: ProviderKeys,
  local: LocalPrefs,
): boolean {
  if (isCompatModelId(modelId)) {
    return endpointModels(endpoints).some((m) => m.id === modelId);
  }
  if (!isKnownModelId(modelId)) return false;
  return isProviderConfigured(getModel(modelId).provider, keys, local);
}

/**
 * Read every provider + custom-endpoint key from the keyring. The credential
 * cache is cleared first so a change made in the Settings window is never
 * served from a stale entry. Returns the raw maps; the caller decides whether
 * to write them (cancelled effects must not clobber newer state).
 */
export async function fetchProviderKeys(
  endpoints: readonly CustomEndpoint[],
): Promise<{ keys: ProviderKeys; endpointKeys: Record<string, string | null> }> {
  clearCredentialCache();
  const [keys, endpointKeys] = await Promise.all([
    getAllKeys(),
    getAllCustomEndpointKeys(endpoints),
  ]);
  return { keys, endpointKeys };
}

/**
 * Map the stored default-model preference onto a live chat selection:
 * keep it when its provider is configured, otherwise fall back to a
 * configured custom endpoint, then to the preference itself if it is still a
 * known built-in id (amber "no key" hint), else the default model.
 */
export function resolveInitialModelId(
  prefDefaultModel: string,
  endpoints: readonly CustomEndpoint[],
  usable: (modelId: string) => boolean,
): string {
  if (usable(prefDefaultModel)) return prefDefaultModel;
  const ep = endpointModelId(prefDefaultModel, endpoints);
  if (ep) return ep;
  if (isKnownModelId(prefDefaultModel)) return prefDefaultModel;
  return DEFAULT_MODEL_ID;
}

/**
 * Startup wiring for the AI subsystem: hydrates provider keys from the
 * keyring (and keeps them in sync with settings changes), hydrates the
 * preference store and mirrors the default model into chatStore, hydrates
 * chat/agents/snippets stores, and fires any pending review for the active
 * session. Returns the two derived flags the shell needs.
 *
 * `hasComposer` is onboarding UI only — it never gates opening the AI panel.
 */
export function useAiBootstrap(): {
  hasComposer: boolean;
  keysLoaded: boolean;
} {
  const setSelectedModelId = useChatStore((s) => s.setSelectedModelId);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const hydrateSessions = useChatStore((s) => s.hydrateSessions);
  const apiKeys = useChatStore((s) => s.apiKeys);

  useEffect(() => {
    if (activeSessionId) firePendingReviewForSession(activeSessionId);
  }, [activeSessionId]);

  const customEndpoints = usePreferencesStore((s) => s.customEndpoints);
  const prefsHydrated = usePreferencesStore((s) => s.hydrated);
  const lmstudioModelId = usePreferencesStore((s) => s.lmstudioModelId);
  const mlxModelId = usePreferencesStore((s) => s.mlxModelId);
  const ollamaModelId = usePreferencesStore((s) => s.ollamaModelId);
  const openaiCompatibleBaseURL = usePreferencesStore(
    (s) => s.openaiCompatibleBaseURL,
  );
  const openaiCompatibleModelId = usePreferencesStore(
    (s) => s.openaiCompatibleModelId,
  );
  const openrouterModelId = usePreferencesStore((s) => s.openrouterModelId);

  // ── Provider keys: hydrate on mount, refresh after every settings change ──
  // `customEndpoints` doubles as the trigger for the first post-hydration
  // load: the preferences store swaps the array identity when prefs arrive.
  const [providerKeysLoaded, setProviderKeysLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    const reload = async () => {
      try {
        const { keys, endpointKeys } = await fetchProviderKeys(customEndpoints);
        if (cancelled) return;
        const chat = useChatStore.getState();
        chat.setApiKeys(keys);
        chat.setCustomEndpointKeys(endpointKeys);
      } finally {
        if (!cancelled) setProviderKeysLoaded(true);
      }
    };
    void reload();
    const unlistenP = onKeysChanged(() => {
      void reload();
    });
    return () => {
      cancelled = true;
      void unlistenP.then((fn) => fn());
    };
  }, [customEndpoints]);

  // Onboarding flag only: a configured built-in provider or custom endpoint
  // enables the composer surface. The panel itself opens regardless.
  const hasComposer = useMemo(() => {
    const local: LocalPrefs = {
      lmstudioModelId,
      mlxModelId,
      ollamaModelId,
      openaiCompatibleBaseURL,
      openaiCompatibleModelId,
      openrouterModelId,
    };
    if (PROVIDERS.some((p) => isProviderConfigured(p.id, apiKeys, local)))
      return true;
    return endpointModels(customEndpoints).length > 0;
  }, [
    apiKeys,
    customEndpoints,
    lmstudioModelId,
    mlxModelId,
    ollamaModelId,
    openaiCompatibleBaseURL,
    openaiCompatibleModelId,
    openrouterModelId,
  ]);

  // Hydrate the cross-window preference store and mirror the default model
  // into chatStore so the dropdown reflects what the user picked in Settings.
  const initPrefs = usePreferencesStore((s) => s.init);
  const prefDefaultModel = usePreferencesStore((s) => s.defaultModelId);
  useEffect(() => {
    void initPrefs();
  }, [initPrefs]);
  useEffect(() => {
    if (!prefsHydrated || !providerKeysLoaded) return;
    const prefs = usePreferencesStore.getState();
    const local: LocalPrefs = {
      lmstudioModelId: prefs.lmstudioModelId,
      mlxModelId: prefs.mlxModelId,
      ollamaModelId: prefs.ollamaModelId,
      openaiCompatibleBaseURL: prefs.openaiCompatibleBaseURL,
      openaiCompatibleModelId: prefs.openaiCompatibleModelId,
      openrouterModelId: prefs.openrouterModelId,
    };
    const keys = useChatStore.getState().apiKeys;
    const next = resolveInitialModelId(
      prefDefaultModel,
      customEndpoints,
      (id) => isModelUsable(id, customEndpoints, keys, local),
    );
    if (useChatStore.getState().selectedModelId !== next)
      setSelectedModelId(next);
  }, [
    prefsHydrated,
    providerKeysLoaded,
    prefDefaultModel,
    customEndpoints,
    setSelectedModelId,
  ]);

  useEffect(() => {
    void hydrateSessions();
    void useAgentsStore.getState().hydrate();
    void useSnippetsStore.getState().hydrate();
  }, [hydrateSessions]);

  return {
    hasComposer,
    keysLoaded: prefsHydrated && providerKeysLoaded,
  };
}
