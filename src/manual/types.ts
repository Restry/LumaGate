export type Protocol = "anthropic" | "openai_chat" | "openai_responses";
export type Agent =
  | "pi"
  | "claude"
  | "codex"
  | "opencode"
  | "openclaw"
  | "hermes"
  | "droid";
export interface Model {
  id: string;
  protocolOverride?: Protocol | null;
  name?: string | null;
  maxOutputTokens?: number | null;
  reasoning?: boolean | null;
  thinkingTypes?: string[] | null;
  reasoningSummaries?: boolean | null;
  reasoningLevels?: string[] | null;
  defaultReasoningLevel?: string | null;
  inputModalities?: string[] | null;
  outputModalities?: string[] | null;
  supportedParameters?: string[] | null;
  metadata?: Record<string, unknown> | null;
  image: boolean;
  tools: boolean | null;
  contextWindow: number | null;
  limitsSource?: "user_override" | null;
  enabled: boolean;
}
export interface Source {
  id: string;
  name: string;
  baseUrl: string;
  keyEnv: string;
  keyRef?: string | null;
  /** Azure AD 应用注册租户 ID；有值时走 Client Credentials */
  azureTenantId?: string | null;
  /** Azure AD 应用注册客户端 ID */
  azureClientId?: string | null;
  copilot?: { accountId: string; grantId: string } | null;
  codex?: { accountId: string; grantId: string } | null;
  protocol: Protocol;
  enabled: boolean;
  costEstimationEnabled?: boolean;
  models: Model[];
}

export interface CodexAuthStatus {
  connected: boolean;
  email: string | null;
  accountId: string | null;
  plan: string | null;
  source: string | null;
  localAvailable: boolean;
  localEmail: string | null;
  message: string | null;
}

export interface CodexClientStatus {
  connected: boolean;
  mode: "desktop" | "profile" | "other" | "official";
  currentBaseUrl: string | null;
  currentModel: string | null;
  catalogPath: string | null;
  catalogOwner: "lumagate" | "magpie" | "other" | null;
  catalogModels: string[];
  autoSync: boolean;
  canRestore: boolean;
}

export interface SimpleClientStatus {
  connected: boolean;
  installed?: boolean;
  currentBaseUrl: string | null;
  autoSync: boolean;
}

export interface AgentsStatus {
  codex: CodexClientStatus;
  claude: SimpleClientStatus;
  pi: SimpleClientStatus;
  opencode?: SimpleClientStatus;
  openclaw?: SimpleClientStatus;
  hermes?: SimpleClientStatus;
  droid?: SimpleClientStatus;
}

/** Azure AD Client Credentials 配置是否完整 */
export function isAzureAd(source: {
  azureTenantId?: string | null;
  azureClientId?: string | null;
}): boolean {
  return !!(source.azureTenantId?.trim() && source.azureClientId?.trim());
}
export type KeyUpdate =
  | { action: "set"; providerId: string; value: string }
  | { action: "clear"; providerId: string };

export function keyUpdateFor(
  source: Source,
  value: string,
  useEnv: boolean,
  remove: boolean,
): KeyUpdate | undefined {
  if (useEnv || remove)
    return source.keyRef
      ? { action: "clear", providerId: source.id }
      : undefined;
  return value.trim()
    ? { action: "set", providerId: source.id, value }
    : undefined;
}

export interface Policy {
  balance: "round_robin" | "priority";
  failover: boolean;
  fallbacks: string[];
}
export interface TestResult {
  success: boolean;
  latencyMs: number;
  checkedAt: string;
  detail: string;
  providerId?: string | null;
  providerName?: string | null;
  modelId?: string | null;
}
export interface GatewayDocument {
  revision: number;
  providers: Source[];
  policies: Record<string, Policy>;
  tests: Record<string, TestResult>;
  blockedModels?: string[];
}
export interface Group {
  id: string;
  /** Safe request name from the backend; id remains the stable internal route. */
  publicId?: string;
  /** 展示名：干净的模型 ID，永不带稳定路由哈希。 */
  displayName?: string;
  /** 组内各来源的模型 ID。跨 copilot/ 或 codex/ 命名空间合并时会有多个。 */
  modelIds?: string[];
  model: Model;
  protocol: Protocol;
  automaticProtocol?: Protocol;
  protocolMode?: "auto" | "manual" | "mixed";
  legacyIds?: string[];
  routingError?: string | null;
  blocked?: boolean;
  providerIds: string[];
  providerNames: string[];
  policy: Policy;
}

export function stripNamespace(id: string): string {
  if (id.startsWith("copilot/") && id.length > "copilot/".length) {
    return id.slice("copilot/".length);
  }
  if (id.startsWith("codex/") && id.length > "codex/".length) {
    return id.slice("codex/".length);
  }
  return id;
}

/** 界面标题用它，不用 publicId —— 后者在名称歧义时会退回稳定路由哈希。 */
export function displayModel(group: Group): string {
  if (group.displayName) return group.displayName;
  return stripNamespace(group.model.id);
}
export function groupModelIds(group: Group): string[] {
  return group.modelIds?.length ? group.modelIds : [group.model.id];
}
export function isCopilotGroup(group: Group): boolean {
  return groupModelIds(group).some((id) => id.startsWith("copilot/"));
}
export function isCodexGroup(group: Group): boolean {
  return groupModelIds(group).some((id) => id.startsWith("codex/"));
}
export function isSubscriptionLockedProtocol(group: Group): boolean {
  return (
    group.model.id.startsWith("copilot/") || group.model.id.startsWith("codex/")
  );
}
export interface GatewayAccessKey {
  id: string;
  name: string;
  enabled: boolean;
  canCopy: boolean;
  createdAt: string | null;
}
export interface GatewayAccessKeys {
  revision: number;
  items: GatewayAccessKey[];
}
export type AccessKeyOperation =
  | { action: "add"; name: string; value: string }
  | { action: "rename"; keyId: string; name: string }
  | { action: "set_enabled"; keyId: string; enabled: boolean }
  | { action: "remove"; keyId: string }
  | { action: "restore_copy"; keyId: string; value: string };
export interface AccessKeyChangeResult {
  keys: GatewayAccessKeys;
  warnings: string[];
}
export interface GatewayNetworkSettings {
  revision: number;
  listenAddress: string;
  listenPort: number;
  allowLan: boolean;
}
export interface GatewayNetworkUpdate {
  revision: number;
  listenAddress: string;
  listenPort: number;
}
export interface GatewayNetworkSnapshot {
  saved: GatewayNetworkSettings;
  active: GatewayNetworkSettings | null;
  restartRequired: boolean;
  lanAddresses: string[];
  addressError?: string | null;
  accessKeys: GatewayAccessKeys;
}
export interface Snapshot {
  copilotAuth?: {
    connected: boolean;
    login: string | null;
    message: string | null;
  };
  codexAuth?: CodexAuthStatus;
  agents?: AgentsStatus | null;
  logStorage?: { ready: boolean; path: string; message: string };
  document: GatewayDocument;
  groups: Group[];
  status: {
    running: boolean;
    total_requests?: number;
    active_connections?: number;
  };
  baseUrl: string;
  network?: GatewayNetworkSnapshot;
}
export interface Preview {
  id: string;
  revision: number;
  note: string;
  files: { path: string; beforeHash: string | null; changes: unknown }[];
}
export const protocolPaths: Record<Protocol, string> = {
  anthropic: "/v1/messages",
  openai_chat: "/v1/chat/completions",
  openai_responses: "/v1/responses",
};
export const protocolLabels: Record<Protocol, string> = {
  anthropic: "Anthropic Messages",
  openai_chat: "OpenAI Chat",
  openai_responses: "OpenAI Responses",
};
export const protocolShortLabels: Record<Protocol, string> = {
  anthropic: "Messages",
  openai_chat: "Chat",
  openai_responses: "Responses",
};
export function isBlocked(group: Group, doc: GatewayDocument): boolean {
  return (
    !!group.blocked ||
    groupModelIds(group).every((id) => !!doc.blockedModels?.includes(id))
  );
}
export function testKey(groupId: string, providerId: string): string {
  return JSON.stringify([groupId, providerId]);
}
export function declaredInput(model: Model): string[] | null {
  return model.inputModalities ?? (model.image ? ["text", "image"] : null);
}
export function modalityLabel(values?: string[] | null): string {
  if (!values) return "未知";
  if (!values.length) return "无";
  const labels: Record<string, string> = {
    text: "文本",
    image: "图片",
    audio: "音频",
    video: "视频",
    file: "文件",
  };
  const order = ["text", "image", "audio", "video", "file"];
  return [...new Set(values)]
    .sort(
      (a, b) =>
        (order.indexOf(a) < 0 ? 99 : order.indexOf(a)) -
        (order.indexOf(b) < 0 ? 99 : order.indexOf(b)),
    )
    .map((value) => labels[value] ?? value)
    .join("、");
}
export function compatible(a: Group, b: Group) {
  return (
    !a.blocked &&
    !b.blocked &&
    a.id !== b.id &&
    a.protocol === b.protocol &&
    a.model.image === b.model.image &&
    a.model.tools === b.model.tools &&
    a.model.contextWindow === b.model.contextWindow &&
    (a.model.maxOutputTokens ?? null) === (b.model.maxOutputTokens ?? null) &&
    (a.model.reasoning ?? null) === (b.model.reasoning ?? null) &&
    (a.model.reasoningSummaries ?? null) ===
      (b.model.reasoningSummaries ?? null) &&
    (a.model.defaultReasoningLevel ?? null) ===
      (b.model.defaultReasoningLevel ?? null) &&
    [
      "reasoningLevels",
      "thinkingTypes",
      "inputModalities",
      "outputModalities",
      "supportedParameters",
    ].every((key) => {
      const field = key as
        | "reasoningLevels"
        | "thinkingTypes"
        | "inputModalities"
        | "outputModalities"
        | "supportedParameters";
      return (
        JSON.stringify(a.model[field]?.slice().sort() ?? null) ===
        JSON.stringify(b.model[field]?.slice().sort() ?? null)
      );
    })
  );
}
export function reasoningLabel(model: Model): string {
  if (model.reasoning === false) return "不支持";
  if (model.reasoningLevels?.length) {
    const order = [
      "none",
      "off",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ];
    return model.reasoningLevels
      .slice()
      .sort(
        (a, b) =>
          (order.indexOf(a) < 0 ? 99 : order.indexOf(a)) -
          (order.indexOf(b) < 0 ? 99 : order.indexOf(b)),
      )
      .join(" / ");
  }
  return model.reasoning === true ? "支持（层级未知）" : "未知";
}
export function emptySource(): Source {
  return {
    id: crypto.randomUUID(),
    name: "",
    baseUrl: "",
    keyEnv: "",
    protocol: "openai_chat",
    enabled: true,
    costEstimationEnabled: true,
    models: [],
  };
}
