import { useId } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Choice } from "./ui";
import {
  compactTokens,
  formatTokens,
  type UsageAnalytics,
  type UsageGrouping,
  type UsagePoint,
} from "./log-usage";

function shortLabel(label: string): string {
  let width = 0,
    result = "";
  for (const char of label) {
    width += /[^\u0000-\u00ff]/.test(char) ? 2 : 1;
    if (width > 18) return `${result}…`;
    result += char;
  }
  return result;
}
function UsageTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: readonly { payload?: UsagePoint }[];
}) {
  const point = payload?.[0]?.payload;
  if (!active || !point) return null;
  return (
    <div className="mg-token-tooltip">
      <strong>{point.label}</strong>
      {point.requests ? (
        <>
          <p>输入 {formatTokens(point.input)}</p>
          <p>输出 {formatTokens(point.output)}</p>
          <p>
            合计 {formatTokens(point.total)} · {point.requests} 条记录
          </p>
        </>
      ) : (
        <p>此时段没有完整报告用量的记录</p>
      )}
    </div>
  );
}
function Plot({
  data,
  horizontal,
  label,
}: {
  data: UsagePoint[];
  horizontal?: boolean;
  label: string;
}) {
  if (!data.length)
    return <div className="mg-token-plot-empty">没有可绘制的记录。</div>;
  if (data.every((point) => point.total === 0))
    return (
      <div className="mg-token-plot-empty">
        已明确报告 0 Token，无非零用量可绘制。
      </div>
    );
  return (
    <div className="mg-token-plot">
      <ResponsiveContainer
        width="100%"
        height="100%"
        minWidth={0}
        debounce={100}
      >
        <BarChart
          data={data}
          layout={horizontal ? "vertical" : "horizontal"}
          accessibilityLayer
          title={label}
          margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
          barCategoryGap="24%"
        >
          <CartesianGrid
            stroke="var(--mg-line)"
            vertical={!!horizontal}
            horizontal={!horizontal}
          />
          {horizontal ? (
            <>
              <XAxis
                type="number"
                tickFormatter={compactTokens}
                axisLine={false}
                tickLine={false}
                tick={{ fill: "var(--mg-muted)", fontSize: 11 }}
                allowDecimals={false}
              />
              <YAxis
                type="category"
                dataKey="label"
                width={112}
                tickFormatter={shortLabel}
                axisLine={false}
                tickLine={false}
                tick={{ fill: "var(--mg-muted)", fontSize: 11 }}
              />
            </>
          ) : (
            <>
              <XAxis
                dataKey="label"
                interval="preserveStartEnd"
                minTickGap={18}
                axisLine={false}
                tickLine={false}
                tick={{ fill: "var(--mg-muted)", fontSize: 11 }}
              />
              <YAxis
                tickFormatter={compactTokens}
                width={52}
                axisLine={false}
                tickLine={false}
                tick={{ fill: "var(--mg-muted)", fontSize: 11 }}
                allowDecimals={false}
              />
            </>
          )}
          <Tooltip
            content={<UsageTooltip />}
            isAnimationActive={false}
            cursor={{ fill: "var(--mg-soft)" }}
          />
          <Bar
            dataKey="input"
            name="输入"
            stackId="tokens"
            fill="var(--mg-token-input)"
            maxBarSize={28}
            isAnimationActive={false}
          />
          <Bar
            dataKey="output"
            name="输出"
            stackId="tokens"
            fill="var(--mg-token-output)"
            maxBarSize={28}
            isAnimationActive={false}
          />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
function DataTable({ data, label }: { data: UsagePoint[]; label: string }) {
  return (
    <div className="mg-table-wrap">
      <table className="mg-token-data-table">
        <caption>{label}</caption>
        <thead>
          <tr>
            <th scope="col">分组</th>
            <th scope="col">完整记录</th>
            <th scope="col">输入</th>
            <th scope="col">输出</th>
            <th scope="col">合计</th>
          </tr>
        </thead>
        <tbody>
          {data.map((point) => (
            <tr key={point.id}>
              <th scope="row">{point.label}</th>
              <td>{point.requests}</td>
              <td>{formatTokens(point.requests ? point.input : null)}</td>
              <td>{formatTokens(point.requests ? point.output : null)}</td>
              <td>{formatTokens(point.requests ? point.total : null)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
export default function TokenCharts({
  analytics,
  grouping,
  onGrouping,
}: {
  analytics: UsageAnalytics;
  grouping: UsageGrouping;
  onGrouping: (value: UsageGrouping) => void;
}) {
  const id = useId();
  return (
    <>
      <div className="mg-token-chart-grid">
        <figure className="mg-token-chart" aria-labelledby={`${id}-trend`}>
          <figcaption>
            <h3 id={`${id}-trend`}>趋势</h3>
            <p title={analytics.rangeLabel}>每 {analytics.intervalLabel}</p>
          </figcaption>
          <Plot data={analytics.timeline} label="输入与输出 Token 用量趋势" />
          {!!analytics.invalidTimes && (
            <p className="mg-hint">
              另有 {analytics.invalidTimes} 条记录时间无效，未进入趋势图。
            </p>
          )}
        </figure>
        <figure className="mg-token-chart" aria-labelledby={`${id}-rank`}>
          <figcaption className="mg-token-rank-heading">
            <div>
              <h3 id={`${id}-rank`} title="前 5 项，其余合并为其他">
                分布
              </h3>
            </div>
            <Choice
              label="Token 分布分组"
              value={grouping}
              onChange={(value) =>
                onGrouping(value === "key" ? "key" : "model")
              }
              options={[
                { value: "model", label: "模型" },
                { value: "key", label: "密钥" },
              ]}
            />
          </figcaption>
          <Plot
            data={analytics.distribution}
            horizontal
            label={
              grouping === "model"
                ? "按请求模型的 Token 分布"
                : "按访问密钥的 Token 分布"
            }
          />
        </figure>
      </div>
      <details className="mg-token-data">
        <summary>查看数据</summary>
        <DataTable data={analytics.timeline} label="Token 趋势数据" />
        <DataTable
          data={analytics.ranking}
          label={grouping === "model" ? "请求模型用量明细" : "访问密钥用量明细"}
        />
      </details>
    </>
  );
}
