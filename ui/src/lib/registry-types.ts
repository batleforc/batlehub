/** Package download visibility level. Not a named type in the SDK (server accepts string). */
export type Visibility = "public" | "internal" | "team";

/** `label` is an i18n key, resolved by the consuming component. */
export const VISIBILITY_OPTIONS = [
  { value: "public" as Visibility, label: "visibility.public" },
  { value: "internal" as Visibility, label: "visibility.internal" },
  { value: "team" as Visibility, label: "visibility.team" },
] as const;

/** Beta-channel member (SDK response is untyped). */
export interface BetaChannelMemberDto {
  principal_type: string;
  principal_id: string;
  granted_by: string | null;
}

/** Team namespace (SDK response is untyped). */
export interface TeamNamespaceDto {
  registry: string;
  prefix: string;
  group_id: string;
  claimed_by: string | null;
}

/** Package under a namespace (SDK response is untyped). */
export interface NamespacePackageDto {
  name: string;
  version: string;
  visibility: Visibility;
  published_by: string;
  published_at: string;
  yanked: boolean;
}

/** IP block entry (SDK response is untyped). */
export interface BlockedIpDto {
  ip: string;
  blocked_at: number;
  unblock_at: number;
  reason: string;
}
