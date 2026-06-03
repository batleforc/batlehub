/** Package download visibility level. Not a named type in the SDK (server accepts string). */
export type Visibility = "public" | "internal" | "team";

export const VISIBILITY_OPTIONS = [
  { value: "public" as Visibility,   label: "Public — anyone can download" },
  { value: "internal" as Visibility, label: "Internal — authenticated users only" },
  { value: "team" as Visibility,     label: "Team — namespace group members only" },
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
