import {
  BarChart3,
  ChevronLeft,
  ChevronRight,
  Info,
  List,
  RefreshCw,
  Search,
  SlidersHorizontal,
  X,
} from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Input } from "@/components/ui/input";
import { ActionButton, Choice } from "./ui";
import { LogSummary } from "./LogSummary";
import { RequestLogTable } from "./RequestLogTable";
import { TokenUsagePanel } from "./TokenUsagePanel";
import { LogStorageRecovery } from "./LogStorageRecovery";
import type { DatePreset, HistoryPage, LogStorage } from "./log-history";
import type { UsageGrouping } from "./log-usage";
import "./log-studio.css";

export interface LogFilters {
  model: string;
  provider: string;
  caller: string;
  endpoint: string;
  status: string;
  search: string;
  preset: DatePreset;
  start: string;
  end: string;
}
type Option = { value: string; label: string };
export interface LogStudioModel {
  filters: LogFilters;
  providers: Option[];
  callers: Option[];
  endpoints: string[];
  codes: number[];
  result: HistoryPage | null;
  storage: LogStorage | null;
  loading: boolean;
  scopePending: boolean;
  error: boolean;
  dateError: string | null;
  recovering: boolean;
  recoveryError: string | null;
  gatewayRunning: boolean;
  showAnalysis: boolean;
  grouping: UsageGrouping;
  page: number;
  pageSize: number;
  tableKey: string;
  timezone: string;
}
export interface LogStudioActions {
  filter: <K extends keyof LogFilters>(key: K, value: LogFilters[K]) => void;
  clear: () => void;
  refresh: () => void;
  analysis: (value: boolean) => void;
  grouping: (value: UsageGrouping) => void;
  page: (page: number) => void;
  pageSize: (size: number) => void;
  retry: () => void;
  openDirectory: () => void;
}
const endpointName = (value: string) =>
  ({
    "/v1/responses": "Responses",
    "/v1/responses/compact": "Responses · compact",
    "/v1/chat/completions": "Chat",
    "/v1/messages": "Messages",
    "/v1/models": "模型目录",
  })[value] ?? value;
export function LogStudio({
  model: m,
  actions: a,
}: {
  model: LogStudioModel;
  actions: LogStudioActions;
}) {
  const { filters: f, result: r } = m;
  const pendingFilters = [
    f.endpoint !== "all",
    f.caller !== "all",
    f.status !== "all",
    !!f.search.trim(),
  ].filter(Boolean).length;
  const filtered =
    pendingFilters > 0 ||
    !!f.model.trim() ||
    f.provider !== "all" ||
    f.preset !== "all";
  const stateLabel = m.error
    ? "更新失败"
    : m.scopePending
      ? "查询中"
      : m.page > 0
        ? "已暂停"
        : "实时";
  const shownPage = r?.page ?? 0;
  const switchView = (value: boolean) => {
    a.analysis(value);
    document
      .getElementById(value ? "log-tab-analysis" : "log-tab-list")
      ?.focus();
  };
  return (
    <section className="mg-request-logs mg-log-studio">
      <header className="ls-heading">
        <div>
          <h1>调用日志</h1>
        </div>
        <div className="ls-heading-actions">
          <span
            className={`ls-live ${m.error ? "is-error" : ""}`}
            data-state={stateLabel}
          >
            <i aria-hidden />
            {stateLabel}
          </span>
          <Choice
            label="日志时间范围"
            value={f.preset}
            onChange={(v) => a.filter("preset", v as DatePreset)}
            options={[
              { value: "all", label: "全部时间" },
              { value: "today", label: "今天" },
              { value: "day", label: "最近 24 小时" },
              { value: "week", label: "近 7 天" },
              { value: "custom", label: "自定义" },
            ]}
          />
          <Popover>
            <PopoverTrigger asChild>
              <ActionButton
                variant="ghost"
                size="icon"
                aria-label="统计口径"
                title="统计口径"
              >
                <Info size={16} />
              </ActionButton>
            </PopoverTrigger>
            <PopoverContent className="ls-popover ls-about" align="end">
              <h2>统计口径</h2>
              <p>
                统计覆盖当前筛选的全部记录，不受分页影响。Provider
                匹配任一尝试；成功率按请求最终结果计算，不是单家来源的成功率。
              </p>
              <p>
                HTTP 200
                不代表模型完成。待确认和进行中的记录不计入成功率；缺失用量不按零计。
              </p>
              <p>
                日期按 {m.timezone}，包含结束当天。第一页每 3
                秒刷新；翻页时固定日期和新增记录边界。刷新返回最新第一页。
              </p>
              <p>数字简写仅用于展示，悬浮、键盘聚焦或展开可查看完整值。</p>
            </PopoverContent>
          </Popover>
          <ActionButton
            variant="outline"
            size="icon"
            className="ls-refresh"
            aria-label="刷新"
            title="刷新到最新"
            aria-disabled={m.loading || m.recovering}
            onClick={() => {
              if (!m.loading && !m.recovering) a.refresh();
            }}
          >
            <RefreshCw size={16} />
          </ActionButton>
        </div>
      </header>
      {m.storage && !m.storage.ready ? (
        <LogStorageRecovery
          storage={m.storage}
          running={m.gatewayRunning}
          busy={m.recovering}
          error={m.recoveryError}
          onRetry={a.retry}
          onOpen={a.openDirectory}
        />
      ) : (
        <>
          <div className="ls-tabs-row">
            <div
              className="ls-tabs"
              role="tablist"
              aria-label="日志视图"
              onKeyDown={(e) => {
                if (e.key === "ArrowLeft" || e.key === "Home") {
                  e.preventDefault();
                  switchView(false);
                } else if (e.key === "ArrowRight" || e.key === "End") {
                  e.preventDefault();
                  switchView(true);
                }
              }}
            >
              <button
                id="log-tab-list"
                role="tab"
                aria-selected={!m.showAnalysis}
                aria-controls="log-workspace-panel"
                tabIndex={!m.showAnalysis ? 0 : -1}
                onClick={() => a.analysis(false)}
              >
                <List size={15} aria-hidden />
                日志
              </button>
              <button
                id="log-tab-analysis"
                role="tab"
                aria-selected={m.showAnalysis}
                aria-controls="log-workspace-panel"
                tabIndex={m.showAnalysis ? 0 : -1}
                onClick={() => a.analysis(true)}
              >
                <BarChart3 size={15} aria-hidden />
                分析
              </button>
            </div>
          </div>
          {!m.showAnalysis && <LogSummary result={r} />}
          <div className="ls-toolbar">
            <div className="ls-search">
              <Search size={15} aria-hidden />
              <label htmlFor="history-model" className="sr-only">
                模型
              </label>
              <Input
                id="history-model"
                value={f.model}
                placeholder="搜索模型"
                onChange={(e) => a.filter("model", e.target.value)}
              />
            </div>
            <Choice
              label="按 Provider 筛选（含回退尝试）"
              value={f.provider}
              options={[{ value: "all", label: "全部来源" }, ...m.providers]}
              onChange={(v) => a.filter("provider", v)}
            />
            <Popover>
              <PopoverTrigger asChild>
                <ActionButton
                  variant="outline"
                  className="ls-more"
                  aria-label="更多筛选"
                >
                  <SlidersHorizontal size={15} aria-hidden />
                  筛选
                  {pendingFilters > 0 && (
                    <span className="ls-filter-count">{pendingFilters}</span>
                  )}
                </ActionButton>
              </PopoverTrigger>
              <PopoverContent className="ls-popover" align="end">
                <div className="ls-popover-heading">
                  <h2>更多筛选</h2>
                  {filtered && <button onClick={a.clear}>清除</button>}
                </div>
                <label htmlFor="history-search">请求 ID / 错误码</label>
                <Input
                  id="history-search"
                  value={f.search}
                  placeholder="ID、#序号或错误码"
                  onChange={(e) => a.filter("search", e.target.value)}
                />
                <label htmlFor="history-endpoint">接口</label>
                <Choice
                  id="history-endpoint"
                  label="按接口筛选"
                  value={f.endpoint}
                  onChange={(v) => a.filter("endpoint", v)}
                  options={[
                    { value: "all", label: "全部接口" },
                    ...m.endpoints.map((v) => ({
                      value: v,
                      label: endpointName(v),
                    })),
                  ]}
                />
                <label htmlFor="history-caller">密钥</label>
                <Choice
                  id="history-caller"
                  label="按访问密钥筛选"
                  value={f.caller}
                  onChange={(v) => a.filter("caller", v)}
                  options={[{ value: "all", label: "全部密钥" }, ...m.callers]}
                />
                <label htmlFor="history-outcome">结果</label>
                <Choice
                  id="history-outcome"
                  label="按调用结果或 HTTP 状态筛选"
                  value={f.status}
                  onChange={(v) => a.filter("status", v)}
                  options={[
                    { value: "all", label: "全部结果" },
                    { value: "success", label: "成功" },
                    { value: "failed", label: "失败" },
                    { value: "pending", label: "待确认" },
                    ...m.codes.map((n) => ({
                      value: String(n),
                      label: `HTTP ${n}`,
                    })),
                  ]}
                />
              </PopoverContent>
            </Popover>
            {filtered && (
              <ActionButton
                variant="ghost"
                size="icon"
                aria-label="清除筛选"
                title="清除筛选"
                onClick={a.clear}
              >
                <X size={15} />
              </ActionButton>
            )}
          </div>
          {f.preset === "custom" && (
            <div className="ls-dates">
              <label htmlFor="history-from">从</label>
              <Input
                id="history-from"
                aria-label="开始日期"
                type="date"
                value={f.start}
                onChange={(e) => a.filter("start", e.target.value)}
              />
              <label htmlFor="history-to">至</label>
              <Input
                id="history-to"
                aria-label="结束日期"
                type="date"
                value={f.end}
                onChange={(e) => a.filter("end", e.target.value)}
              />
              <span>{m.timezone}</span>
            </div>
          )}
          {pendingFilters > 0 && (
            <div className="ls-active-filters" aria-label="已启用筛选">
              {f.endpoint !== "all" && (
                <button onClick={() => a.filter("endpoint", "all")}>
                  {endpointName(f.endpoint)}
                  <X size={12} />
                </button>
              )}
              {f.caller !== "all" && (
                <button onClick={() => a.filter("caller", "all")}>
                  密钥 ·{" "}
                  {m.callers.find((c) => c.value === f.caller)?.label ??
                    f.caller}
                  <X size={12} />
                </button>
              )}
              {f.status !== "all" && (
                <button onClick={() => a.filter("status", "all")}>
                  {{ success: "成功", failed: "失败", pending: "待确认" }[
                    f.status
                  ] ?? `HTTP ${f.status}`}
                  <X size={12} />
                </button>
              )}
              {f.search && (
                <button onClick={() => a.filter("search", "")}>
                  {f.search}
                  <X size={12} />
                </button>
              )}
            </div>
          )}
          {m.dateError && (
            <p role="alert" className="ls-alert">
              {m.dateError} · 仍显示上次结果
            </p>
          )}
          {m.error && (
            <p role="alert" className="ls-alert">
              更新失败，保留上次数据。请重试或检查日志存储。
            </p>
          )}
          <div
            id="log-workspace-panel"
            role="tabpanel"
            aria-labelledby={
              m.showAnalysis ? "log-tab-analysis" : "log-tab-list"
            }
          >
            {m.showAnalysis ? (
              r ? (
                <TokenUsagePanel
                  rows={[]}
                  loaded
                  summary={{
                    analytics: r.analytics,
                    cache: r.cache,
                    overflow: r.overflow,
                  }}
                  grouping={m.grouping}
                  onGrouping={a.grouping}
                />
              ) : (
                <div className="ls-empty">正在读取统计…</div>
              )
            ) : (
              <>
                <div className="ls-list-meta">
                  <span role="status">
                    {r
                      ? `${r.matched ? shownPage * r.pageSize + 1 : 0}–${shownPage * r.pageSize + r.rows.length} / ${r.matched.toLocaleString()} 条`
                      : "正在读取记录…"}
                  </span>
                  <span>
                    {m.page > 0
                      ? "分页已固定"
                      : m.scopePending
                        ? "筛选中 · 暂显示上次结果"
                        : "最新记录"}
                  </span>
                </div>
                {r && r.rows.length > 0 && (
                  <RequestLogTable key={m.tableKey} rows={r.rows} />
                )}
                {r && !r.rows.length && (
                  <div className="ls-empty">
                    <h2>没有匹配记录</h2>
                    <p>
                      {filtered
                        ? "调整筛选，或查看全部记录。"
                        : "请求网关后，记录会出现在这里。"}
                    </p>
                    {filtered && (
                      <ActionButton variant="outline" onClick={a.clear}>
                        清除筛选
                      </ActionButton>
                    )}
                  </div>
                )}
                {r && (
                  <div className="ls-pagination" aria-label="日志分页">
                    <Choice
                      label="每页记录数"
                      value={String(m.pageSize)}
                      onChange={(v) => a.pageSize(Number(v))}
                      options={[25, 50, 100].map((n) => ({
                        value: String(n),
                        label: `${n} 条 / 页`,
                      }))}
                    />
                    <div>
                      <span>
                        {shownPage + 1} / {Math.max(1, r.pages)}
                      </span>
                      <ActionButton
                        variant="outline"
                        size="icon"
                        aria-label="上一页"
                        disabled={m.scopePending || shownPage === 0}
                        onClick={() => a.page(shownPage - 1)}
                      >
                        <ChevronLeft size={16} />
                      </ActionButton>
                      <ActionButton
                        variant="outline"
                        size="icon"
                        aria-label="下一页"
                        disabled={m.scopePending || shownPage + 1 >= r.pages}
                        onClick={() => a.page(shownPage + 1)}
                      >
                        <ChevronRight size={16} />
                      </ActionButton>
                    </div>
                  </div>
                )}
              </>
            )}
          </div>
        </>
      )}
    </section>
  );
}
