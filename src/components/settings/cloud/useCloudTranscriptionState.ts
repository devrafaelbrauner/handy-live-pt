import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type Result } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import type { DropdownOption } from "../../ui/Dropdown";
import type { ModelOption } from "../PostProcessingSettingsApi/types";
import {
  API_KEY_PAGES,
  cloudProviderLabel,
  CUSTOM_CLOUD_PROVIDER_ID,
} from "./cloudProviders";

type PendingAction = "provider" | "baseUrl" | "apiKey" | "model" | "models";

/**
 * Provider, key, base URL and model state for cloud transcription. Settings
 * stay the single source of truth: every change goes through a backend
 * command, then the settings are reloaded.
 */
export const useCloudTranscriptionState = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const [pending, setPending] = useState<ReadonlySet<PendingAction>>(
    () => new Set(),
  );
  // Model ids each provider listed, filled by the refresh button.
  const [listedModels, setListedModels] = useState<
    ReadonlyMap<string, string[]>
  >(() => new Map());
  // Bumped after every base URL save so the field remounts with the stored
  // value: a rejected URL must not linger in the input as if it were saved.
  const [baseUrlRevision, setBaseUrlRevision] = useState(0);

  const providers = settings?.cloud_transcription_providers ?? [];
  const selectedProviderId =
    settings?.cloud_transcription_provider_id ?? providers[0]?.id ?? "openai";
  const selectedProvider =
    providers.find((provider) => provider.id === selectedProviderId) ??
    providers[0];
  const providerLabel = selectedProvider
    ? cloudProviderLabel(selectedProvider, t)
    : selectedProviderId;
  const isCustomProvider = selectedProviderId === CUSTOM_CLOUD_PROVIDER_ID;
  const baseUrl = selectedProvider?.base_url ?? "";
  const apiKey = settings?.post_process_api_keys?.[selectedProviderId] ?? "";
  const model =
    settings?.cloud_transcription_models?.[selectedProviderId] ?? "";

  const track = useCallback(
    async (action: PendingAction, work: () => Promise<void>) => {
      setPending((current) => new Set(current).add(action));
      try {
        await work();
      } finally {
        setPending((current) => {
          const next = new Set(current);
          next.delete(action);
          return next;
        });
      }
    },
    [],
  );

  const runCommand = useCallback(
    (action: PendingAction, command: () => Promise<Result<null, string>>) =>
      track(action, async () => {
        const result = await command();
        if (result.status === "error") {
          toast.error(result.error);
        }
        await refreshSettings();
      }),
    [refreshSettings, track],
  );

  const selectProvider = useCallback(
    (providerId: string) => {
      if (providerId === selectedProviderId) return;
      void runCommand("provider", () =>
        commands.setCloudTranscriptionProvider(providerId),
      );
    },
    [runCommand, selectedProviderId],
  );

  const updateBaseUrl = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (!trimmed || trimmed === baseUrl) return;
      // Models listed by the previous server say nothing about the new one.
      setListedModels((current) => {
        const next = new Map(current);
        next.delete(selectedProviderId);
        return next;
      });
      void runCommand("baseUrl", () =>
        commands.changeCloudTranscriptionBaseUrlSetting(
          selectedProviderId,
          trimmed,
        ),
      ).then(() => setBaseUrlRevision((revision) => revision + 1));
    },
    [baseUrl, runCommand, selectedProviderId],
  );

  const updateApiKey = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (trimmed === apiKey) return;
      void runCommand("apiKey", () =>
        commands.changeCloudTranscriptionApiKeySetting(
          selectedProviderId,
          trimmed,
        ),
      );
    },
    [apiKey, runCommand, selectedProviderId],
  );

  const updateModel = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (!trimmed || trimmed === model) return;
      void runCommand("model", () =>
        commands.changeCloudTranscriptionModelSetting(
          selectedProviderId,
          trimmed,
        ),
      );
    },
    [model, runCommand, selectedProviderId],
  );

  const refreshModels = useCallback(() => {
    const providerId = selectedProviderId;
    void track("models", async () => {
      const result = await commands.fetchCloudTranscriptionModels(providerId);
      if (result.status === "ok") {
        setListedModels((current) =>
          new Map(current).set(providerId, result.data),
        );
      } else {
        toast.error(result.error);
      }
    });
  }, [selectedProviderId, track]);

  const providerOptions = useMemo<DropdownOption[]>(
    () =>
      providers.map((provider) => ({
        value: provider.id,
        label: cloudProviderLabel(provider, t),
      })),
    [providers, t],
  );

  const modelOptions = useMemo<ModelOption[]>(() => {
    const ids = new Set(listedModels.get(selectedProviderId) ?? []);
    if (model) ids.add(model);
    return [...ids].map((id) => ({ value: id, label: id }));
  }, [listedModels, model, selectedProviderId]);

  return {
    providerOptions,
    selectedProviderId,
    providerLabel,
    isCustomProvider,
    baseUrl,
    baseUrlRevision,
    apiKey,
    apiKeyPage: API_KEY_PAGES[selectedProviderId],
    missingApiKey: !isCustomProvider && apiKey.trim() === "",
    model,
    modelOptions,
    isPending: (action: PendingAction) => pending.has(action),
    selectProvider,
    updateBaseUrl,
    updateApiKey,
    updateModel,
    refreshModels,
  };
};
