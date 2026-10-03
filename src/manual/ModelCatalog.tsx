import { useEffect, useLayoutEffect, useRef } from "react";
import {
  AlertCircle,
  CheckCircle2,
  ChevronRight,
  CircleHelp,
  EyeOff,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { compactTokens } from "./log-usage";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ActionButton, Choice } from "./ui";
import { ModelDetail, type ModelActions } from "./ModelDetail";
import {
  capabilitySummary,
  catalogEntries,
  initialCatalogView,
  resultLabel,
  sourceResults,
  type CatalogStatus,
  type CatalogView,
} from "./catalog-view";
import { protocolLabels, type Snapshot } from "./types";

export function ModelCatalog({
  snapshot,
  view,
  onViewChange,
  onAdd,
  onProviders,
  onRefresh,
  ...actions
}: ModelActions & {
  snapshot: Snapshot;
  view: CatalogView;
  onViewChange: (view: CatalogView) => void;
  onAdd: () => void;
  onProviders: () => void;
  onRefresh: () => void;
}) {
  const returnTarget = useRef<HTMLButtonElement | null>(null);
  const closeRequested = useRef(false);
  const { active, counts, matches, blocked } = catalogEntries(snapshot, view);
  const selected =
    view.status !== "blocked"
      ? active.find((group) => group.id === view.selectedId)
      : undefined;
  const container = useRef<HTMLElement>(null);
  const previousSelection = useRef<string | undefined>();
  useLayoutEffect(() => {
    if (closeRequested.current && !selected) {
      closeRequested.current = false;
      const target = returnTarget.current?.isConnected
        ? returnTarget.current
        : container.current?.querySelector<HTMLButtonElement>(
            '[role="tab"][aria-selected="true"]',
          );
      target?.focus({ preventScroll: true });
    }
  }, [selected?.id]);
  useEffect(() => {
    if (
      previousSelection.current &&
      !selected &&
      document.activeElement === document.body
    ) {
      container.current
        ?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')
        ?.focus();
    }
    previousSelection.current = selected?.id;
  }, [selected?.id]);
  const sources = snapshot.document.providers;
  const hasFilters =
    !!view.query || view.protocol !== "all" || view.providerId !== "all";
  function update(patch: Partial<CatalogView>) {
    onViewChange({ ...view, selectedId: null, ...patch });
  }
  const categories: { value: CatalogStatus; label: string }[] = [
    { value: "all", label: "全部模型" },
    { value: "failed", label: "待检查" },
    { value: "untested", label: "含未测试" },
    { value: "blocked", label: "已屏蔽" },
  ];
  const emptyTitle = !sources.length
    ? "接入你的第一个模型来源"
    : !sources.some((source) => source.models.length > 0)
      ? "来源已保存，下一步获取模型"
      : "还没有启用的模型";
  return (
    <section
      className="mg-catalog"
      aria-label="模型工作区"
      ref={container}
      data-detail-open={!!selected}
    >
      <div className="mg-page-heading">
        <div>
          <h1>模型</h1>
          <p>
            {active.length} 个模型 ·{" "}
            {active.reduce(
              (count, group) => count + group.providerIds.length,
              0,
            )}{" "}
            个来源部署
            <span className="mg-heading-note">
              选择模型，查看来源与测试结果
            </span>
          </p>
        </div>
        <div className="mg-heading-actions">
          <ActionButton
            variant="outline"
            size="sm"
            disabled={actions.busy}
            onClick={onRefresh}
            aria-label="刷新状态"
          >
            <RefreshCw size={14} aria-hidden />
            刷新
          </ActionButton>
          <ActionButton
            variant="outline"
            size="sm"
            disabled={actions.busy}
            onClick={onAdd}
          >
            <Plus size={14} aria-hidden />
            接入来源
          </ActionButton>
        </div>
      </div>
      <Tabs
        className="mg-catalog-tabs"
        value={view.status}
        onValueChange={(status) =>
          update({
            status: status as CatalogStatus,
            providerId: status === "blocked" ? "all" : view.providerId,
            protocol: status === "blocked" ? "all" : view.protocol,
          })
        }
      >
        <TabsList className="mg-view-tabs" aria-label="模型状态筛选">
          {categories.map(({ value, label }) => (
            <TabsTrigger key={value} className="mg-view-tab" value={value}>
              {label}
              <span className="mg-tab-count">{counts[value]}</span>
            </TabsTrigger>
          ))}
        </TabsList>
        <div className="mg-catalog-toolbar">
          <div className="mg-search">
            <Search size={16} aria-hidden />
            <Input
              aria-label="搜索模型"
              type="search"
              placeholder="搜索模型名称或 ID…"
              value={view.query}
              onChange={(event) => update({ query: event.target.value })}
            />
          </div>
          {view.status !== "blocked" && (
            <>
              <Choice
                label="筛选模型来源"
                value={view.providerId}
                onChange={(providerId) => update({ providerId })}
                options={[
                  { value: "all", label: "全部来源" },
                  ...sources.map((source) => ({
                    value: source.id,
                    label: source.name,
                  })),
                ]}
              />
              <Choice
                label="筛选 Endpoint"
                value={view.protocol}
                onChange={(protocol) => update({ protocol })}
                options={[
                  { value: "all", label: "全部 Endpoint" },
                  ...Object.entries(protocolLabels).map(([value, label]) => ({
                    value,
                    label,
                  })),
                ]}
              />
            </>
          )}
        </div>
        <div className="mg-catalog-context">
          <span role="status">
            显示 {view.status === "blocked" ? blocked.length : matches.length}{" "}
            个{view.status === "blocked" ? "已屏蔽模型" : "模型"}
          </span>
          {view.status === "failed" && (
            <span>含失败记录或路由错误，不代表所有来源都不可用</span>
          )}
          {hasFilters && (
            <ActionButton
              variant="ghost"
              size="sm"
              onClick={() =>
                onViewChange({ ...initialCatalogView, status: view.status })
              }
            >
              清除筛选
            </ActionButton>
          )}
        </div>
        <TabsContent
          value={view.status}
          className="mg-catalog-content"
          tabIndex={-1}
        >
          <div className={`mg-model-workbench ${selected ? "has-detail" : ""}`}>
            <div className="mg-model-browser">
              {view.status === "blocked" ? (
                <>
                  <div className="mg-list-heading">
                    <span>已屏蔽模型</span>
                    <span>已退出路由与同步列表</span>
                  </div>
                  <ul
                    className="mg-model-scroll mg-blocked-list"
                    aria-label="已屏蔽模型"
                  >
                    {blocked.map((id) => (
                      <li key={id}>
                        <span>
                          <EyeOff size={15} aria-hidden />
                          {id}
                        </span>
                        <ActionButton
                          size="sm"
                          variant="outline"
                          disabled={actions.busy}
                          aria-label={`恢复 ${id}`}
                          onClick={() => actions.onBlock(id, false)}
                        >
                          <RotateCcw size={13} aria-hidden />
                          恢复
                        </ActionButton>
                      </li>
                    ))}
                  </ul>
                  {!blocked.length && (
                    <div className="mg-catalog-empty">
                      <h2>
                        {view.query ? "没有匹配的已屏蔽模型" : "没有已屏蔽模型"}
                      </h2>
                      <p>屏蔽的模型会保留在这里，可随时恢复。</p>
                    </div>
                  )}
                </>
              ) : active.length === 0 ? (
                <div className="mg-catalog-empty">
                  <h2>
                    {counts.blocked ? "模型已全部屏蔽或停用" : emptyTitle}
                  </h2>
                  <p>
                    {!sources.length
                      ? "填写 API 地址和密钥，获取目录后选择要使用的模型。"
                      : "目录获取、模型启用与推理测试是独立步骤，不会自动发起测试。"}
                  </p>
                  <ActionButton
                    disabled={actions.busy}
                    onClick={
                      counts.blocked
                        ? () => update({ status: "blocked" })
                        : !sources.length
                          ? onAdd
                          : onProviders
                    }
                  >
                    {counts.blocked
                      ? "查看已屏蔽模型"
                      : !sources.length
                        ? "添加 Provider"
                        : "管理来源与模型"}
                  </ActionButton>
                </div>
              ) : (
                <>
                  <div className="mg-list-heading mg-model-columns" aria-hidden>
                    <span>模型</span>
                    <span>来源</span>
                    <span>上下文</span>
                    <span>最大输出</span>
                    <span>路由</span>
                    <span>上次测试</span>
                    <span>启用</span>
                  </div>
                  <ul
                    className="mg-model-scroll mg-model-records"
                    aria-label="模型列表"
                  >
                    {matches.map((group) => {
                      const result = sourceResults(
                        group,
                        snapshot.document.tests,
                      );
                      const issue = !!group.routingError || result.failed > 0;
                      const Icon = issue
                        ? AlertCircle
                        : result.untested
                          ? CircleHelp
                          : CheckCircle2;
                      return (
                        <li key={group.id} className="mc-model-list-row">
                          <button
                            className="mg-model-record mg-model-columns"
                            type="button"
                            data-selected={selected?.id === group.id}
                            aria-label={`查看模型 ${group.publicId ?? group.model.id}`}
                            aria-expanded={selected?.id === group.id}
                            aria-controls={
                              selected?.id === group.id
                                ? "manual-model-detail"
                                : undefined
                            }
                            onClick={(event) => {
                              returnTarget.current = event.currentTarget;
                              onViewChange({ ...view, selectedId: group.id });
                            }}
                          >
                            <span className="mg-record-name">
                              <strong>
                                {group.publicId ?? group.model.id}
                              </strong>
                              <small>{capabilitySummary(group.model)}</small>
                            </span>
                            <span className="mg-record-sources">
                              <strong>{group.providerIds.length} 个来源</strong>
                              <small>{group.providerNames.join("、")}</small>
                            </span>
                            <span
                              className="mc-model-limit"
                              title={
                                group.model.contextWindow?.toLocaleString() ??
                                "未知"
                              }
                            >
                              {group.model.contextWindow == null
                                ? "—"
                                : compactTokens(group.model.contextWindow)}
                              {group.model.limitsSource === "user_override" && (
                                <sup title="用户指定限额">*</sup>
                              )}
                            </span>
                            <span
                              className="mc-model-limit"
                              title={
                                group.model.maxOutputTokens?.toLocaleString() ??
                                "未知"
                              }
                            >
                              {group.model.maxOutputTokens == null
                                ? "—"
                                : compactTokens(group.model.maxOutputTokens)}
                            </span>
                            <span className="mc-model-route">
                              {group.policy.balance === "priority"
                                ? "优先顺序"
                                : "轮询"}
                              {group.policy.fallbacks.length > 0 && (
                                <small>
                                  {group.policy.fallbacks.length} 个后备
                                </small>
                              )}
                            </span>
                            <span
                              className={`mg-record-result ${issue ? "is-failed" : !result.untested ? "is-passed" : ""}`}
                            >
                              <Icon size={14} aria-hidden />
                              <span>
                                {group.routingError
                                  ? "路由待检查"
                                  : resultLabel(group, snapshot.document.tests)}
                              </span>
                            </span>
                            <ChevronRight
                              size={15}
                              aria-hidden
                              className="mg-record-chevron"
                            />
                          </button>
                          <Switch
                            checked
                            disabled={actions.busy}
                            aria-label={`启用模型 ${group.model.id}`}
                            onCheckedChange={() =>
                              actions.onBlock(group.model.id, true)
                            }
                          />
                        </li>
                      );
                    })}
                  </ul>
                  {!matches.length && (
                    <div className="mg-catalog-empty">
                      <h2>没有符合条件的模型</h2>
                      <p>
                        {view.query
                          ? `未找到“${view.query}”。试试其他名称，或清除筛选。`
                          : "当前状态或来源下没有模型，可返回全部模型。"}
                      </p>
                      <ActionButton
                        variant="outline"
                        onClick={() => onViewChange({ ...initialCatalogView })}
                      >
                        显示全部模型
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
        </TabsContent>
      </Tabs>
    </section>
  );
}
