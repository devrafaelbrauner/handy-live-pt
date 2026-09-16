import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { StreamLatencyPreset } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Dropdown, type DropdownOption } from "@/components/ui/Dropdown";
import { SettingContainer } from "@/components/ui/SettingContainer";

interface StreamLatencyPresetSelectorProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/**
 * Latency/accuracy trade-off for models with native streaming. Models without
 * a native streaming path ignore the setting, and batch transcription never
 * reads it. "maximum" is the default and leaves the engine's own defaults
 * untouched.
 */
export const StreamLatencyPresetSelector: React.FC<
  StreamLatencyPresetSelectorProps
> = ({ descriptionMode = "tooltip", grouped = false }) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const selectedPreset = getSetting("stream_latency_preset") ?? "maximum";

  const options = useMemo<DropdownOption[]>(
    () => [
      {
        value: "low_latency",
        label: t("settings.advanced.streamLatencyPreset.options.lowLatency"),
      },
      {
        value: "balanced",
        label: t("settings.advanced.streamLatencyPreset.options.balanced"),
      },
      {
        value: "high_latency",
        label: t("settings.advanced.streamLatencyPreset.options.highLatency"),
      },
      {
        value: "maximum",
        label: t("settings.advanced.streamLatencyPreset.options.maximum"),
      },
    ],
    [t],
  );

  return (
    <SettingContainer
      title={t("settings.advanced.streamLatencyPreset.label")}
      description={t("settings.advanced.streamLatencyPreset.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <Dropdown
        options={options}
        selectedValue={selectedPreset}
        onSelect={(value) =>
          updateSetting("stream_latency_preset", value as StreamLatencyPreset)
        }
        disabled={isUpdating("stream_latency_preset")}
      />
    </SettingContainer>
  );
};
