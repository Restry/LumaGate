import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { TokenUsage, UsageGrouping } from "./log-usage";
import { useLogPreference } from "./log-preferences";
import { LogStudio, type LogFilters } from "./LogStudio";
import {
  displayHistory,
  historyDateBounds,
  localDateInput,
  type HistoryPage,
  type HistoryQuery,
  type HistoryReply,
  type LogStorage,
} from "./log-history";

export type RequestCaller =
  | { kind: "key"; keyId: string; name: string }
  | { kind: "local" | "rejected" | "unknown" };
export interface RequestLog {
  id: number;
  startedAt: string;
  endpoint: string;
  model: string | null;
  status: number;
  responseMs: number;
  streaming: boolean;
  caller?: RequestCaller;
  providers?: { id: string; name: string; outcome: string }[];
  routeNote?: string | null;
  input?: unknown;
  response?: unknown;
  responseState?: string;
  completion?: "unknown" | "completed" | "failed" | "incomplete" | null;
  usage?: TokenUsage | null;
  result?: { state: "success" | "failed" | "pending"; label: string };
}
const blankFilters = (): LogFilters => ({
  model: "",
  provider: "all",
  caller: "all",
  endpoint: "all",
  status: "all",
  search: "",
  preset: "all",
  start: localDateInput(),
  end: localDateInput(),
});

export function RequestLogs({
  gatewayRunning = false,
  onStorageChange,
  active = true,
  navigation,
}: {
  gatewayRunning?: boolean;
  onStorageChange?: () => void;
  active?: boolean;
  navigation?: { id: number; filters: Partial<LogFilters> } | null;
} = {}) {
  const [filters, setFilters] = useState(blankFilters),
    [applied, setApplied] = useState(filters);
  const [page, setPage] = useState(0),
    [pageSize, setPageSize] = useState(50),
    [anchor, setAnchor] = useState<number | undefined>();
  const [grouping, setGrouping] = useState<UsageGrouping>("model");
  const loadedDates = useRef<{ from?: number; to?: number }>({});
  const [revision, setRevision] = useState(0),
    [loadedScope, setLoadedScope] = useState("");
  const [result, setResult] = useState<HistoryPage | null>(null),
    [storage, setStorage] = useState<LogStorage | null>(null);
  const [loading, setLoading] = useState(true),
    [error, setError] = useState(false),
    [recovering, setRecovering] = useState(false),
    [recoveryError, setRecoveryError] = useState<string | null>(null);
  const [showAnalysis, setShowAnalysis] = useLogPreference("analysis", false);
  const [providerLabel, setProviderLabel] = useState(""),
    [callerLabel, setCallerLabel] = useState("");
  useEffect(() => {
    if (!navigation) return;
    const next = { ...blankFilters(), ...navigation.filters };
    setFilters(next);
    setApplied(next);
    setPage(0);
    setAnchor(undefined);
    setShowAnalysis(false);
  }, [navigation?.id]);
  useEffect(() => {
    const timer = setTimeout(() => setApplied(filters), 200);
    return () => clearTimeout(timer);
  }, [filters]);
  const request = useMemo(() => {
    try {
      return {
        query: {
          ...(page === 0
            ? historyDateBounds(applied.preset, applied.start, applied.end)
            : loadedDates.current),
          model: applied.model,
          provider: applied.provider,
          caller: applied.caller,
          endpoint: applied.endpoint,
          status: applied.status,
          search: applied.search,
          page,
          pageSize,
          anchor: page === 0 ? undefined : anchor,
          grouping,
        } as HistoryQuery,
        error: null,
      };
    } catch (e) {
      return {
        query: null,
        error: e instanceof Error ? e.message : "日期无效",
      };
    }
  }, [applied, page, pageSize, anchor, grouping, revision]);
  const scopeKey = JSON.stringify([applied, page, pageSize, grouping, anchor]);
  useEffect(() => {
    if (!active) return;
    if (!request.query) {
      setLoading(false);
      return;
    }
    let current = true,
      timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      setLoading(true);
      try {
        const dates =
          page === 0
            ? historyDateBounds(applied.preset, applied.start, applied.end)
            : loadedDates.current;
        const query = { ...request.query!, from: dates.from, to: dates.to };
        const reply = await invoke<HistoryReply>("manual_query_logs", {
          query,
        });
        if (!current) return;
        if (!reply?.storage) throw new Error("Invalid history response");
        setStorage(reply.storage);
        if (!reply.storage.ready) {
          setResult(null);
          setError(false);
          return;
        }
        if (!("rows" in reply)) throw new Error("Missing history page");
        loadedDates.current = dates;
        setResult(displayHistory(reply));
        setLoadedScope(scopeKey);
        setError(false);
      } catch {
        if (current) setError(true);
      } finally {
        if (current) {
          setLoading(false);
          if (page === 0) timer = setTimeout(refresh, 3000);
        }
      }
    }
    void refresh();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [request, page, scopeKey, active]);
  function change<K extends keyof LogFilters>(key: K, value: LogFilters[K]) {
    setFilters((current) => ({ ...current, [key]: value }));
    setPage(0);
    setAnchor(undefined);
  }
  function newest() {
    setPage(0);
    setAnchor(undefined);
    setRevision((v) => v + 1);
  }
  function clear() {
    setFilters(blankFilters());
    setPage(0);
    setAnchor(undefined);
    setProviderLabel("");
    setCallerLabel("");
  }
  async function retry() {
    setRecovering(true);
    setRecoveryError(null);
    try {
      const next = await invoke<LogStorage>("manual_retry_log_storage");
      setStorage(next);
      if (next.ready) {
        newest();
        onStorageChange?.();
      }
    } catch {
      setRecoveryError(
        "仍无法打开日志库。请先停止网关、关闭其他实例并检查磁盘和权限，再重试；未清空历史。",
      );
    } finally {
      setRecovering(false);
    }
  }
  async function openDirectory() {
    try {
      await invoke("manual_open_log_directory");
    } catch {
      setRecoveryError(
        "无法打开目录，请手动在文件管理器中打开上面显示的路径。",
      );
    }
  }
  const options = result?.options;
  const providers = (options?.providers ?? []).map((p) => ({
    value: p.value,
    label: `${p.label} (${p.value.slice(0, 8)})`,
  }));
  if (
    filters.provider !== "all" &&
    !providers.some((p) => p.value === filters.provider)
  )
    providers.push({
      value: filters.provider,
      label: providerLabel || filters.provider,
    });
  const callers = (options?.callers ?? []).map((p) => ({
    value: p.value,
    label: p.value.startsWith("key:")
      ? `${p.label} (${p.value.slice(4, 12)})`
      : p.label,
  }));
  if (
    filters.caller !== "all" &&
    !callers.some((p) => p.value === filters.caller)
  )
    callers.push({
      value: filters.caller,
      label: callerLabel || filters.caller,
    });
  const endpoints = [
    ...new Set([
      "/v1/models",
      "/v1/messages",
      "/v1/chat/completions",
      "/v1/responses",
      "/v1/responses/compact",
      ...(options?.endpoints ?? []),
      ...(filters.endpoint !== "all" ? [filters.endpoint] : []),
    ]),
  ];
  const codes = [
    ...new Set([
      ...(options?.statuses ?? []),
      ...(/^\d+$/.test(filters.status) ? [Number(filters.status)] : []),
    ]),
  ].sort((a, b) => a - b);
  const scopePending =
    !result ||
    loadedScope !== scopeKey ||
    JSON.stringify(filters) !== JSON.stringify(applied);
  if (!active) return null;
  return (
    <LogStudio
      model={{
        filters,
        providers,
        callers,
        endpoints,
        codes,
        result,
        storage,
        loading,
        scopePending,
        error,
        dateError: request.error,
        recovering,
        recoveryError,
        gatewayRunning,
        showAnalysis,
        grouping,
        page,
        pageSize,
        tableKey: JSON.stringify([applied, page, pageSize]),
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      }}
      actions={{
        filter: (key, value) => {
          if (key === "provider")
            setProviderLabel(
              providers.find((p) => p.value === value)?.label ?? value,
            );
          if (key === "caller")
            setCallerLabel(
              callers.find((p) => p.value === value)?.label ?? value,
            );
          change(key, value);
        },
        clear,
        refresh: newest,
        analysis: setShowAnalysis,
        grouping: setGrouping,
        page: (next) => {
          if (result) {
            setAnchor(result.anchor);
            setPage(next);
          }
        },
        pageSize: (size) => {
          setPageSize(size);
          setPage(0);
          setAnchor(undefined);
        },
        retry: () => void retry(),
        openDirectory: () => void openDirectory(),
      }}
    />
  );
}
