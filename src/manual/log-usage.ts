import type { RequestLog } from "./RequestLogs";

export interface TokenUsage {
  state: "pending" | "reported" | "partial" | "unavailable";
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  cacheReadTokens: number | null;
  cacheWriteTokens: number | null;
  reasoningTokens: number | null;
  reason: string | null;
}
export type UsageGrouping = "model" | "key";
export interface UsagePoint {
  id: string;
  label: string;
  input: number;
  output: number;
  total: number;
  requests: number;
}
export interface UsageAnalytics {
  eligible: number;
  reported: number;
  pending: number;
  partial: number;
  unavailable: number;
  input: number | null;
  output: number | null;
  total: number | null;
  timeline: UsagePoint[];
  intervalLabel: string;
  rangeLabel: string;
  invalidTimes: number;
  distribution: UsagePoint[];
  ranking: UsagePoint[];
}
const endpoints = new Set([
  "/v1/messages",
  "/v1/chat/completions",
  "/v1/responses",
  "/v1/responses/compact",
]);
const MAX_COUNT = Math.floor(Number.MAX_SAFE_INTEGER / 200);
export function validTokenCount(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= MAX_COUNT
  );
}
export function usageEligible(row: RequestLog): boolean {
  return (
    endpoints.has(row.endpoint) &&
    (row.providers === undefined || row.providers.length > 0)
  );
}
export function completeUsage(row: RequestLog): row is RequestLog & {
  usage: TokenUsage & {
    inputTokens: number;
    outputTokens: number;
    totalTokens: number;
  };
} {
  const value = row.usage;
  return (
    usageEligible(row) &&
    value?.state === "reported" &&
    validTokenCount(value.inputTokens) &&
    validTokenCount(value.outputTokens) &&
    validTokenCount(value.totalTokens) &&
    value.inputTokens + value.outputTokens === value.totalTokens
  );
}
export function cacheUsageAnalytics(rows: RequestLog[]) {
  const complete = rows.filter(completeUsage);
  let reported = 0,
    input = 0,
    read = 0;
  for (const row of complete) {
    const cached = row.usage.cacheReadTokens;
    if (!validTokenCount(cached) || cached > row.usage.inputTokens) continue;
    reported += 1;
    input += row.usage.inputTokens;
    read += cached;
  }
  return {
    complete: complete.length,
    reported,
    read: reported ? read : null,
    nonRead: reported ? input - read : null,
    share: reported && input > 0 ? read / input : null,
  };
}

export function formatTokens(value?: number | null): string {
  return value == null || !Number.isSafeInteger(value) || value < 0
    ? "—"
    : new Intl.NumberFormat("zh-CN").format(value);
}
export function compactTokens(value: number): string {
  return new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 2,
  }).format(value);
}
export function usageLabel(row: RequestLog): string {
  if (!usageEligible(row)) return "不适用";
  if (completeUsage(row)) return "已报告";
  if (row.responseState === "接收中" || row.usage?.state === "pending")
    return "用量待返回";
  if (row.usage?.state === "partial") return "部分用量";
  if (row.usage?.reason === "event_too_large") return "用量事件过大";
  if (row.usage?.reason === "encoded_response") return "未解析压缩用量";
  if (
    row.usage?.state === "reported" ||
    ["invalid_usage", "inconsistent_usage"].includes(row.usage?.reason ?? "")
  )
    return "用量格式异常";
  return "未报告";
}
function identity(row: RequestLog, grouping: UsageGrouping): [string, string] {
  if (grouping === "model")
    return [`model:${row.model ?? "\0"}`, row.model ?? "未识别模型"];
  if (row.caller?.kind === "key")
    return [
      `key:${row.caller.keyId}`,
      `${row.caller.name} (${row.caller.keyId.slice(0, 8)})`,
    ];
  if (row.caller?.kind === "local") return ["local", "本机免鉴权"];
  if (row.caller?.kind === "rejected") return ["rejected", "鉴权拒绝"];
  return ["unknown", "未记录的调用来源"];
}
function add(
  point: UsagePoint,
  row: RequestLog & {
    usage: { inputTokens: number; outputTokens: number; totalTokens: number };
  },
) {
  point.input += row.usage.inputTokens;
  point.output += row.usage.outputTokens;
  point.total += row.usage.totalTokens;
  point.requests += 1;
}
export function usageAnalytics(
  rows: RequestLog[],
  grouping: UsageGrouping,
): UsageAnalytics {
  const eligible = rows.filter(usageEligible);
  const reported = eligible.filter(completeUsage);
  const missing = eligible.filter((row) => !completeUsage(row));
  const pending = missing.filter(
    (row) => row.responseState === "接收中" || row.usage?.state === "pending",
  ).length;
  const partial = missing.filter(
    (row) => row.responseState !== "接收中" && row.usage?.state === "partial",
  ).length;
  const rankingMap = new Map<string, UsagePoint>();
  const nameTimes = new Map<string, number>();
  for (const row of reported) {
    const [id, label] = identity(row, grouping);
    const time = Date.parse(row.startedAt);
    let point = rankingMap.get(id);
    if (!point) {
      point = { id, label, input: 0, output: 0, total: 0, requests: 0 };
      rankingMap.set(id, point);
    }
    if (
      !nameTimes.has(id) ||
      (Number.isFinite(time) && time > nameTimes.get(id)!)
    ) {
      point.label = label;
      nameTimes.set(id, Number.isFinite(time) ? time : -Infinity);
    }
    add(point, row);
  }
  const ranking = [...rankingMap.values()].sort(
    (a, b) => b.total - a.total || (a.id < b.id ? -1 : 1),
  );
  const distribution = ranking.slice(0, 5).map((point) => ({ ...point }));
  if (ranking.length > 5) {
    const rest = ranking.slice(5);
    distribution.push({
      id: "other",
      label: `其他（${rest.length}项）`,
      input: rest.reduce((s, p) => s + p.input, 0),
      output: rest.reduce((s, p) => s + p.output, 0),
      total: rest.reduce((s, p) => s + p.total, 0),
      requests: rest.reduce((s, p) => s + p.requests, 0),
    });
  }
  const timed = reported
    .map((row) => ({ row, time: Date.parse(row.startedAt) }))
    .filter(({ time }) => Number.isFinite(time));
  const first = timed.length ? Math.min(...timed.map(({ time }) => time)) : 0;
  const last = timed.length ? Math.max(...timed.map(({ time }) => time)) : 0;
  const steps: [number, string][] = [
    [60_000, "1 分钟"],
    [300_000, "5 分钟"],
    [900_000, "15 分钟"],
    [3_600_000, "1 小时"],
    [21_600_000, "6 小时"],
    [86_400_000, "1 天"],
  ];
  const [step, intervalLabel] = steps.find(
    ([step]) => (last - first) / step <= 11,
  ) ?? [Math.ceil((last - first) / 11 / 86_400_000) * 86_400_000, "多日"];
  const timelineMap = new Map<number, UsagePoint>();
  const fullDate = new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  const date = new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  const clock = new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  for (const { row, time } of timed) {
    const bucket = Math.floor(time / step) * step;
    let point = timelineMap.get(bucket);
    if (!point) {
      point = {
        id: String(bucket),
        label: (last - first >= 86_400_000 ? date : clock).format(bucket),
        input: 0,
        output: 0,
        total: 0,
        requests: 0,
      };
      timelineMap.set(bucket, point);
    }
    add(point, row);
  }
  if (timed.length) {
    // Empty intervals retain their place on the clock. requests=0 means no reported sample,
    // not an invented zero-token request; tooltip/data-table render it as unavailable.
    for (
      let bucket = Math.floor(first / step) * step;
      bucket <= Math.floor(last / step) * step;
      bucket += step
    ) {
      if (!timelineMap.has(bucket))
        timelineMap.set(bucket, {
          id: String(bucket),
          label: (last - first >= 86_400_000 ? date : clock).format(bucket),
          input: 0,
          output: 0,
          total: 0,
          requests: 0,
        });
    }
  }
  return {
    eligible: eligible.length,
    reported: reported.length,
    pending,
    partial,
    unavailable: missing.length - pending - partial,
    input: reported.length
      ? reported.reduce((s, r) => s + r.usage.inputTokens, 0)
      : null,
    output: reported.length
      ? reported.reduce((s, r) => s + r.usage.outputTokens, 0)
      : null,
    total: reported.length
      ? reported.reduce((s, r) => s + r.usage.totalTokens, 0)
      : null,
    timeline: [...timelineMap]
      .sort(([a], [b]) => a - b)
      .map(([, point]) => point),
    intervalLabel,
    rangeLabel: timed.length
      ? `${fullDate.format(first)} – ${fullDate.format(last)}`
      : "无有效时间",
    invalidTimes: reported.length - timed.length,
    distribution,
    ranking,
  };
}
