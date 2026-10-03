import { RollingNumber } from "./RollingNumber";
import type { HistoryPage } from "./log-history";
export function LogSummary({ result }: { result: HistoryPage | null }) {
  const stats = result?.stats;
  const rate =
    stats && stats.success + stats.failed
      ? stats.success / (stats.success + stats.failed)
      : null;
  const number = (value?: number) => (
    <RollingNumber
      value={value}
      kind={(value ?? 0) >= 10000 ? "compact" : "count"}
    />
  );
  return (
    <dl className="ls-overview" aria-label="当前筛选全部记录统计">
      <div className="ls-stat ls-stat-total">
        <dt>总数</dt>
        <dd>{number(stats?.total)}</dd>
      </div>
      <div className="ls-stat">
        <dt>成功</dt>
        <dd>{number(stats?.success)}</dd>
      </div>
      <div className="ls-stat">
        <dt>失败</dt>
        <dd className="mg-inline-error">{number(stats?.failed)}</dd>
      </div>
      <div className="ls-stat">
        <dt>待确认</dt>
        <dd>{number(stats?.pending)}</dd>
      </div>
      <div className="ls-stat ls-stat-rate">
        <dt>成功率</dt>
        <dd>
          <RollingNumber value={rate} kind="percent" />
        </dd>
      </div>
      <div className="ls-stat ls-stat-tokens">
        <dt>Token</dt>
        <dd>
          <RollingNumber value={result?.analytics.total} kind="compact" />
          <small>
            {result
              ? `${result.analytics.reported}/${result.analytics.eligible} 完整报告`
              : "等待读取"}
          </small>
        </dd>
      </div>
    </dl>
  );
}
