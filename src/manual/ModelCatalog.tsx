import { useEffect, useLayoutEffect, useRef } from "react";
import {
  AlertCircle,
  CheckCircle2,
  RefreshCw,
  Search,
  Sparkles,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { compactTokens } from "./log-usage";
import { ActionButton, Choice } from "./ui";
import { ModelDetail, type ModelActions } from "./ModelDetail";
import {
  catalogEntries,
  initialCatalogView,
  resultLabel,
  scopedSourceModels,
  sourceResults,
  type CatalogStatus,
  type CatalogView,
} from "./catalog-view";
import {
  displayModel,
  groupModelIds,
  protocolLabels,
  protocolShortLabels,
  stripNamespace,
  type Protocol,
  type Snapshot,
} from "./types";

export function ModelCatalog({
  snapshot,
  view,
  onViewChange,
  onAdd,
  onRefresh,
  sourceName,
  onCandidate,
  onBatchCandidates,
  onOpenAgents,
  ...actions
}: ModelActions & {
  snapshot: Snapshot;
  view: CatalogView;
  onViewChange: (view: CatalogView) => void;
  onAdd: () => void;
  onRefresh: () => void;
  sourceName?: string;
  onCandidate?: (modelId: string, providerId: string, enabled: boolean) => void;
  onBatchCandidates?: (
    providerId: string,
    enabled: boolean,
    modelIds?: string[],
  ) => void;
  onOpenAgents?: () => void;
}) {
  const returnTarget = useRef<HTMLButtonElement | null>(null);
  const closeRequested = useRef(false);
  const sources = snapshot.document.providers;
  const scopedSource =
    view.providerId === "all"
      ? undefined
      : sources.find((source) => source.id === view.providerId);
  const scoped = !!scopedSource;

  const globalView = catalogEntries(snapshot, view);
  const scopedView = scopedSource
    ? scopedSourceModels(scopedSource, snapshot, view)
    : null;

  const selected =
    view.status !== "disabled"
      ? globalView.active.find((group) => group.id === view.selectedId)
      : undefined;

  const container = useRef<HTMLElement>(null);
  const previousSelection = useRef<string | undefined>();
  useLayoutEffect(() => {
    if (closeRequested.current && !selected) {
      closeRequested.current = false;
      returnTarget.current?.focus({ preventScroll: true });
    }
  }, [selected?.id]);
  useEffect(() => {
    previousSelection.current = selected?.id;
  }, [selected?.id]);

  function update(patch: Partial<CatalogView>) {
    onViewChange({ ...view, selectedId: null, ...patch });
  }

  const tabs: { value: CatalogStatus; label: string; count: number }[] = scoped
    ? [
        { value: "all", label: "全部", count: scopedView!.counts.all },
        { value: "enabled", label: "已启用", count: scopedView!.counts.enabled },
        {
          value: "disabled",
          label: "未启用",
          count: scopedView!.counts.disabled,
        },
        ...(scopedView!.counts.failed > 0
          ? [
              {
                value: "failed" as const,
                label: "异常",
                count: scopedView!.counts.failed,
              },
            ]
          : []),
      ]
    : [
        { value: "all", label: "可用模型", count: globalView.counts.all },
        {
          value: "disabled",
          label: "已停用",
          count: globalView.counts.disabled,
        },
        ...(globalView.counts.failed > 0
          ? [
              {
                value: "failed" as const,
                label: "异常",
                count: globalView.counts.failed,
              },
            ]
          : []),
      ];

  const codexAutoSynced =
    !!snapshot.agents?.codex.connected && !!snapshot.agents?.codex.autoSync;

  const fmtLimits = (ctx: number | null, out?: number | null) => {
    if (ctx == null && out == null) return "—";
    const c = ctx == null ? "—" : compactTokens(ctx);
    const o = out == null ? "" : ` / ${compactTokens(out)}`;
    return `${c}${o}`;
  };

  return (
    <section
      className="mg-catalog"
      aria-label="模型列表"
      ref={container}
      data-detail-open={!!selected}
    >
      <div className="mg-catalog-bar">
        <div className="mg-segmented" role="tablist" aria-label="模型筛选">
          {tabs.map((tab) => (
            <button
              key={tab.value}
              type="button"
              role="tab"
              aria-selected={view.status === tab.value}
              className={`mg-seg-item ${view.status === tab.value ? "is-active" : ""}`}
              onClick={() => update({ status: tab.value })}
            >
              <span>{tab.label}</span>
              <em>{tab.count}</em>
            </button>
          ))}
        </div>

        <div className="mg-catalog-controls">
          <div className="mg-search">
            <Search size={14} aria-hidden />
            <Input
              aria-label="搜索模型"
              type="search"
              placeholder="搜索模型…"
              value={view.query}
              onChange={(event) => update({ query: event.target.value })}
            />
          </div>

          {(scoped || view.status !== "disabled") && (
            <Choice
              label="筛选协议"
              value={view.protocol}
              onChange={(protocol) => update({ protocol })}
              options={[
                { value: "all", label: "全部协议" },
                ...Object.entries(protocolLabels).map(([value, label]) => ({
                  value,
                  label,
                })),
              ]}
            />
          )}

          {scoped && scopedSource && onBatchCandidates && scopedSource.models.length > 0 && (() => {
            const filtered =
              !!view.query.trim() ||
              view.protocol !== "all" ||
              view.status !== "all";
            const targetIds = filtered
              ? scopedView!.matches.map((row) => row.model.id)
              : undefined;
            const canEnable = filtered
              ? scopedView!.matches.some((row) => !row.model.enabled)
              : scopedView!.counts.disabled > 0;
            const canDisable = filtered
              ? scopedView!.matches.some((row) => row.model.enabled)
              : scopedView!.counts.enabled > 0;
            return (
              <div className="mg-batch-btns">
                <ActionButton
                  variant="ghost"
                  size="sm"
                  disabled={actions.busy || !canEnable}
                  onClick={() =>
                    onBatchCandidates(scopedSource.id, true, targetIds)
                  }
                >
                  全选
                </ActionButton>
                <ActionButton
                  variant="ghost"
                  size="sm"
                  disabled={actions.busy || !canDisable}
                  onClick={() =>
                    onBatchCandidates(scopedSource.id, false, targetIds)
                  }
                >
                  全不选
                </ActionButton>
              </div>
            );
          })()}

          {codexAutoSynced && onOpenAgents && (
            <button
              type="button"
              className="mg-sync-pill"
              onClick={onOpenAgents}
              title="启停模型时会自动同步到本机 Codex"
            >
              <Sparkles size={12} aria-hidden />
              Codex 自动同步
            </button>
          )}
        </div>
      </div>

      <div className={`mg-model-workbench ${selected ? "has-detail" : ""}`}>
        <div className="mg-model-browser">
          {scoped && scopedSource ? (
            scopedSource.models.length === 0 ? (
              <div className="mg-catalog-empty">
                <h2>还没有拉取模型</h2>
                <p>点击上方「刷新模型」获取该来源的可用模型清单。</p>
              </div>
            ) : (
              <>
                <div className="mg-list-heading mg-model-columns mg-model-columns--scoped">
                  <span>模型</span>
                  <span>上下文 / 输出</span>
                  <span>协议</span>
                  <span>状态</span>
                  <span className="mg-switch-col">可用</span>
                </div>
                <ul className="mg-model-scroll mg-model-records" aria-label="来源模型列表">
                  {scopedView!.matches.map((row) => {
                    const group = row.group;
                    const res = group
                      ? sourceResults(group, snapshot.document.tests)
                      : null;
                    const failed =
                      !!group?.routingError || (res?.failed ?? 0) > 0;
                    const passed = !failed && (res?.passed ?? 0) > 0;
                    return (
                      <li
                        key={row.model.id}
                        className={`mc-model-list-row is-scoped ${!row.model.enabled ? "is-off" : ""}`}
                      >
                        <button
                          className="mg-model-record mg-model-columns mg-model-columns--scoped"
                          type="button"
                          disabled={!group}
                          data-selected={!!group && selected?.id === group.id}
                          aria-label={`查看模型 ${row.displayName}`}
                          onClick={(event) => {
                            if (!group) return;
                            returnTarget.current = event.currentTarget;
                            onViewChange({ ...view, selectedId: group.id });
                          }}
                        >
                          <span className="mg-record-name">
                            <strong>{row.displayName}</strong>
                            {row.model.name &&
                              row.model.name !== row.displayName && (
                                <small>{row.model.name}</small>
                              )}
                          </span>
                          <span className="mc-model-limit">
                            {fmtLimits(
                              row.model.contextWindow,
                              row.model.maxOutputTokens,
                            )}
                          </span>
                          <span className="mc-model-route">
                            <span className="mg-proto-tag">
                              {protocolShortLabels[row.protocol as Protocol] ??
                                row.protocol}
                            </span>
                          </span>
                          <span
                            className={`mg-record-result ${failed ? "is-failed" : passed ? "is-passed" : ""}`}
                          >
                            {failed ? (
                              <>
                                <AlertCircle size={13} aria-hidden />
                                <span>失败</span>
                              </>
                            ) : passed ? (
                              <>
                                <CheckCircle2 size={13} aria-hidden />
                                <span>正常</span>
                              </>
                            ) : (
                              <span className="mg-muted-text">
                                {row.model.enabled ? "就绪" : "已关闭"}
                              </span>
                            )}
                          </span>
                        </button>
                        <div className="mg-row-switch">
                          <Switch
                            checked={row.model.enabled}
                            disabled={actions.busy || !onCandidate}
                            aria-label={`启用模型 ${row.displayName} (${sourceName ?? scopedSource.name})`}
                            onCheckedChange={(next) =>
                              onCandidate?.(row.model.id, scopedSource.id, next)
                            }
                          />
                        </div>
                      </li>
                    );
                  })}
                </ul>
                {scopedView!.matches.length === 0 && (
                  <div className="mg-catalog-empty">
                    <h2>没有匹配的模型</h2>
                    <ActionButton
                      variant="outline"
                      size="sm"
                      onClick={() => update({ query: "", protocol: "all", status: "all" })}
                    >
                      重置筛选
                    </ActionButton>
                  </div>
                )}
              </>
            )
          ) : view.status === "disabled" ? (
            <>
              <div className="mg-list-heading mg-model-columns mg-model-columns--blocked">
                <span>已停用模型</span>
                <span className="mg-switch-col">可用</span>
              </div>
              <ul
                className="mg-model-scroll mg-model-records"
                aria-label="已停用模型"
              >
                {globalView.blocked.map((id) => (
                  <li key={id} className="mc-model-list-row is-blocked is-off">
                    <div className="mg-blocked-row-main">
                      <strong>{stripNamespace(id)}</strong>
                      {id !== stripNamespace(id) && <small>{id}</small>}
                    </div>
                    <div className="mg-row-switch">
                      <Switch
                        checked={false}
                        disabled={actions.busy}
                        aria-label={`恢复模型 ${stripNamespace(id)}`}
                        onCheckedChange={() => actions.onBlock([id], false)}
                      />
                    </div>
                  </li>
                ))}
              </ul>
              {globalView.blocked.length === 0 && (
                <div className="mg-catalog-empty">
                  <h2>没有已停用的模型</h2>
                  <p>在「可用模型」中关闭任意开关即可将其移入这里。</p>
                </div>
              )}
            </>
          ) : globalView.active.length === 0 ? (
            <div className="mg-catalog-empty">
              <h2>
                {!sources.length
                  ? "还没有模型来源"
                  : !sources.some((s) => s.models.length > 0)
                    ? "请先拉取模型目录"
                    : "没有可用的模型"}
              </h2>
              <p>
                {!sources.length
                  ? "从左侧连接 Codex 订阅、GitHub Copilot，或添加 API 来源。"
                  : "选择左侧来源并刷新模型，或在「已停用」中重新开启。"}
              </p>
              <ActionButton
                disabled={actions.busy}
                onClick={!sources.length ? onAdd : onRefresh}
              >
                {!sources.length ? (
                  "添加 API 来源"
                ) : (
                  <>
                    <RefreshCw size={14} aria-hidden />
                    刷新状态
                  </>
                )}
              </ActionButton>
            </div>
          ) : (
            <>
              <div className="mg-list-heading mg-model-columns">
                <span>模型</span>
                <span>来源</span>
                <span>上下文 / 输出</span>
                <span>协议</span>
                <span>状态</span>
                <span className="mg-switch-col">可用</span>
              </div>
              <ul
                className="mg-model-scroll mg-model-records"
                aria-label="模型列表"
              >
                {globalView.matches.map((group) => {
                  const res = sourceResults(group, snapshot.document.tests);
                  const failed = !!group.routingError || res.failed > 0;
                  const passed = !failed && res.passed > 0;
                  return (
                    <li key={group.id} className="mc-model-list-row">
                      <button
                        className="mg-model-record mg-model-columns"
                        type="button"
                        data-selected={selected?.id === group.id}
                        aria-label={`查看模型 ${displayModel(group)}`}
                        aria-expanded={selected?.id === group.id}
                        onClick={(event) => {
                          returnTarget.current = event.currentTarget;
                          onViewChange({ ...view, selectedId: group.id });
                        }}
                      >
                        <span className="mg-record-name">
                          <strong>{displayModel(group)}</strong>
                          {group.model.name &&
                            group.model.name !== displayModel(group) && (
                              <small>{group.model.name}</small>
                            )}
                        </span>
                        <span className="mg-record-sources">
                          <span className="mg-source-chips">
                            {group.providerNames.slice(0, 2).join(" · ")}
                            {group.providerNames.length > 2
                              ? ` +${group.providerNames.length - 2}`
                              : ""}
                          </span>
                        </span>
                        <span
                          className="mc-model-limit"
                          title={
                            group.model.contextWindow?.toLocaleString() ??
                            "未知"
                          }
                        >
                          {fmtLimits(
                            group.model.contextWindow,
                            group.model.maxOutputTokens,
                          )}
                          {group.model.limitsSource === "user_override" && (
                            <sup title="用户指定限额">*</sup>
                          )}
                        </span>
                        <span className="mc-model-route">
                          <span className="mg-proto-tag">
                            {protocolShortLabels[group.protocol]}
                          </span>
                        </span>
                        <span
                          className={`mg-record-result ${failed ? "is-failed" : passed ? "is-passed" : ""}`}
                        >
                          {failed ? (
                            <>
                              <AlertCircle size={13} aria-hidden />
                              <span>
                                {group.routingError
                                  ? "路由冲突"
                                  : resultLabel(group, snapshot.document.tests)}
                              </span>
                            </>
                          ) : passed ? (
                            <>
                              <CheckCircle2 size={13} aria-hidden />
                              <span>
                                {resultLabel(group, snapshot.document.tests)}
                              </span>
                            </>
                          ) : (
                            <span className="mg-muted-text">就绪</span>
                          )}
                        </span>
                      </button>
                      <div className="mg-row-switch">
                        <Switch
                          checked={true}
                          disabled={actions.busy}
                          aria-label={`停用模型 ${displayModel(group)}`}
                          onCheckedChange={(next) => {
                            if (!next) {
                              actions.onBlock(groupModelIds(group), true);
                            }
                          }}
                        />
                      </div>
                    </li>
                  );
                })}
              </ul>
              {globalView.matches.length === 0 && (
                <div className="mg-catalog-empty">
                  <h2>没有符合条件的模型</h2>
                  <ActionButton
                    variant="outline"
                    size="sm"
                    onClick={() => onViewChange({ ...initialCatalogView })}
                  >
                    清除筛选
                  </ActionButton>
                </div>
              )}
            </>
          )}
        </div>

        {selected && (
          <ModelDetail
            key={selected.id}
            group={selected}
            snapshot={snapshot}
            {...actions}
            onClose={() => {
              closeRequested.current = true;
              onViewChange({ ...view, selectedId: null });
            }}
          />
        )}
      </div>
    </section>
  );
}
