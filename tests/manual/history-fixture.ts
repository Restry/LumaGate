import type { RequestLog } from "@/manual/RequestLogs";
import type { HistoryPage, HistoryQuery } from "@/manual/log-history";
import { usageAnalytics, cacheUsageAnalytics } from "@/manual/log-usage";
import { requestOutcome } from "@/manual/log-outcome";
export const readyStorage = {
  ready: true,
  path: "/isolated/logs/requests.sqlite3",
  message: "日志存储可用",
};
export function historyFixture(
  source: RequestLog[],
  query: Partial<HistoryQuery> = {},
): HistoryPage {
  const sorted = [...source].sort((a, b) => b.id - a.id),
    latest = Math.max(0, ...sorted.map((r) => r.id)),
    anchor = query.anchor ?? latest;
  const caller = (r: RequestLog) =>
    r.caller?.kind === "key"
      ? `key:${r.caller.keyId}`
      : (r.caller?.kind ?? "unknown");
  const matched = sorted.filter((r) => {
    const at = Date.parse(r.startedAt),
      response =
        r.response && typeof r.response === "object"
          ? (r.response as Record<string, unknown>)
          : {},
      error =
        response.error && typeof response.error === "object"
          ? (response.error as Record<string, unknown>)
          : {};
    return (
      r.id <= anchor &&
      (query.from == null || at >= query.from) &&
      (query.to == null || at < query.to) &&
      (!query.model ||
        (r.model ?? "")
          .toLowerCase()
          .includes(query.model.trim().toLowerCase())) &&
      (!query.provider ||
        query.provider === "all" ||
        r.providers?.some((p) => p.id === query.provider)) &&
      (!query.caller || query.caller === "all" || caller(r) === query.caller) &&
      (!query.endpoint ||
        query.endpoint === "all" ||
        r.endpoint === query.endpoint) &&
      (!query.status ||
        query.status === "all" ||
        query.status === requestOutcome(r).state ||
        String(r.status) === query.status) &&
      (!query.search ||
        String(r.id) === query.search.replace(/^#/, "") ||
        String(response.id ?? "")
          .toLowerCase()
          .includes(query.search.toLowerCase()) ||
        String(error.code ?? error.type ?? "")
          .toLowerCase()
          .includes(query.search.toLowerCase()))
    );
  });
  const providerMap = new Map<string, string>(),
    callerMap = new Map<string, string>();
  for (const r of sorted) {
    for (const p of r.providers ?? [])
      if (!providerMap.has(p.id)) providerMap.set(p.id, p.name);
    const id = caller(r);
    if (!callerMap.has(id))
      callerMap.set(
        id,
        r.caller?.kind === "key"
          ? r.caller.name
          : id === "local"
            ? "本机免鉴权"
            : id === "rejected"
              ? "鉴权拒绝"
              : "未记录的调用来源",
      );
  }
  const option = (map: Map<string, string>) =>
    [...map].map(([value, label]) => ({ value, label }));
  const pageSize = query.pageSize ?? 50,
    pages = Math.ceil(matched.length / pageSize),
    page = Math.min(query.page ?? 0, Math.max(0, pages - 1));
  const analytics = usageAnalytics(matched, query.grouping ?? "model"),
    times = matched.map((r) => Date.parse(r.startedAt)).filter(Number.isFinite);
  return {
    storage: readyStorage,
    rows: matched
      .slice(page * pageSize, (page + 1) * pageSize)
      .map((r) => ({ ...r, result: requestOutcome(r) })),
    matched: matched.length,
    page,
    pageSize,
    pages,
    anchor,
    newerAvailable: latest > anchor,
    overflow: false,
    stats: {
      total: matched.length,
      success: matched.filter((r) => requestOutcome(r).state === "success")
        .length,
      failed: matched.filter((r) => requestOutcome(r).state === "failed")
        .length,
      pending: matched.filter((r) => requestOutcome(r).state === "pending")
        .length,
    },
    analytics: {
      ...analytics,
      intervalMs: 60000,
      bounds: times.length
        ? [Math.min(...times), Math.max(...times)]
        : [null, null],
    },
    requestTimeline: [
      ...matched
        .reduce((map, row) => {
          const at = Math.floor(Date.parse(row.startedAt) / 3600000) * 3600000;
          if (!Number.isFinite(at)) return map;
          const point = map.get(at) ?? {
            at,
            success: 0,
            failed: 0,
            pending: 0,
          };
          point[requestOutcome(row).state] += 1;
          map.set(at, point);
          return map;
        }, new Map<number, { at: number; success: number; failed: number; pending: number }>())
        .values(),
    ].sort((a, b) => a.at - b.at),
    providerObservations: [
      ...new Map(
        matched.flatMap((row) => {
          const provider = row.providers?.at(-1);
          return provider ? [[provider.id, provider.name] as const] : [];
        }),
      ).entries(),
    ].map(([id, name]) => ({
      id,
      name,
      records: matched
        .filter((row) => row.providers?.at(-1)?.id === id)
        .slice(0, 20)
        .map((row) => ({
          id: row.id,
          at: Date.parse(row.startedAt),
          state: requestOutcome(row).state,
        })),
    })),
    recentFailures: matched
      .filter((row) => requestOutcome(row).state === "failed")
      .slice(0, 3)
      .map((row) => ({
        id: row.id,
        at: Date.parse(row.startedAt),
        model: row.model,
        provider: row.providers?.at(-1)?.name ?? null,
        http: row.status,
        code: requestOutcome(row).label,
      })),
    cache: cacheUsageAnalytics(matched),
    options: {
      providers: option(providerMap),
      callers: option(callerMap),
      endpoints: [...new Set(source.map((r) => r.endpoint))],
      statuses: [...new Set(source.map((r) => r.status))],
    },
  };
}
