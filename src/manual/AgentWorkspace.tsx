import { useMemo, useState } from "react";
import {
  ArrowDownToLine,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  RefreshCw,
  Search,
  Star,
  Unlink,
  Wand2,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { ActionButton } from "./ui";
import { compactTokens } from "./log-usage";
import {
  displayModel,
  groupModelIds,
  protocolShortLabels,
  type Agent,
  type AgentsStatus,
  type Group,
} from "./types";

export interface SyncReceipt {
  appliedAt: string;
  files: { path: string; backup: string | null }[];
}

export interface AgentMeta {
  id: Agent;
  name: string;
  configPath: string;
}

export const agentInfo: AgentMeta[] = [
  {
    id: "codex",
    name: "Codex",
    configPath: "~/.codex/config.toml",
  },
  {
    id: "claude",
    name: "Claude Code",
    configPath: "~/.claude/settings.json",
  },
  {
    id: "pi",
    name: "Pi Agent",
    configPath: "~/.pi/agent/models.json",
  },
  {
    id: "opencode",
    name: "OpenCode",
    configPath: "~/.config/opencode/opencode.jsonc",
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    configPath: "~/.openclaw/openclaw.json",
  },
  {
    id: "hermes",
    name: "Hermes Agent",
    configPath: "~/.hermes/config.yaml",
  },
  {
    id: "droid",
    name: "Factory Droid",
    configPath: "~/.factory/settings.json",
  },
];

export function AgentWorkspace({
  agent: controlledAgent,
  onAgentChange,
  disabled,
  canSync,
  activeGroups,
  agentsStatus,
  onCopy,
  onModels,
  onSync,
  onSyncNow,
  onToggleAutoSync,
  onDisconnectCodex,
  onSetCodexDefaultModel,
  onMatchCodexModels,
  onBlockModel,
}: {
  agent?: Agent;
  onAgentChange?: (agent: Agent | undefined) => void;
  disabled: boolean;
  canSync: boolean;
  activeGroups: Group[];
  agentsStatus?: AgentsStatus | null;
  selectedModel?: Group;
  receipts?: Partial<Record<Agent, SyncReceipt>>;
  onCopy: (text: string, label: string) => void;
  onModels: () => void;
  onLaunch?: (agent: Agent, name: string) => void;
  onSync: (agent: Agent) => void;
  onSyncNow?: (agent: Agent) => void;
  onToggleAutoSync?: (agent: Agent, enabled: boolean) => void;
  onDisconnectCodex?: () => void;
  onSetCodexDefaultModel?: (modelId: string) => void;
  onMatchCodexModels?: (modelIds: string[]) => void;
  onBlockModel?: (modelIds: string[], blocked: boolean) => void;
}) {
  const [expandedAgent, setExpandedAgent] = useState<Agent | null>(
    controlledAgent ?? null,
  );
  const [query, setQuery] = useState("");

  const activeExpanded = controlledAgent ?? expandedAgent;

  const setExpanded = (next: Agent | null) => {
    setExpandedAgent(next);
    if (onAgentChange) onAgentChange(next ?? undefined);
  };

  const codex = agentsStatus?.codex;

  const getStatus = (id: Agent) => {
    if (!agentsStatus) return null;
    if (id === "codex") return agentsStatus.codex;
    return agentsStatus[id] ?? null;
  };

  const isConnected = (id: Agent) => !!getStatus(id)?.connected;
  const isInstalled = (id: Agent) => {
    if (id === "codex") {
      return !!(
        codex?.connected ||
        codex?.currentModel ||
        codex?.currentBaseUrl ||
        codex?.catalogPath
      );
    }
    const st = getStatus(id);
    return !!(st && ("installed" in st ? st.installed : false));
  };
  const getAutoSync = (id: Agent) => !!getStatus(id)?.autoSync;

  const connectedCount = agentInfo.filter((a) => isConnected(a.id)).length;

  const sortedAgents = useMemo(() => {
    return [...agentInfo].sort((a, b) => {
      const score = (id: Agent) =>
        (isConnected(id) ? 2 : 0) + (isInstalled(id) ? 1 : 0);
      return score(b.id) - score(a.id);
    });
  }, [agentsStatus]);

  const filteredModels = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return activeGroups.filter((g) => {
      if (!q) return true;
      return `${displayModel(g)} ${g.publicId ?? ""} ${g.providerNames.join(" ")}`
        .toLocaleLowerCase()
        .includes(q);
    });
  }, [activeGroups, query]);

  const codexStatusLabel = !codex
    ? "未接入"
    : codex.connected
      ? "已接入"
      : codex.catalogOwner === "magpie"
        ? "Magpie 接管"
        : codex.mode === "other"
          ? "其他配置"
          : "未接入";

  return (
    <section className="mg-stage mg-agents-stage" aria-label="客户端配置">
      <header className="mg-stage-header">
        <div>
          <h1>Agents</h1>
          <p className="mg-stage-subtitle">
            {`已接入 ${connectedCount} / ${agentInfo.length} · ${activeGroups.length} 个可用模型`}
          </p>
        </div>
        <div className="mg-heading-actions">
          <ActionButton variant="outline" size="sm" onClick={onModels}>
            管理 Providers
          </ActionButton>
        </div>
      </header>

      {!canSync && (
        <div className="mg-inline-banner" role="status">
          <span>请先在 Providers 中启用来源和模型。</span>
          <ActionButton size="sm" onClick={onModels}>
            去配置模型
          </ActionButton>
        </div>
      )}

      <div className="mg-accordion-list" role="list" aria-label="支持接入的 Agent">
        {sortedAgents.map((item) => {
          const connected = isConnected(item.id);
          const installed = isInstalled(item.id);
          const autoSync = getAutoSync(item.id);
          const expanded = activeExpanded === item.id;
          const showForeignMatchBanner =
            item.id === "codex" &&
            codex &&
            codex.catalogOwner !== "lumagate" &&
            codex.catalogModels.length > 0 &&
            !!onMatchCodexModels;
          const defaultModel =
            item.id === "codex" ? codex?.currentModel ?? null : null;

          const summarySub = defaultModel
            ? `默认模型 ${defaultModel}`
            : item.configPath;

          return (
            <article
              key={item.id}
              role="listitem"
              className={`mg-acc-item ${expanded ? "is-expanded" : ""}`}
              aria-label={`Agent ${item.name}`}
            >
              {/* Tier 1: Compact Summary Row */}
              <div className="mg-acc-summary">
                <button
                  type="button"
                  className="mg-acc-trigger"
                  aria-expanded={expanded}
                  aria-label={`展开 ${item.name}`}
                  onClick={() => setExpanded(expanded ? null : item.id)}
                >
                  <span className="mg-acc-chevron" aria-hidden>
                    {expanded ? (
                      <ChevronDown size={15} />
                    ) : (
                      <ChevronRight size={15} />
                    )}
                  </span>
                  <i
                    className={`mg-status-dot ${connected ? "is-on" : ""}`}
                    aria-hidden
                  />
                  <span className="mg-acc-identity">
                    <span className="mg-acc-title-line">
                      <strong>{item.name}</strong>
                      <span
                        className={`mg-pill ${connected ? "is-ok" : item.id === "codex" && codex?.mode === "other" ? "is-warn" : ""}`}
                      >
                        {item.id === "codex"
                          ? codexStatusLabel
                          : connected
                            ? "已接入"
                            : installed
                              ? "本机已装"
                              : "未接入"}
                      </span>
                    </span>
                    <small className="mg-acc-meta">{summarySub}</small>
                  </span>
                </button>

                <div className="mg-acc-right">
                  {!connected ? (
                    <ActionButton
                      size="sm"
                      disabled={disabled || !canSync}
                      aria-label={`一键接入 ${item.name}`}
                      onClick={() => onSync(item.id)}
                    >
                      <ArrowDownToLine size={13} aria-hidden />
                      一键接入
                    </ActionButton>
                  ) : (
                    <ActionButton
                      size="sm"
                      variant="outline"
                      disabled={disabled || !canSync}
                      onClick={() =>
                        onSyncNow ? onSyncNow(item.id) : onSync(item.id)
                      }
                    >
                      <RefreshCw size={12} aria-hidden />
                      同步
                    </ActionButton>
                  )}
                </div>
              </div>

              {/* Tier 2: Minimal Detail Panel (Default model + Config path + Model list) */}
              {expanded && (
                <div className="mg-acc-body mg-agent-detail">
                  <div className="mg-agent-meta-row">
                    <div className="mg-agent-meta-items">
                      {defaultModel && (
                        <span className="mg-agent-kv">
                          <span className="mg-agent-kv-label">默认模型</span>
                          <code>{defaultModel}</code>
                        </span>
                      )}
                      <span className="mg-agent-kv">
                        <span className="mg-agent-kv-label">配置文件</span>
                        <button
                          type="button"
                          className="mg-path-copy"
                          aria-label={`复制 ${item.name} 配置文件路径`}
                          title="复制路径"
                          onClick={() => onCopy(item.configPath, "配置文件路径")}
                        >
                          <code>{item.configPath}</code>
                          <Copy size={11} aria-hidden />
                        </button>
                      </span>
                    </div>

                    <div className="mg-agent-meta-actions">
                      {onToggleAutoSync && (
                        <label className="mg-inline-switch">
                          <span>自动同步</span>
                          <Switch
                            checked={autoSync}
                            disabled={disabled}
                            aria-label={`${item.name} 自动更新可用模型`}
                            onCheckedChange={(next) =>
                              onToggleAutoSync(item.id, next)
                            }
                          />
                        </label>
                      )}
                      {item.id === "codex" && connected && onDisconnectCodex && (
                        <ActionButton
                          size="sm"
                          variant="ghost"
                          disabled={disabled}
                          onClick={onDisconnectCodex}
                        >
                          <Unlink size={12} aria-hidden />
                          断开
                        </ActionButton>
                      )}
                    </div>
                  </div>

                  {showForeignMatchBanner && (
                    <div className="mg-local-match-bar">
                      <div className="mg-local-match-text">
                        <Wand2 size={14} aria-hidden />
                        <span>
                          检测到本机 Codex 当前配置了{" "}
                          <strong>{codex.catalogModels.length}</strong> 个模型
                        </span>
                      </div>
                      <ActionButton
                        size="sm"
                        variant="outline"
                        disabled={disabled}
                        onClick={() => onMatchCodexModels(codex.catalogModels)}
                      >
                        按本机列表匹配启用
                      </ActionButton>
                    </div>
                  )}

                  <div className="mg-agent-models">
                    <div className="mg-agent-models-head">
                      <span className="mg-agent-models-title">
                        可用模型
                        <em>{activeGroups.length}</em>
                      </span>
                      <div className="mg-sub-search">
                        <Search size={13} aria-hidden />
                        <Input
                          type="search"
                          aria-label={`搜索 ${item.name} 可用模型`}
                          placeholder="搜索模型…"
                          value={query}
                          onChange={(e) => setQuery(e.target.value)}
                        />
                      </div>
                    </div>

                    <ul
                      className="mg-client-model-list"
                      aria-label={`${item.name} 模型清单`}
                    >
                      {filteredModels.map((group) => {
                        const slug = group.publicId ?? displayModel(group);
                        const isDefault =
                          item.id === "codex" && codex?.currentModel === slug;
                        return (
                          <li key={group.id} className="mg-client-model-row">
                            <div className="mg-client-model-main">
                              <div className="mg-client-model-name">
                                <strong>{displayModel(group)}</strong>
                                <span className="mg-proto-tag">
                                  {protocolShortLabels[group.protocol]}
                                </span>
                                {isDefault && (
                                  <span className="mg-default-tag">
                                    <Check size={11} aria-hidden />
                                    默认
                                  </span>
                                )}
                              </div>
                              <small>
                                {group.providerNames.join(" · ")}
                                {group.model.contextWindow
                                  ? ` · ${compactTokens(group.model.contextWindow)}`
                                  : ""}
                              </small>
                            </div>
                            <div className="mg-client-model-actions">
                              {item.id === "codex" &&
                                onSetCodexDefaultModel &&
                                !isDefault && (
                                  <ActionButton
                                    variant="ghost"
                                    size="sm"
                                    disabled={disabled}
                                    onClick={() => onSetCodexDefaultModel(slug)}
                                  >
                                    <Star size={12} aria-hidden />
                                    设为默认
                                  </ActionButton>
                                )}
                              <ActionButton
                                variant="ghost"
                                size="icon"
                                aria-label={`复制模型 ID ${slug}`}
                                title="复制模型 ID"
                                onClick={() => onCopy(slug, "模型 ID")}
                              >
                                <Copy size={13} />
                              </ActionButton>
                              {onBlockModel && (
                                <Switch
                                  checked={true}
                                  disabled={disabled}
                                  aria-label={`停用模型 ${displayModel(group)}`}
                                  onCheckedChange={(next) => {
                                    if (!next)
                                      onBlockModel(groupModelIds(group), true);
                                  }}
                                />
                              )}
                            </div>
                          </li>
                        );
                      })}
                    </ul>
                  </div>
                </div>
              )}
            </article>
          );
        })}
      </div>
    </section>
  );
}
