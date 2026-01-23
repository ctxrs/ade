import type { MobileConnectionProfile, MobileDeviceRegistration } from "@ctx/types";
import { api } from "./clientBase";

export type CreateMobileProfileRequest = {
  label: string;
  base_url: string;
  scopes?: string[];
};

export type CreateMobileProfileResponse = {
  profile: MobileConnectionProfile;
  token: string;
  qr_payload: any;
};

export type MobileTunnelState = "idle" | "running" | "error";

export type MobileAccessStatus = {
  enabled: boolean;
  tunnel_id?: string | null;
  public_base_url?: string | null;
  relay_base_url?: string | null;
  daemon_public_key?: string | null;
  tunnel_state: MobileTunnelState;
  last_error?: string | null;
};

export type EnableMobileAccessResponse = {
  status: MobileAccessStatus;
  qr_payload: any;
  pairing_expires_at: string;
};

export const listMobileConnectionProfiles = () =>
  api<MobileConnectionProfile[]>(`/api/mobile/connection_profiles`);

export const createMobileConnectionProfile = (payload: CreateMobileProfileRequest) =>
  api<CreateMobileProfileResponse>(`/api/mobile/connection_profiles`, {
    method: "POST",
    body: JSON.stringify(payload),
  });

export const deleteMobileConnectionProfile = (id: string) =>
  api<void>(`/api/mobile/connection_profiles/${id}`, { method: "DELETE" });

export const listMobileDevicesForProfile = (profileId: string) =>
  api<MobileDeviceRegistration[]>(`/api/mobile/connection_profiles/${profileId}/devices`);

export const getMobileAccessStatus = () => api<MobileAccessStatus>(`/api/mobile/access/status`);

export const enableMobileAccess = (supabase_token: string) =>
  api<EnableMobileAccessResponse>(`/api/mobile/access/enable`, {
    method: "POST",
    body: JSON.stringify({ supabase_token }),
  });

export const disableMobileAccess = (supabase_token: string) =>
  api<void>(`/api/mobile/access/disable`, {
    method: "POST",
    body: JSON.stringify({ supabase_token }),
  });
