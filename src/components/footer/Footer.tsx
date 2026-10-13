import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useTranslation } from "react-i18next";
import { Cloud } from "lucide-react";

import ModelSelector from "../model-selector";
import UpdateChecker from "../update-checker";
import { useSettings } from "../../hooks/useSettings";
import { activeCloudModelName } from "../settings/cloud/cloudProviders";

const Footer: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [version, setVersion] = useState("");
  // While cloud transcription is on, the local model picker would only
  // mislead: the footer names the cloud provider and model instead.
  const cloudModelName = activeCloudModelName(settings, t);

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("0.1.2");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="w-full border-t border-mid-gray/20 pt-3">
      <div className="flex justify-between items-center text-xs px-4 pb-3 text-text/60">
        <div className="flex items-center gap-4">
          {cloudModelName ? (
            <div
              className="flex items-center gap-2"
              title={t("modelSelector.cloudTitle")}
            >
              <Cloud className="w-3 h-3 text-logo-primary" />
              <span className="max-w-48 truncate">{cloudModelName}</span>
            </div>
          ) : (
            <ModelSelector />
          )}
        </div>

        {/* Update Status */}
        <div className="flex items-center gap-1">
          <UpdateChecker />
          <span>•</span>
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span>v{version}</span>
        </div>
      </div>
    </div>
  );
};

export default Footer;
