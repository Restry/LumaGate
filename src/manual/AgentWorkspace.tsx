import { useMemo, useState } from "react";
import {
  ArrowDownToLine,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  Play,
  RefreshCw,
  Search,
  Star,
  Unlink,
  Wand2,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { ActionButton } from "./ui";
import { checkedAtLabel } from "./catalog-view";
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
  desc: string;
  configPath: string;
  command?: string;
}

export const agentInfo: AgentMeta[] = [
  {
    id: "codex",
    name: "Codex",
    desc: "OpenAI Codex 桌面端与 CLI（支持自动更新模型目录）",
    configPath: "~/.codex/config.toml",
    command: "codex --profile cc_switch_manual",
  },
  {
    id: "claude",
    name: "Claude Code",
    desc: "Anthropic Claude Code CLI",
    configPath: "~/.claude/settings.json",
  },
  {
    id: "pi",
    name: "Pi Agent",
    desc: "Pi 编程代理（自动按协议分组写入模型）",
    configPath: "~/.pi/agent/models.json",
  },
  {
    id: "opencode",
    name: "OpenCode",
    desc: "OpenCode CLI（写入 provider.lumagate）",
    configPath: "~/.config/opencode/opencode.jsonc",
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    desc: "OpenClaw Agent（合并写入 models.providers.lumagate）",
    configPath: "~/.openclaw/openclaw.json",
  },
  {
    id: "hermes",
    name: "Hermes Agent",
    desc: "Nous Hermes（写入 custom_providers）",
    configPath: "~/.hermes/config.yaml",
  },
  {
    id: "droid",
    name: "Factory Droid",
    desc: "Factory Droid CLI（写入 customModels）",
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
  receipts,
  onCopy,
  onModels,
  onLaunch,
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
  receipts: Partial<Record<Agent, SyncReceipt>>;
  onCopy: (text: string, label: string) => void;
  onModels: () => void;
  onLaunch: (agent: Agent, name: string) => void;
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
  const [modelsOpenFor, setModelsOpenFor] = useState<Agent | null>(null);
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
    : codex.mode === "desktop"
      ? "已接入（桌面端 + CLI）"
      : codex.mode === "profile"
        ? "已接入（Profile）"
        : codex.catalogOwner === "magpie"
          ? "由 Magpie 接管"
          : codex.mode === "other"
            ? "指向其他配置"
            : "未接入";

  return (
    <section className="mg-stage mg-agents-stage" aria-label="客户端配置">
      <header className="mg-stage-header">
        <div>
          <h1>Agents</h1>
          <p className="mg-stage-subtitle">
            {`已接入 ${connectedCount} / ${agentInfo.length} 款 Agent · 当前有 ${activeGroups.length} 个可用模型 · 点击任意条目展开详情`}
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
          <span>还没有可用模型，接入前请先在 Providers 中启用来源和模型。</span>
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
          const receipt = receipts[item.id];
          const status = getStatus(item.id);
          const showForeignMatchBanner =
            item.id === "codex" &&
            codex &&
            codex.catalogOwner !== "lumagate" &&
            codex.catalogModels.length > 0 &&
            !!onMatchCodexModels;
          const modelsOpen = modelsOpenFor === item.id;

          const summarySub =
            item.id === "codex" && codex?.currentModel
              ? `默认模型 ${codex.currentModel} · ${item.configPath}`
              : item.desc;

          return (
            <article
              key={item.id}
              role="listitem"
              className={`mg-acc-item ${expanded ? "is-expanded" : ""}`}
              aria-label={`Agent ${item.name}`}
            >
              {/* Collapsed Summary Row (Tier 1) */}
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
                      {connected && autoSync && (
                        <span className="mg-pill is-subtle">自动同步</span>
                      )}
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
                    <>
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
                      <ActionButton
                        size="sm"
                        variant="ghost"
                        disabled={disabled}
                        aria-label={`启动 ${item.name}`}
                        title={`在终端启动 ${item.name}`}
                        onClick={() => onLaunch(item.id, item.name)}
                      >
                        <Play size={13} aria-hidden />
                      </ActionButton>
                    </>
                  )}
                </div>
              </div>

              {/* Expanded Detail Panel (Tier 2) */}
              {expanded && (
                <div className="mg-acc-body mg-agent-detail">
                  <div className="mg-agent-detail-bar">
                    <div className="mg-agent-detail-kv">
                      <span>
                        配置文件：<code>{item.configPath}</code>
                      </span>
                      {status?.currentBaseUrl && (
                        <span>
                          当前接口：<code>{status.currentBaseUrl}</code>
                        </span>
                      )}
                      {item.command && (
                        <button
                          type="button"
                          className="mg-inline-copy"
                          aria-label="复制网关启动命令"
                          onClick={() =>
                            onCopy(item.command!, `${item.name} 启动命令`)
                          }
                        >
                          <Copy size={11} aria-hidden />
                          <code>{item.command}</code>
                        </button>
                      )}
                    </div>

                    <div className="mg-agent-detail-controls">
                      {onToggleAutoSync && (
                        <label className="mg-inline-switch">
                          <span>模型变更自动更新</span>
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
                      <ActionButton
                        size="sm"
                        variant="outline"
                        disabled={disabled}
                        onClick={() => onLaunch(item.id, item.name)}
                      >
                        <Play size={12} aria-hidden />
                        启动 {item.name}
                      </ActionButton>
                      {item.id === "codex" && connected && onDisconnectCodex && (
                        <ActionButton
                          size="sm"
                          variant="ghost"
                          disabled={disabled}
                          onClick={onDisconnectCodex}
                        >
                          <Unlink size={12} aria-hidden />
                          断开还原
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
                          <strong>{codex.catalogModels.length}</strong> 个模型（
                          {codex.catalogOwner === "magpie" ? "Magpie" : "本机目录"}），
                          可一键在 LumaGate 中匹配启用同名模型。
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

                  {receipt && (
                    <section
                      className="mg-sync-toast"
                      aria-label={`${item.name} 同步结果`}
                    >
                      <div className="mg-sync-toast-head">
                        <CheckCircle2 size={14} aria-hidden />
                        <span>
                          本次会话已同步 · {checkedAtLabel(receipt.appliedAt)} · 写入{" "}
                          {receipt.files.length} 个文件（不代表客户端已连接）
                        </span>
                      </div>
                      {receipt.files.length > 0 && (
                        <details className="mg-sync-toast-files">
                          <summary>查看写入文件与备份</summary>
                          <ul>
                            {receipt.files.map((file) => (
                              <li key={file.path}>
                                <code>{file.path}</code>
                                {file.backup && <span>备份：{file.backup}</span>}
                              </li>
                            ))}
                          </ul>
                        </details>
                      )}
                    </section>
                  )}

                  {/* Tier 3: Collapsible model list for this Agent */}
                  <div className="mg-sub-disclosure">
                    <button
                      type="button"
                      className="mg-sub-disclosure-trigger"
                      aria-expanded={modelsOpen}
                      onClick={() =>
                        setModelsOpenFor(modelsOpen ? null : item.id)
                      }
                    >
                      {modelsOpen ? (
                        <ChevronDown size={14} aria-hidden />
                      ) : (
                        <ChevronRight size={14} aria-hidden />
                      )}
                      <span>
                        {item.name} 可用模型（{activeGroups.length}）
                      </span>
                      <small>
                        {item.id === "codex"
                          ? "点击展开可设默认模型、复制 ID 或停用"
                          : "点击展开查看或复制同步到此 Agent 的模型"}
                      </small>
                    </button>

                    {modelsOpen && (
                      <div className="mg-sub-disclosure-body">
                        <div className="mg-sub-search">
                          <Search size={13} aria-hidden />
                          <Input
                            type="search"
                            aria-label={`搜索 ${item.name} 可用模型`}
                            placeholder="搜索可用模型…"
                            value={query}
                            onChange={(e) => setQuery(e.target.value)}
                          />
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
                                        当前默认
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
                    )}
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
