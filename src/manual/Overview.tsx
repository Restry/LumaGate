import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  ChevronDown,
  ChevronRight,
  Info,
  Layers3,
  RefreshCw,
  SquareTerminal,
} from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ActionButton, Choice } from "./ui";
import { RollingNumber } from "./RollingNumber";
import { DefaultCost } from "./DefaultCost";
import { compactTokens } from "./log-usage";
import {
  displayHistory,
  historyDateBounds,
  type HistoryPage,
  type HistoryReply,
} from "./log-history";
import type { LogFilters } from "./LogStudio";
import { isBlocked, type Snapshot } from "./types";

type Props = {
  snapshot: Snapshot;
  onLogs: (filters?: Partial<LogFilters>) => void;
  onProviders: () => void;
  onAgents?: () => void;
};

export function Overview({ snapshot, onLogs, onProviders, onAgents }: Props) {
  const [period, setPeriod] = useState<"day" | "week" | "all">("day");
  const [metric, setMetric] = useState<"requests" | "tokens">("requests");
  const [refresh, setRefresh] = useState(0);
  const [result, setResult] = useState<HistoryPage | null>(null);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(true);
  const [storageError, setStorageError] = useState<string | null>(null);
  const [updated, setUpdated] = useState<Date | null>(null);
  const [loadedPeriod, setLoadedPeriod] = useState(period);
  const [showDetails, setShowDetails] = useState(false);

  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      setLoading(true);
      try {
        const reply = await invoke<HistoryReply>("manual_query_logs", {
          query: {
            ...historyDateBounds(period, "", ""),
            model: "",
            provider: "all",
            caller: "all",
            endpoint: "all",
            status: "all",
            search: "",
            page: 0,
            pageSize: 25,
            grouping: "model",
          },
        });
        if (!active) return;
        if (!reply.storage.ready) {
          setStorageError(reply.storage.message);
          return;
        }
        if (!("rows" in reply)) throw Error("Missing history page");
        setResult(displayHistory(reply));
        setLoadedPeriod(period);
        setError(false);
        setStorageError(null);
        setUpdated(new Date());
      } catch {
        if (active) setError(true);
      } finally {
        if (active) {
          setLoading(false);
          timer = setTimeout(read, 10000);
        }
      }
    };
    void read();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [period, refresh, snapshot.document.revision]);

  const stats = result?.stats;
  const usage = result?.analytics;
  const rate =
    stats && stats.success + stats.failed
      ? stats.success / (stats.success + stats.failed)
      : null;

  const activeProviders = snapshot.document.providers.filter((p) => p.enabled).length;
  const totalProviders = snapshot.document.providers.length;
  const activeModels = snapshot.groups.filter((g) => !isBlocked(g, snapshot.document)).length;
  const agents = snapshot.agents;
  const agentEntries = agents
    ? [
        agents.codex,
        agents.claude,
        agents.pi,
        agents.opencode,
        agents.openclaw,
        agents.hermes,
        agents.droid,
      ].filter(Boolean)
    : [];
  const connectedAgents = agentEntries.filter((a) => a?.connected).length;

  const chart =
    metric === "requests"
      ? (result?.requestTimeline ?? []).map((p) => ({
          ...p,
          label: new Date(p.at).toLocaleString("zh-CN", {
            month: "numeric",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
            hour12: false,
          }),
        }))
      : (usage?.timeline ?? []).map((p) => ({
          label: p.label,
          tokens: p.total,
        }));

  const providers = [...snapshot.document.providers]
    .sort(
      (a, b) =>
        (result?.providerObservations?.find((p) => p.id === b.id)?.records[0]?.at ?? 0) -
        (result?.providerObservations?.find((p) => p.id === a.id)?.records[0]?.at ?? 0),
    )
    .slice(0, 4);

  const pending = loadedPeriod !== period;
  const open = (filters: Partial<LogFilters> = {}) =>
    onLogs({ preset: loadedPeriod, ...filters });

  return (
    <section className="mc-overview mg-stage" aria-label="运行概览">
      <header className="mg-stage-header">
        <div>
          <h1>运行概览</h1>
          <p className="mg-stage-subtitle">
            {totalProviders === 0
              ? "先添加可用的 Provider，再接入本地 Agent"
              : `${activeProviders} 个 Provider 启用 · ${activeModels} 个可用模型 · ${connectedAgents} 个 Agent 已接入`}
          </p>
        </div>
        <div className="mg-heading-actions">
          <Choice
            label="概览时间范围"
            value={period}
            onChange={(v) => setPeriod(v as "day" | "week" | "all")}
            options={[
              { value: "day", label: "最近 24 小时" },
              { value: "week", label: "最近 7 天" },
              { value: "all", label: "全部" },
            ]}
          />
          <ActionButton
            variant="outline"
            size="sm"
            aria-label="刷新概览"
            disabled={loading}
            onClick={() => setRefresh((v) => v + 1)}
          >
            <RefreshCw size={13} />
            刷新
          </ActionButton>
        </div>
      </header>

      {/* Primary Workflow Strip: Step 1 Providers -> Step 2 Agents */}
      <div className="mg-workflow-strip" aria-label="核心工作流">
        <button type="button" className="mg-workflow-card" onClick={onProviders}>
          <span className="mg-workflow-step">01</span>
          <div className="mg-workflow-body">
            <strong>Providers 与模型</strong>
            <small>
              {totalProviders === 0
                ? "添加 Codex 订阅、Copilot 或 API 来源"
                : `${activeProviders} / ${totalProviders} 来源启用 · ${activeModels} 模型可用`}
            </small>
          </div>
          <span className="mg-workflow-action">
            <Layers3 size={14} aria-hidden />
            管理
            <ArrowRight size={13} aria-hidden />
          </span>
        </button>

        <button
          type="button"
          className="mg-workflow-card"
          onClick={onAgents ?? onProviders}
        >
          <span className="mg-workflow-step">02</span>
          <div className="mg-workflow-body">
            <strong>Agents 客户端接入</strong>
            <small>
              {connectedAgents > 0
                ? `已接入 ${connectedAgents} 个本地 Agent（支持 7 款主流 CLI）`
                : "一键接入 Codex、Claude Code、Pi、OpenCode、Hermes 等"}
            </small>
          </div>
          <span className="mg-workflow-action">
            <SquareTerminal size={14} aria-hidden />
            接入
            <ArrowRight size={13} aria-hidden />
          </span>
        </button>
      </div>

      {(error || storageError || pending) && (
        <div className="mc-notice" role={pending ? "status" : "alert"}>
          {storageError
            ? `日志存储不可用：${storageError}`
            : error
              ? "读取失败，下面保留上次结果。请重试或检查日志存储。"
              : "正在切换时间范围，下面暂时保留上次结果。"}
          {storageError && (
            <ActionButton size="sm" variant="outline" onClick={() => onLogs()}>
              查看日志存储
            </ActionButton>
          )}
        </div>
      )}

      {/* Tier 1: Core Metrics */}
      <div className="mc-metrics" aria-label="概览核心指标">
        <article>
          <div className="mc-metric-label">请求</div>
          <strong>
            <RollingNumber value={stats?.total} kind="count" />
          </strong>
          <p>{stats ? `成功 ${stats.success} · 失败 ${stats.failed}` : "暂无记录"}</p>
        </article>
        <article>
          <div className="mc-metric-label">Token 用量</div>
          <strong>
            <RollingNumber value={usage?.total} kind="compact" />
          </strong>
          <p>
            输入 {usage?.input == null ? "—" : compactTokens(usage.input)} / 输出{" "}
            {usage?.output == null ? "—" : compactTokens(usage.output)}
          </p>
        </article>
        <DefaultCost
          data={result?.cost}
          onRefresh={() => setRefresh((v) => v + 1)}
        />
        <article>
          <div className="mc-metric-label">成功率</div>
          <strong>
            <RollingNumber value={rate} kind="percent" />
          </strong>
          <p>
            {stats
              ? `${stats.failed} 失败 · ${stats.pending} 待确认`
              : "暂无记录"}
          </p>
        </article>
      </div>

      {result?.overflow && (
        <p className="mc-notice" role="alert">
          累计用量超过精确展示范围。请缩小时间范围；未填入近似值。
        </p>
      )}

      {/* Tier 1.5: Single primary trend chart */}
      <section className="mc-panel mg-overview-chart">
        <div className="mc-panel-heading">
          <div>
            <h2>{metric === "requests" ? "请求趋势" : "Token 用量趋势"}</h2>
            <p>{usage?.intervalLabel ?? "本机日志汇总"}</p>
          </div>
          <div className="mg-panel-controls">
            <div className="mc-segmented" aria-label="概览趋势指标">
              <button
                type="button"
                aria-pressed={metric === "requests"}
                onClick={() => setMetric("requests")}
              >
                请求
              </button>
              <button
                type="button"
                aria-pressed={metric === "tokens"}
                onClick={() => setMetric("tokens")}
              >
                Token
              </button>
            </div>
            <ActionButton variant="ghost" size="sm" onClick={() => open()}>
              日志明细
              <ArrowRight size={13} />
            </ActionButton>
          </div>
        </div>

        {chart.length ? (
          <div className="mc-chart">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart
                data={chart}
                margin={{ left: -12, right: 12, top: 8, bottom: 0 }}
              >
                <CartesianGrid
                  stroke="var(--mg-line)"
                  vertical={false}
                  strokeDasharray="3 5"
                />
                <XAxis
                  dataKey="label"
                  axisLine={false}
                  tickLine={false}
                  minTickGap={45}
                  tick={{ fontSize: 10, fill: "var(--mg-muted)" }}
                />
                <YAxis
                  axisLine={false}
                  tickLine={false}
                  allowDecimals={false}
                  tickFormatter={compactTokens}
                  tick={{ fontSize: 10, fill: "var(--mg-muted)" }}
                />
                <Tooltip
                  contentStyle={{
                    background: "var(--mg-surface)",
                    border: "1px solid var(--mg-line)",
                    color: "var(--mg-ink)",
                    borderRadius: 6,
                  }}
                  formatter={(v: number) => v.toLocaleString()}
                />
                <Area
                  type="monotone"
                  dataKey={metric === "requests" ? "success" : "tokens"}
                  name={metric === "requests" ? "成功" : "Token"}
                  stroke="var(--mc-accent)"
                  fill="var(--mc-accent-soft)"
                  strokeWidth={2}
                  isAnimationActive={false}
                />
                {metric === "requests" && (
                  <Area
                    type="monotone"
                    dataKey="failed"
                    name="失败"
                    stroke="var(--mc-warning)"
                    fill="transparent"
                    isAnimationActive={false}
                  />
                )}
              </AreaChart>
            </ResponsiveContainer>
          </div>
        ) : (
          <div className="mc-empty">
            {loading && !result
              ? "正在读取历史统计…"
              : stats?.total
                ? "当前记录尚无可展示的趋势数据"
                : "当前时间范围内没有请求"}
          </div>
        )}
      </section>

      {/* Tier 2: Progressive Disclosure for Provider health, Recent failures & Model ranking */}
      <div className="mg-disclosure-wrap">
        <button
          type="button"
          className="mg-disclosure-trigger"
          aria-expanded={showDetails}
          onClick={() => setShowDetails((v) => !v)}
        >
          {showDetails ? (
            <ChevronDown size={15} aria-hidden />
          ) : (
            <ChevronRight size={15} aria-hidden />
          )}
          <span>展开详细分析</span>
          <small>
            来源健康 · 近期异常（{stats?.failed ?? 0}） · 模型用量排行
          </small>
        </button>

        {showDetails && (
          <div className="mg-overview-details">
            <section className="mc-panel">
              <div className="mc-panel-heading">
                <h2>来源表现</h2>
                <ActionButton variant="ghost" size="sm" onClick={onProviders}>
                  管理 Providers
                  <ArrowRight size={13} />
                </ActionButton>
              </div>
              <div className="mc-source-list">
                {providers.map((p) => {
                  const records =
                    result?.providerObservations?.find((o) => o.id === p.id)
                      ?.records ?? [];
                  return (
                    <div className="mc-source" key={p.id}>
                      <div className="mc-source-heading">
                        <span className="mc-avatar">{p.name.slice(0, 1)}</span>
                        <div>
                          <button onClick={() => open({ provider: p.id })}>
                            {p.name}
                          </button>
                          <small>{p.enabled ? "已启用" : "已停用"}</small>
                        </div>
                        <span>
                          {records.length
                            ? `${records.filter((r) => r.state === "failed").length} 失败`
                            : "暂无"}
                        </span>
                      </div>
                      <div
                        className="mc-observations"
                        aria-label={`${p.name} 历史请求终态`}
                      >
                        {Array.from({ length: 20 }, (_, i) => {
                          const r = records[records.length - 1 - i];
                          return (
                            <i
                              key={i}
                              data-state={r?.state ?? "none"}
                              title={
                                r
                                  ? `#${r.id} ${r.state === "success" ? "成功" : r.state === "failed" ? "失败" : "待确认"}`
                                  : "无记录"
                              }
                            />
                          );
                        })}
                      </div>
                    </div>
                  );
                })}
                {!providers.length && (
                  <div className="mc-empty">还没有配置来源。</div>
                )}
              </div>
            </section>

            <section className="mc-panel">
              <div className="mc-panel-heading">
                <h2>
                  近期异常 <span className="mc-count">{stats?.failed ?? "—"}</span>
                </h2>
                <ActionButton
                  variant="ghost"
                  size="sm"
                  onClick={() => open({ status: "failed" })}
                >
                  查看日志
                  <ArrowRight size={13} />
                </ActionButton>
              </div>
              {result?.recentFailures?.map((row) => (
                <button
                  className="mc-issue"
                  key={row.id}
                  onClick={() => open({ search: `#${row.id}` })}
                >
                  <span>
                    <strong>{row.model ?? "未识别模型"}</strong>
                    <small>
                      {row.at === null
                        ? "未知时间"
                        : new Date(row.at).toLocaleTimeString("zh-CN", {
                            hour12: false,
                          })}{" "}
                      · {row.provider ?? "未发出"}
                    </small>
                  </span>
                  <span className="mc-error-code">
                    {row.http === 200 ? "流式 / 模型失败" : `HTTP ${row.http}`}
                  </span>
                  <span className="mc-issue-code" title={row.code}>
                    {row.code || "查看详情"}
                    <ArrowRight size={13} />
                  </span>
                </button>
              ))}
              {!result?.recentFailures?.length && (
                <div className="mc-empty">当前范围内没有失败记录</div>
              )}
            </section>

            <section className="mc-panel">
              <div className="mc-panel-heading">
                <h2>模型排行</h2>
              </div>
              <div className="mc-rankings">
                {usage?.ranking.slice(0, 5).map((item, i) => (
                  <button key={item.id} onClick={() => open({ model: item.label })}>
                    <span>{String(i + 1).padStart(2, "0")}</span>
                    <div>
                      <div>
                        <strong>{item.label}</strong>
                        <small>{compactTokens(item.total)}</small>
                      </div>
                      <div className="mc-rank-bar">
                        <i
                          style={{
                            width: `${usage.total ? (item.total / usage.total) * 100 : 0}%`,
                          }}
                        />
                      </div>
                    </div>
                  </button>
                ))}
                {!usage?.ranking.length && (
                  <div className="mc-empty">还没有完整报告的模型用量</div>
                )}
              </div>
            </section>
          </div>
        )}
      </div>

      <footer className="mc-overview-footer">
        <span>
          {updated
            ? `更新于 ${updated.toLocaleTimeString("zh-CN", { hour12: false })}`
            : "等待读取"}{" "}
          · 完整用量覆盖 {usage ? `${usage.reported}/${usage.eligible}` : "—"}{" "}
          条记录
        </span>
        <Popover>
          <PopoverTrigger asChild>
            <ActionButton variant="ghost" size="sm">
              统计口径
              <Info size={13} />
            </ActionButton>
          </PopoverTrigger>
          <PopoverContent className="ls-popover">
            <h2>本机历史日志</h2>
            <p>
              指标与图表统计当前范围的全部记录。失败或未知用量不填零，成功率排除待确认记录。
            </p>
            <p>
              概览只读刷新，不会自动测试模型、启动网关或修改配置。
            </p>
          </PopoverContent>
        </Popover>
      </footer>
    </section>
  );
}
