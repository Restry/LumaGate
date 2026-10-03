import { lazy, Suspense, useId, useMemo, useState } from "react";
import { BarChart3, Info } from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useLogPreference } from "./log-preferences";
import { TokenNumber } from "./TokenNumber";
import { RollingNumber } from "./RollingNumber";
import { ActionButton } from "./ui";
import type { RequestLog } from "./RequestLogs";
import {
  completeUsage,
  cacheUsageAnalytics,
  formatTokens,
  usageAnalytics,
  usageEligible,
  usageLabel,
  type UsageGrouping,
} from "./log-usage";
import "./token-usage.css";
const TokenCharts = lazy(() => import("./TokenCharts"));
export function TokenUsagePanel({
  rows,
  loaded,
  summary,
  grouping: controlledGrouping,
  onGrouping,
}: {
  rows: RequestLog[];
  loaded: boolean;
  summary?: {
    analytics: ReturnType<typeof usageAnalytics>;
    cache: ReturnType<typeof cacheUsageAnalytics>;
    overflow: boolean;
  };
  grouping?: UsageGrouping;
  onGrouping?: (v: UsageGrouping) => void;
}) {
  const [localGrouping, setLocalGrouping] = useState<UsageGrouping>("model"),
    grouping = controlledGrouping ?? localGrouping,
    setGrouping = onGrouping ?? setLocalGrouping;
  const [showCharts, setShowCharts] = useLogPreference("charts", true);
  const localAnalytics = useMemo(
    () => (summary ? null : usageAnalytics(rows, grouping)),
    [rows, grouping, summary],
  );
  const localCache = useMemo(
    () => (summary ? null : cacheUsageAnalytics(rows)),
    [rows, summary],
  );
  const analytics = summary?.analytics ?? localAnalytics!,
    cache = summary?.cache ?? localCache!,
    chartsId = useId();
  return (
    <section className="mg-token-panel ls-analysis" aria-label="Token 用量分析">
      <div className="ls-analysis-heading">
        <h2>用量</h2>
        <div className="ls-analysis-tools">
          <Popover>
            <PopoverTrigger asChild>
              <ActionButton variant="ghost" size="sm" aria-label="用量口径">
                <Info size={14} aria-hidden />
                口径
              </ActionButton>
            </PopoverTrigger>
            <PopoverContent className="ls-popover ls-about" align="end">
              <h2>用量口径</h2>
              <p>
                当前筛选范围内，仅汇总上游完整报告的
                usage，不受分页影响，不按文字长度估算。未报告不当成 0。
              </p>
              <p>
                缓存读取已包含在输入中，推理已包含在输出中，不重复相加。“其他输入”是对应输入减缓存读取，可能含缓存写入。缓存占比是
                Token 占比，不代表费用折扣。
              </p>
              <p>
                缓存统计只覆盖明确报告该字段的记录。用量：{analytics.reported} /{" "}
                {analytics.eligible}；缓存字段：{cache.reported} /{" "}
                {cache.complete}。
              </p>
              <p>
                未计入用量：待返回 {analytics.pending}、部分 {analytics.partial}
                、未报告 {analytics.unavailable}
                。目录查询和未转发请求不计入覆盖率。
              </p>
              <p>未知的失败重试消耗不在统计内。这不是完整账单。</p>
            </PopoverContent>
          </Popover>
          <ActionButton
            variant="ghost"
            size="sm"
            disabled={summary?.overflow}
            aria-label={showCharts ? "收起图表" : "显示图表"}
            aria-expanded={showCharts && !summary?.overflow}
            aria-controls={
              showCharts && !summary?.overflow ? chartsId : undefined
            }
            onClick={() => setShowCharts(!showCharts)}
          >
            <BarChart3 size={14} aria-hidden />
            {showCharts ? "隐藏图表" : "显示图表"}
          </ActionButton>
        </div>
      </div>
      <div className="ls-usage-hero">
        <div className="ls-usage-card">
          <dl className="ls-usage-metrics" aria-label="当前筛选的 Token 统计">
            <div className="ls-token-total">
              <dt>总用量</dt>
              <dd>
                <TokenNumber
                  value={loaded ? analytics.total : null}
                  focusable
                  animated
                />
              </dd>
            </div>
            <div className="ls-token-secondary">
              <dt>输入</dt>
              <dd>
                <TokenNumber
                  value={loaded ? analytics.input : null}
                  focusable
                  animated
                />
              </dd>
            </div>
            <div className="ls-token-secondary">
              <dt>输出</dt>
              <dd>
                <TokenNumber
                  value={loaded ? analytics.output : null}
                  focusable
                  animated
                />
              </dd>
            </div>
          </dl>
          <div className="ls-card-foot">
            <span>已报告</span>
            <span>
              {loaded
                ? `${analytics.reported.toLocaleString()} / ${analytics.eligible.toLocaleString()}`
                : "—"}
            </span>
          </div>
        </div>
        <div className="ls-usage-card ls-cache-card">
          <dl
            aria-label="当前筛选的缓存 Token 统计"
            className="ls-cache-metrics"
          >
            <div className="ls-cache-share">
              <dt>缓存占比</dt>
              <dd>
                <RollingNumber
                  value={loaded ? cache.share : null}
                  kind="ratio"
                />
              </dd>
            </div>
            <div className="ls-cache-meter" aria-hidden>
              <dt hidden>缓存占比</dt>
              <dd>
                <span
                  data-known={loaded && cache.share !== null}
                  style={{
                    transform: `scaleX(${loaded ? (cache.share ?? 0) : 0})`,
                  }}
                />
              </dd>
            </div>
            <div className="ls-token-secondary">
              <dt>缓存读取</dt>
              <dd>
                <TokenNumber
                  value={loaded ? cache.read : null}
                  focusable
                  animated
                />
              </dd>
            </div>
            <div className="ls-token-secondary">
              <dt>其他输入</dt>
              <dd>
                <TokenNumber
                  value={loaded ? cache.nonRead : null}
                  focusable
                  animated
                />
              </dd>
            </div>
          </dl>
          <div className="ls-card-foot">
            <span>缓存字段</span>
            <span>
              {loaded
                ? `${cache.reported.toLocaleString()} / ${cache.complete.toLocaleString()}`
                : "—"}
            </span>
          </div>
        </div>
      </div>
      {summary?.overflow && (
        <p role="alert" className="ls-alert">
          用量超出展示上限，请缩小日期范围。原始记录仍保留。
        </p>
      )}
      {showCharts && !summary?.overflow && (
        <div id={chartsId} className="ls-analysis-plots">
          {analytics.reported ? (
            <>
              <div className="ls-chart-legend">
                <span>
                  <i className="is-input" aria-hidden />
                  输入
                </span>
                <span>
                  <i className="is-output" aria-hidden />
                  输出
                </span>
              </div>
              <Suspense
                fallback={
                  <div className="ls-chart-placeholder">正在加载图表…</div>
                }
              >
                <TokenCharts
                  analytics={analytics}
                  grouping={grouping}
                  onGrouping={setGrouping}
                />
              </Suspense>
            </>
          ) : (
            <div className="ls-empty">
              <h3>暂无完整用量</h3>
              <p>{loaded ? "上游报告用量后显示图表。" : "正在读取…"}</p>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
export function TokenUsageCell({ row }: { row: RequestLog }) {
  if (!usageEligible(row)) return <span className="mg-token-muted">—</span>;
  if (completeUsage(row))
    return (
      <>
        <strong>
          <TokenNumber value={row.usage.totalTokens} />
        </strong>
        <small>
          入 <TokenNumber value={row.usage.inputTokens} /> / 出{" "}
          <TokenNumber value={row.usage.outputTokens} />
        </small>
      </>
    );
  return (
    <>
      <span>
        <TokenNumber value={row.usage?.totalTokens} />
      </span>
      <small>
        {usageLabel(row)}
        {row.usage?.state === "partial" ? " · 未计入汇总" : ""}
      </small>
    </>
  );
}
export function TokenUsageDetails({ row }: { row: RequestLog }) {
  const usage = row.usage;
  return (
    <section className="mg-token-record" aria-label="这条请求的 Token 用量">
      <h2>Token · {usageLabel(row)}</h2>
      {usageEligible(row) ? (
        <>
          <dl>
            <div>
              <dt>输入（含缓存）</dt>
              <dd>{formatTokens(usage?.inputTokens)}</dd>
            </div>
            <div>
              <dt>输出</dt>
              <dd>{formatTokens(usage?.outputTokens)}</dd>
            </div>
            <div>
              <dt>合计</dt>
              <dd>{formatTokens(usage?.totalTokens)}</dd>
            </div>
            {usage?.cacheReadTokens != null && (
              <div>
                <dt>缓存读取</dt>
                <dd>{formatTokens(usage.cacheReadTokens)}</dd>
              </div>
            )}
            {usage?.cacheWriteTokens != null && (
              <div>
                <dt>缓存写入</dt>
                <dd>{formatTokens(usage.cacheWriteTokens)}</dd>
              </div>
            )}
            {usage?.reasoningTokens != null && (
              <div>
                <dt>推理输出</dt>
                <dd>{formatTokens(usage.reasoningTokens)}</dd>
              </div>
            )}
          </dl>
          <p>
            {completeUsage(row)
              ? "上游实际用量；缓存、推理不重复累加。"
              : "部分或未确认的用量不计入汇总。"}
          </p>
        </>
      ) : (
        <p>无已转发的推理调用。</p>
      )}
    </section>
  );
}
