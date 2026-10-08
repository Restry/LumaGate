import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  ArrowRight,
  Info,
  RefreshCw,
  ShieldCheck,
  Zap,
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
import type { Snapshot } from "./types";

type Props = {
  snapshot: Snapshot;
  onLogs: (filters?: Partial<LogFilters>) => void;
  onProviders: () => void;
};
export function Overview({ snapshot, onLogs, onProviders }: Props) {
  const [period, setPeriod] = useState<"day" | "week" | "all">("day"),
    [metric, setMetric] = useState("requests"),
    [refresh, setRefresh] = useState(0);
  const [result, setResult] = useState<HistoryPage | null>(null),
    [error, setError] = useState(false),
    [loading, setLoading] = useState(true),
    [storageError, setStorageError] = useState<string | null>(null),
    [updated, setUpdated] = useState<Date | null>(null);
  const [loadedPeriod, setLoadedPeriod] = useState(period);
  useEffect(() => {
    let active = true,
      timer: ReturnType<typeof setTimeout> | undefined;
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
  const stats = result?.stats,
    usage = result?.analytics,
    rate =
      stats && stats.success + stats.failed
        ? stats.success / (stats.success + stats.failed)
        : null;
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
        (result?.providerObservations?.find((p) => p.id === b.id)?.records[0]
          ?.at ?? 0) -
        (result?.providerObservations?.find((p) => p.id === a.id)?.records[0]
          ?.at ?? 0),
    )
    .slice(0, 3);
  const pending = loadedPeriod !== period;
  const open = (filters: Partial<LogFilters> = {}) =>
    onLogs({ preset: loadedPeriod, ...filters });
  return (
    <section className="mc-overview" aria-label="运行概览">
      <header className="mg-page-heading">
        <div>
          <h1>运行概览</h1>
        </div>
        <div className="mg-heading-actions">
          <span className="mc-data-state">
            {storageError
              ? "存储异常"
              : error
                ? "更新失败"
                : pending
                  ? "切换中"
                  : loading && !result
                    ? "读取中"
                    : "本机日志"}
          </span>
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
            <RefreshCw size={14} />
            刷新
          </ActionButton>
        </div>
      </header>
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
      <div className="mc-metrics" aria-label="概览核心指标">
        <article>
          <div className="mc-metric-label">
            总数
            <Activity size={15} />
          </div>
          <strong>
            <RollingNumber value={stats?.total} kind="count" />
          </strong>
          <p>当前范围内的全部请求</p>
        </article>
        <article>
          <div className="mc-metric-label">
            总用量
            <Zap size={15} />
          </div>
          <strong>
            <RollingNumber value={usage?.total} kind="compact" />
          </strong>
          <p>
            输入 {usage?.input == null ? "—" : compactTokens(usage.input)} /
            输出 {usage?.output == null ? "—" : compactTokens(usage.output)}
            <br />
            缓存 <RollingNumber value={result?.cache.share} kind="percent" />
            {result?.cache.read != null &&
              ` · ${compactTokens(result.cache.read)} Token`}
          </p>
        </article>
        <DefaultCost
          data={result?.cost}
          onRefresh={() => setRefresh((v) => v + 1)}
        />
        <article>
          <div className="mc-metric-label">
            成功率
            <ShieldCheck size={15} />
          </div>
          <strong>
            <RollingNumber value={rate} kind="percent" />
          </strong>
          <p>
            {stats
              ? `${stats.failed} 次失败 · ${stats.pending} 次待确认`
              : "等待日志统计"}
          </p>
        </article>
      </div>
      {result?.overflow && (
        <p className="mc-notice" role="alert">
          累计用量超过精确展示范围。请缩小时间范围；未填入近似值。
        </p>
      )}
      <div className="mc-primary-grid">
        <section className="mc-panel">
          <div className="mc-panel-heading">
            <div>
              <h2>{metric === "requests" ? "请求趋势" : "用量趋势"}</h2>
              <p>{usage?.intervalLabel ?? "等待统计"}</p>
            </div>
            <div className="mc-segmented" aria-label="概览趋势指标">
              <button
                aria-pressed={metric === "requests"}
                onClick={() => setMetric("requests")}
              >
                请求
              </button>
              <button
                aria-pressed={metric === "tokens"}
                onClick={() => setMetric("tokens")}
              >
                Token
              </button>
            </div>
          </div>
          <div className="mc-chart-legend">
            <span>
              <i />
              {metric === "requests" ? "成功" : "Token"}
            </span>
            {metric === "requests" && (
              <>
                <span>
                  <i className="is-failed" />
                  失败
                </span>
                <span>
                  <i className="is-pending" />
                  待确认
                </span>
              </>
            )}
          </div>
          {chart.length ? (
            <div className="mc-chart">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart
                  data={chart}
                  margin={{ left: -12, right: 12, top: 12, bottom: 0 }}
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
                    <>
                      <Area
                        type="monotone"
                        dataKey="failed"
                        name="失败"
                        stroke="var(--mc-warning)"
                        fill="transparent"
                        isAnimationActive={false}
                      />
                      <Area
                        type="monotone"
                        dataKey="pending"
                        name="待确认"
                        stroke="var(--mg-muted)"
                        strokeDasharray="4 4"
                        fill="transparent"
                        isAnimationActive={false}
                      />
                    </>
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
          <details className="mc-chart-data">
            <summary>查看趋势数值</summary>
            <div className="mg-table-wrap">
              <table aria-label="概览趋势数值">
                <thead>
                  <tr>
                    <th>时间</th>
                    {metric === "requests" ? (
                      <>
                        <th>成功</th>
                        <th>失败</th>
                        <th>待确认</th>
                      </>
                    ) : (
                      <th>Token</th>
                    )}
                  </tr>
                </thead>
                <tbody>
                  {metric === "requests"
                    ? result?.requestTimeline?.map((p) => (
                        <tr key={p.at}>
                          <td>{new Date(p.at).toLocaleString("zh-CN")}</td>
                          <td>{p.success}</td>
                          <td>{p.failed}</td>
                          <td>{p.pending}</td>
                        </tr>
                      ))
                    : usage?.timeline.map((p) => (
                        <tr key={p.id}>
                          <td>{p.label}</td>
                          <td>{p.total.toLocaleString()}</td>
                        </tr>
                      ))}
                </tbody>
              </table>
            </div>
          </details>
        </section>
        <section className="mc-panel">
          <div className="mc-panel-heading">
            <div>
              <h2>来源表现</h2>
              <p>作为最终来源的最近 20 条请求</p>
            </div>
            <ActionButton variant="ghost" size="sm" onClick={onProviders}>
              查看全部
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
                      <small>{p.enabled ? "配置已启用" : "配置已停用"}</small>
                    </div>
                    <span>
                      {records.length
                        ? `${records.filter((r) => r.state === "failed").length} 次失败`
                        : "暂无匹配观测"}
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
                              : "没有观测"
                          }
                        />
                      );
                    })}
                  </div>
                </div>
              );
            })}
            {!providers.length && (
              <div className="mc-empty">
                还没有 Provider。接入来源后可在这里查看历史表现。
              </div>
            )}
          </div>
          <p className="mc-panel-footnote">
            只展示历史请求结果，不代表实时健康；回退前的其他尝试仍在日志详情中。
          </p>
        </section>
      </div>
      <div className="mc-secondary-grid">
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
                {row.code || "查看终态详情"}
                <ArrowRight size={13} />
              </span>
            </button>
          ))}
          {!result?.recentFailures?.length && (
            <div className="mc-empty">
              {stats?.failed
                ? "可在日志中查看失败记录"
                : "当前范围内没有失败记录"}
            </div>
          )}
        </section>
        <section className="mc-panel">
          <div className="mc-panel-heading">
            <div>
              <h2>模型排行</h2>
              <p>按完整报告的 Token</p>
            </div>
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
      <footer className="mc-overview-footer">
        <span>
          {updated
            ? `更新于 ${updated.toLocaleTimeString("zh-CN", { hour12: false })}`
            : "等待读取"}{" "}
          · 完整用量覆盖 {usage ? `${usage.reported}/${usage.eligible}` : "—"}{" "}
          条推理记录
        </span>
        <Popover>
          <PopoverTrigger asChild>
            <ActionButton variant="ghost" size="sm">
              统计口径
              <Info size={14} />
            </ActionButton>
          </PopoverTrigger>
          <PopoverContent className="ls-popover">
            <h2>本机历史日志</h2>
            <p>
              指标与图表统计当前范围的全部记录，不受页面条数限制。失败或未知用量不填零。成功率排除待确认记录。
            </p>
            <p>
              缓存已包含在输入 Token
              中，不重复相加。费用仅按已获取的默认单价估算。来源观测仅归属最终来源，不代表所有回退尝试的成功率。
            </p>
            <p>
              概览只读刷新；不会自动测试模型、启动网关或修改配置。最近 7
              天按本机日历计算。
            </p>
          </PopoverContent>
        </Popover>
      </footer>
    </section>
  );
}
