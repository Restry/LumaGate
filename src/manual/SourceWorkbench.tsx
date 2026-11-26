import { useMemo, useState } from "react";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  KeyRound,
  Layers3,
  Plus,
  RefreshCw,
  Settings2,
  Sparkles,
  Trash2,
  Unlink,
} from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { ActionButton } from "./ui";
import { ModelCatalog } from "./ModelCatalog";
import type { ModelActions } from "./ModelDetail";
import type { CatalogView } from "./catalog-view";
import {
  isAzureAd,
  isBlocked,
  type CodexAuthStatus,
  type Snapshot,
  type Source,
} from "./types";

export function SourceWorkbench({
  snapshot,
  view,
  onViewChange,
  sources,
  refreshingId,
  feedback,
  copilotStatus,
  codexStatus,
  onToggle,
  onAdd,
  onEdit,
  onDelete,
  onDiscover,
  onRefresh,
  onCandidate,
  onBatchCandidates,
  onCostEstimation,
  onCopilotLogin,
  onCopilotReset,
  onCodexLogin,
  onCodexReset,
  onOpenAgents,
  ...actions
}: ModelActions & {
  snapshot: Snapshot;
  view: CatalogView;
  onViewChange: (view: CatalogView) => void;
  sources: Source[];
  refreshingId?: string | null;
  feedback: Record<string, { text: string; error: boolean }>;
  copilotStatus?: {
    connected: boolean;
    login: string | null;
    message: string | null;
  };
  codexStatus?: CodexAuthStatus;
  onToggle: (source: Source, enabled: boolean) => void;
  onAdd: () => void;
  onEdit: (source: Source) => void;
  onDelete: (source: Source) => void;
  onDiscover: (source: Source) => void;
  onRefresh: () => void;
  onCandidate: (modelId: string, providerId: string, enabled: boolean) => void;
  onBatchCandidates?: (
    providerId: string,
    enabled: boolean,
    modelIds?: string[],
  ) => void;
  onCostEstimation: (source: Source, enabled: boolean) => void;
  onBulkSelect?: (source: Source) => void;
  onCopilotLogin?: () => void;
  onCopilotReset?: () => void;
  onCodexLogin?: () => void;
  onCodexReset?: () => void;
  onOpenAgents?: () => void;
}) {
  const [showAllModels, setShowAllModels] = useState(false);

  // Expanded provider id follows view.providerId when scoped to a specific source
  const expandedId = view.providerId !== "all" ? view.providerId : null;
  const allModelsVisible =
    view.providerId === "all" && (showAllModels || sources.length === 0);

  const codexSource = sources.find((s) => !!s.codex);
  const copilotSource = sources.find((s) => !!s.copilot);
  const apiSources = sources.filter((s) => !s.codex && !s.copilot);

  const orderedSources = useMemo(() => {
    const list: Source[] = [];
    if (codexSource) list.push(codexSource);
    if (copilotSource) list.push(copilotSource);
    list.push(...apiSources);
    return list;
  }, [codexSource, copilotSource, apiSources]);

  const activeGroupCount = snapshot.groups.filter(
    (g) => !isBlocked(g, snapshot.document),
  ).length;
  const enabledProviderCount = sources.filter((s) => s.enabled).length;

  function toggleExpand(sourceId: string) {
    setShowAllModels(false);
    if (expandedId === sourceId) {
      onViewChange({
        ...view,
        providerId: "all",
        selectedId: null,
      });
    } else {
      onViewChange({
        ...view,
        providerId: sourceId,
        selectedId: null,
        status: view.status === "disabled" ? "all" : view.status,
      });
    }
  }

  function toggleAllModelsView() {
    const next = !showAllModels;
    setShowAllModels(next);
    if (next) {
      onViewChange({
        ...view,
        providerId: "all",
        selectedId: null,
      });
    }
  }

  function authBadge(source: Source) {
    if (source.codex) {
      return codexStatus?.plan
        ? `Codex ${codexStatus.plan.toUpperCase()}`
        : "Codex 订阅";
    }
    if (source.copilot) return "GitHub Copilot";
    if (isAzureAd(source)) return "Azure AD";
    return source.protocol === "anthropic" ? "Anthropic" : "OpenAI";
  }

  function sourceMetaText(source: Source) {
    if (source.codex) {
      return codexStatus?.email || "chatgpt.com/backend-api/codex";
    }
    if (source.copilot) {
      return copilotStatus?.login ? `@${copilotStatus.login}` : "GitHub 个人订阅";
    }
    return source.baseUrl.replace(/^https?:\/\//, "");
  }

  return (
      <section
        className="mg-stage mg-providers-stage"
        aria-label="来源与模型工作区"
      >
        <header className="mg-stage-header">
          <div>
            <h1>Providers</h1>
          <p className="mg-stage-subtitle">
            {sources.length === 0
              ? "先连接 Codex 订阅、Copilot 或 API 来源，点击任意条目展开管理模型"
              : `${enabledProviderCount} / ${sources.length} 个已启用 · ${activeGroupCount} 个可用模型 · 点击条目展开模型清单`}
          </p>
        </div>
        <div className="mg-heading-actions">
          {sources.length > 0 && !codexSource && onCodexLogin && (
            <ActionButton
              variant="outline"
              size="sm"
              disabled={actions.busy}
              onClick={onCodexLogin}
            >
              <Sparkles size={13} aria-hidden />
              接入 Codex 订阅
              {codexStatus?.localAvailable && (
                <em className="mg-inline-tag">本机已登录</em>
              )}
            </ActionButton>
          )}
          {sources.length > 0 && !copilotSource && onCopilotLogin && (
            <ActionButton
              variant="outline"
              size="sm"
              disabled={actions.busy}
              onClick={onCopilotLogin}
            >
              <Plus size={13} aria-hidden />
              接入 GitHub Copilot
            </ActionButton>
          )}
          {sources.length > 0 && (
            <ActionButton size="sm" disabled={actions.busy} onClick={onAdd}>
              <Plus size={14} aria-hidden />
              添加 API 来源
            </ActionButton>
          )}
        </div>
      </header>

      {/* Main Tier 1: Progressive Provider Stack */}
      {orderedSources.length === 0 ? (
        <div className="mg-empty-stage">
          <h2>还没有配置任何 Provider</h2>
          <p>
            支持直接使用本机已登录的 Codex 订阅、GitHub Copilot，或任意 OpenAI /
            Anthropic 兼容 API。
          </p>
          <div className="mg-empty-actions">
            {onCodexLogin && (
              <ActionButton disabled={actions.busy} onClick={onCodexLogin}>
                <Sparkles size={14} aria-hidden />
                接入 Codex 订阅
                {codexStatus?.localAvailable && (
                  <em className="mg-inline-tag">本机已登录</em>
                )}
              </ActionButton>
            )}
            {onCopilotLogin && (
              <ActionButton
                variant="outline"
                disabled={actions.busy}
                onClick={onCopilotLogin}
              >
                接入 GitHub Copilot
              </ActionButton>
            )}
            <ActionButton
              variant="outline"
              disabled={actions.busy}
              onClick={onAdd}
            >
              <Plus size={14} aria-hidden />
              添加 API 来源
            </ActionButton>
          </div>
        </div>
      ) : (
        <div className="mg-accordion-list" aria-label="模型来源">
          {orderedSources.map((source) => {
            const isExpanded = expandedId === source.id;
            const enabledModels = source.models.filter((m) => m.enabled).length;
            const totalModels = source.models.length;
            const notice = feedback[source.id];
            const isRefreshing = refreshingId === source.id;

            return (
              <section
                key={source.id}
                className={`mg-acc-item ${isExpanded ? "is-expanded" : ""} ${!source.enabled ? "is-muted" : ""}`}
                aria-label={`来源 ${source.name}`}
              >
                {/* Collapsed Summary Row (Tier 1) */}
                <div className="mg-acc-summary">
                  <button
                    type="button"
                    className="mg-acc-trigger"
                    aria-expanded={isExpanded}
                    aria-label={`筛选来源 ${source.name}`}
                    onClick={() => toggleExpand(source.id)}
                  >
                    <span className="mg-acc-chevron" aria-hidden>
                      {isExpanded ? (
                        <ChevronDown size={15} />
                      ) : (
                        <ChevronRight size={15} />
                      )}
                    </span>
                    <i
                      className={`mg-status-dot ${source.enabled ? "is-on" : ""}`}
                      aria-hidden
                    />
                    <span className="mg-acc-identity">
                      <span className="mg-acc-title-line">
                        <strong>{source.name}</strong>
                        <span className="mg-pill">{authBadge(source)}</span>
                        {notice?.error && (
                          <span className="mg-pill is-error" title={notice.text}>
                            <AlertCircle size={11} aria-hidden />
                            需关注
                          </span>
                        )}
                      </span>
                      <small className="mg-acc-meta">{sourceMetaText(source)}</small>
                    </span>
                  </button>

                  <div className="mg-acc-right">
                    <button
                      type="button"
                      className="mg-acc-count"
                      onClick={() => toggleExpand(source.id)}
                      title="展开管理此来源的模型"
                    >
                      <span>
                        {enabledModels} / {totalModels}
                      </span>
                      <small>模型</small>
                    </button>

                    <ActionButton
                      variant="ghost"
                      size="sm"
                      disabled={actions.busy}
                      aria-busy={isRefreshing}
                      aria-label={`刷新 ${source.name} 模型`}
                      title="刷新模型列表"
                      onClick={() => onDiscover(source)}
                    >
                      <RefreshCw
                        size={13}
                        className={isRefreshing ? "is-spin" : ""}
                        aria-hidden
                      />
                    </ActionButton>

                    <Switch
                      checked={source.enabled}
                      disabled={actions.busy}
                      aria-label={`${source.name} 参与路由`}
                      onCheckedChange={(enabled) => onToggle(source, enabled)}
                    />
                  </div>
                </div>

                {/* Expanded Detail Panel (Tier 2) */}
                {isExpanded && (
                  <div className="mg-acc-body">
                    <div className="mg-acc-toolbar">
                      <div className="mg-acc-toolbar-meta">
                        <KeyRound size={12} aria-hidden />
                        <code>{sourceMetaText(source)}</code>
                      </div>
                      <div className="mg-acc-toolbar-actions">
                        <label className="mg-inline-switch">
                          <span>费用估算</span>
                          <Switch
                            checked={source.costEstimationEnabled !== false}
                            disabled={actions.busy}
                            aria-label={`${source.name} 参与费用估算`}
                            onCheckedChange={(enabled) =>
                              onCostEstimation(source, enabled)
                            }
                          />
                        </label>
                        <ActionButton
                          variant="outline"
                          size="sm"
                          disabled={actions.busy}
                          aria-busy={isRefreshing}
                          onClick={() => onDiscover(source)}
                        >
                          <RefreshCw
                            size={13}
                            className={isRefreshing ? "is-spin" : ""}
                            aria-hidden
                          />
                          {isRefreshing ? "正在刷新模型…" : "刷新模型"}
                        </ActionButton>
                        <ActionButton
                          variant="outline"
                          size="sm"
                          disabled={actions.busy}
                          onClick={() => onEdit(source)}
                        >
                          <Settings2 size={13} aria-hidden />
                          {source.codex || source.copilot ? "账号" : "编辑"}
                        </ActionButton>
                        {source.codex && onCodexReset ? (
                          <ActionButton
                            variant="ghost"
                            size="icon"
                            disabled={actions.busy}
                            aria-label="断开 Codex 订阅"
                            title="断开 Codex 订阅"
                            onClick={onCodexReset}
                          >
                            <Unlink size={14} />
                          </ActionButton>
                        ) : source.copilot && onCopilotReset ? (
                          <ActionButton
                            variant="ghost"
                            size="icon"
                            disabled={actions.busy}
                            aria-label="断开 Copilot"
                            title="断开 Copilot"
                            onClick={onCopilotReset}
                          >
                            <Unlink size={14} />
                          </ActionButton>
                        ) : (
                          <ActionButton
                            variant="ghost"
                            size="icon"
                            disabled={actions.busy}
                            aria-label={`删除来源 ${source.name}`}
                            title="删除来源"
                            onClick={() => onDelete(source)}
                          >
                            <Trash2 size={14} />
                          </ActionButton>
                        )}
                      </div>
                    </div>

                    {notice && (
                      <div
                        className={`mg-source-toast ${notice.error ? "is-error" : ""}`}
                        role={notice.error ? "alert" : "status"}
                      >
                        {!notice.error && (
                          <CheckCircle2 size={14} aria-hidden />
                        )}
                        <span>{notice.text}</span>
                        {notice.error && source.copilot && onCopilotLogin && (
                          <ActionButton
                            variant="outline"
                            size="sm"
                            disabled={actions.busy}
                            onClick={onCopilotLogin}
                          >
                            重新登录 GitHub Copilot
                          </ActionButton>
                        )}
                        {notice.error && source.codex && onCodexLogin && (
                          <ActionButton
                            variant="outline"
                            size="sm"
                            disabled={actions.busy}
                            onClick={onCodexLogin}
                          >
                            重新连接 Codex 订阅
                          </ActionButton>
                        )}
                      </div>
                    )}

                    <ModelCatalog
                      snapshot={snapshot}
                      view={{ ...view, providerId: source.id }}
                      onViewChange={onViewChange}
                      onAdd={onAdd}
                      onRefresh={onRefresh}
                      sourceName={source.name}
                      onCandidate={onCandidate}
                      onBatchCandidates={onBatchCandidates}
                      onOpenAgents={onOpenAgents}
                      {...actions}
                    />
                  </div>
                )}
              </section>
            );
          })}
        </div>
      )}

      {/* Tier 2: Progressive disclosure for unified cross-provider model pool */}
      {sources.length > 0 && (
        <div className="mg-disclosure-wrap">
          <button
            type="button"
            className="mg-disclosure-trigger"
            aria-expanded={allModelsVisible}
            onClick={toggleAllModelsView}
          >
            {allModelsVisible ? (
              <ChevronDown size={15} aria-hidden />
            ) : (
              <ChevronRight size={15} aria-hidden />
            )}
            <Layers3 size={14} aria-hidden />
            <span>查看全部可用模型（{activeGroupCount} 个）</span>
            <small>同名模型按 Provider 自动加前缀区分（如 codex/…、copilot/…）</small>
          </button>

          {allModelsVisible && (
            <div className="mg-disclosure-body">
              <ModelCatalog
                snapshot={snapshot}
                view={view}
                onViewChange={onViewChange}
                onAdd={onAdd}
                onRefresh={onRefresh}
                onCandidate={onCandidate}
                onBatchCandidates={onBatchCandidates}
                onOpenAgents={onOpenAgents}
                {...actions}
              />
            </div>
          )}
        </div>
      )}
    </section>
  );
}
