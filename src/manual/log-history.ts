import type { RequestLog } from "./RequestLogs";
import type { UsageAnalytics, UsageGrouping } from "./log-usage";
export interface LogStorage {
  ready: boolean;
  path: string;
  message: string;
}
export interface HistoryQuery {
  from?: number;
  to?: number;
  model: string;
  provider: string;
  caller: string;
  endpoint: string;
  status: string;
  search: string;
  page: number;
  pageSize: number;
  anchor?: number;
  grouping: UsageGrouping;
}
export interface HistoryPage {
  storage: LogStorage;
  rows: RequestLog[];
  matched: number;
  page: number;
  pageSize: number;
  pages: number;
  anchor: number;
  newerAvailable: boolean;
  overflow: boolean;
  stats: { total: number; success: number; failed: number; pending: number };
  analytics: UsageAnalytics & {
    intervalMs: number;
    bounds: [number | null, number | null];
  };
  cache: {
    complete: number;
    reported: number;
    read: number | null;
    nonRead: number | null;
    share: number | null;
  };
  requestTimeline?: {
    at: number;
    success: number;
    failed: number;
    pending: number;
  }[];
  providerObservations?: {
    id: string;
    name: string;
    records: {
      id: number;
      at: number | null;
      state: "success" | "failed" | "pending";
    }[];
  }[];
  recentFailures?: {
    id: number;
    at: number | null;
    model: string | null;
    provider: string | null;
    http: number;
    code: string;
  }[];
  options: {
    providers: { value: string; label: string }[];
    callers: { value: string; label: string }[];
    endpoints: string[];
    statuses: number[];
  };
}
export type HistoryReply = HistoryPage | { storage: LogStorage };
export type DatePreset = "all" | "day" | "today" | "week" | "custom";
export function localDateInput(date = new Date()): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
function localDay(text: string): Date {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(text))
    throw new Error("请选择有效的起止日期");
  const [year, month, day] = text.split("-").map(Number);
  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  )
    throw new Error("请选择有效日期");
  return date;
}
/** Calendar dates use local midnight, not a fixed 24-hour subtraction (DST safe). End date is inclusive in UI. */
export function historyDateBounds(
  preset: DatePreset,
  start: string,
  end: string,
  now = new Date(),
): { from?: number; to?: number } {
  if (preset === "all") return {};
  if (preset === "day")
    return { from: now.getTime() - 86400000, to: now.getTime() };
  const first =
    preset === "custom"
      ? localDay(start)
      : new Date(
          now.getFullYear(),
          now.getMonth(),
          now.getDate() - (preset === "week" ? 6 : 0),
        );
  const last =
    preset === "custom"
      ? localDay(end)
      : new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (first > last) throw new Error("开始日期不能晚于结束日期");
  last.setDate(last.getDate() + 1);
  return { from: first.getTime(), to: last.getTime() };
}
export function displayHistory(page: HistoryPage): HistoryPage {
  const [first, last] = page.analytics.bounds;
  const long = first !== null && last !== null && last - first >= 86400000;
  const date = (value: number) =>
    new Date(value).toLocaleString("zh-CN", { hour12: false });
  return {
    ...page,
    analytics: {
      ...page.analytics,
      rangeLabel:
        first === null || last === null
          ? "没有有效时间记录"
          : `${date(first)} – ${date(last)}`,
      timeline: page.analytics.timeline.map((point) => ({
        ...point,
        label: new Date(Number(point.id)).toLocaleString(
          "zh-CN",
          long
            ? {
                month: "2-digit",
                day: "2-digit",
                hour: "2-digit",
                minute: "2-digit",
                hour12: false,
              }
            : { hour: "2-digit", minute: "2-digit", hour12: false },
        ),
      })),
    },
  };
}
