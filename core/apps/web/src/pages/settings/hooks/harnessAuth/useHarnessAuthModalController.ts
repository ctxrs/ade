import { useCallback, useEffect, useRef, useState } from "react";
import type { HarnessAuthModalState } from "../../../SettingsPage.types";
import { defaultEndpointBaseUrlForProvider } from "../../harnessAuthRows";
import {
  defaultEndpointProviderPresetForHarness,
  getHarnessEndpointProviderPreset,
} from "../../harnessEndpointProviders";
import { createOperationOwner, type OwnedOperation } from "./operationOwner";
import {
  harnessEndpointRequiresBaseUrl,
  resolveHarnessAuthModalInitialStage,
} from "./capabilities";

type HarnessAuthModalOperationKey = "subscription-flow" | "modal-action";

export type HarnessAuthModalOperation = OwnedOperation<HarnessAuthModalOperationKey>;

type HarnessAuthModalStateController = {
  harnessAuthModal: HarnessAuthModalState | null;
  claudePendingLoginId: string | null;
  openHarnessAuthModal: (providerId: string) => void;
  closeHarnessAuthModal: () => void;
  patchHarnessAuthModal: (patch: Partial<HarnessAuthModalState>) => void;
  startOperation: (key: HarnessAuthModalOperationKey) => HarnessAuthModalOperation;
  finishOperation: (operation: HarnessAuthModalOperation) => void;
  hasActiveOperation: (key: HarnessAuthModalOperationKey) => boolean;
  patchHarnessAuthModalForOperation: (
    operation: HarnessAuthModalOperation,
    patch: Partial<HarnessAuthModalState>,
  ) => boolean;
  closeHarnessAuthModalForOperation: (operation: HarnessAuthModalOperation) => boolean;
  setClaudePendingLoginIdForOperation: (
    operation: HarnessAuthModalOperation,
    loginId: string | null,
  ) => boolean;
};

const createInitialHarnessAuthModal = (providerId: string): HarnessAuthModalState => {
  const defaultPresetId = defaultEndpointProviderPresetForHarness(providerId);
  const defaultPreset = getHarnessEndpointProviderPreset(defaultPresetId);
  const requiresBaseUrl = harnessEndpointRequiresBaseUrl(providerId);

  return {
    provider_id: providerId,
    stage: resolveHarnessAuthModalInitialStage(providerId),
    endpoint_id: null,
    endpoint_provider_id: defaultPresetId,
    gemini_endpoint_auth_type: "gemini_api_key",
    endpoint_name: "",
    base_url: providerId === "gemini"
      ? (defaultPreset.base_url ?? "")
      : requiresBaseUrl
        ? (defaultPreset.base_url ?? defaultEndpointBaseUrlForProvider(providerId))
        : "",
    api_key: "",
    manual_model_ids: "",
    subscription_label: "",
    subscription_token: "",
    subscription_email: "",
    subscription_provider: "",
    subscription_credentials_json: "",
    subscription_config_toml: "",
    subscription_auth_token_json: "",
    subscription_oauth_creds_json: "",
    subscription_google_accounts_json: "",
    subscription_device_code: null,
    subscription_auth_url: null,
    subscription_status: null,
    subscription_busy: false,
    api_key_busy: false,
  };
};

export function useHarnessAuthModalController(): HarnessAuthModalStateController {
  const [harnessAuthModal, setHarnessAuthModal] = useState<HarnessAuthModalState | null>(null);
  const [claudePendingLoginId, setClaudePendingLoginIdState] = useState<string | null>(null);
  const operationOwnerRef = useRef(createOperationOwner<HarnessAuthModalOperationKey>());

  const cancelAllOperations = useCallback(() => {
    operationOwnerRef.current.cancelAll();
  }, []);

  const openHarnessAuthModal = useCallback((providerId: string) => {
    cancelAllOperations();
    setClaudePendingLoginIdState(null);
    setHarnessAuthModal(createInitialHarnessAuthModal(providerId));
  }, [cancelAllOperations]);

  const closeHarnessAuthModal = useCallback(() => {
    cancelAllOperations();
    setHarnessAuthModal(null);
    setClaudePendingLoginIdState(null);
  }, [cancelAllOperations]);

  const patchHarnessAuthModal = useCallback((patch: Partial<HarnessAuthModalState>) => {
    setHarnessAuthModal((prev: HarnessAuthModalState | null) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const startOperation = useCallback(
    (key: HarnessAuthModalOperationKey): HarnessAuthModalOperation => operationOwnerRef.current.start(key),
    [],
  );

  const finishOperation = useCallback((operation: HarnessAuthModalOperation): void => {
    operationOwnerRef.current.finish(operation);
  }, []);

  const hasActiveOperation = useCallback(
    (key: HarnessAuthModalOperationKey): boolean => operationOwnerRef.current.hasActive(key),
    [],
  );

  const patchHarnessAuthModalForOperation = useCallback(
    (operation: HarnessAuthModalOperation, patch: Partial<HarnessAuthModalState>): boolean => {
      if (!operation.isCurrent()) return false;
      setHarnessAuthModal((prev: HarnessAuthModalState | null) => (prev ? { ...prev, ...patch } : prev));
      return true;
    },
    [],
  );

  const closeHarnessAuthModalForOperation = useCallback((operation: HarnessAuthModalOperation): boolean => {
    if (!operation.isCurrent()) return false;
    closeHarnessAuthModal();
    return true;
  }, [closeHarnessAuthModal]);

  const setClaudePendingLoginIdForOperation = useCallback(
    (operation: HarnessAuthModalOperation, loginId: string | null): boolean => {
      if (!operation.isCurrent()) return false;
      setClaudePendingLoginIdState(loginId);
      return true;
    },
    [],
  );

  useEffect(() => () => {
    operationOwnerRef.current.cancelAll();
  }, []);

  return {
    harnessAuthModal,
    claudePendingLoginId,
    openHarnessAuthModal,
    closeHarnessAuthModal,
    patchHarnessAuthModal,
    startOperation,
    finishOperation,
    hasActiveOperation,
    patchHarnessAuthModalForOperation,
    closeHarnessAuthModalForOperation,
    setClaudePendingLoginIdForOperation,
  };
}
