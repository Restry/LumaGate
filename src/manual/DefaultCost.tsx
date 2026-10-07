import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ActionButton } from "./ui";

export interface DefaultCostData {
  totals: Record<string, string>;
  parts: Record<string, [string, string, string, string]>;
  covered: number;
  estimated: number;
  assumptions: { model: number; input: number };
  excluded: number;
  missing: { model: string; reason: string; requests: number }[];
  catalog: {
    source: string;
    fetchedAt: string;
    checkedAt: string | null;
    error: string | null;
    models: number;
    unit: string;
  };
}
function amount(value: string) {
  const number = Number(value);
  if (number > 0 && number < 0.000001) return "<0.000001";
  return new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: number < 0.01 ? 6 : 2,
  }).format(number);
}
export function DefaultCost({
  data,
  onRefresh,
}: {
  data?: DefaultCostData;
  onRefresh: () => void;
}) {
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const totals = Object.entries(data?.totals ?? {});
  const unpriced = new Set(
    data?.missing.filter((m) => m.reason === "未定价").map((m) => m.model),
  ).size;
  const problem = error || data?.catalog.error;
  async function refresh() {
    setRefreshing(true);
    setError(null);
    try {
      await invoke("manual_refresh_default_prices");
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshing(false);
      onRefresh();
    }
  }
  return (
    <article className="mc-cost-metric" aria-label="预估费用">
      <div className="mc-metric-label">
        <span>预估费用</span>
        <Popover>
          <PopoverTrigger asChild>
            <ActionButton
              className="mc-cost-trigger"
              variant="ghost"
              size="sm"
              aria-label="费用明细"
            >
              明细
            </ActionButton>
          </PopoverTrigger>
          <PopoverContent
            className="ls-popover mc-cost-details"
            align="end"
            aria-label="费用明细"
          >
            <h2>费用明细</h2>
            {data ? (
              <>
                {Object.entries(data.parts).map(([currency, parts]) => (
                  <dl key={currency} className="mc-cost-breakdown">
                    {parts.map((value, i) => (
                      <div key={i}>
                        <dt>{["输入", "输出", "缓存读取", "缓存写入"][i]}</dt>
                        <dd>
                          {currency} {amount(value)}
                        </dd>
                      </div>
                    ))}
                    <div className="mc-cost-total">
                      <dt>合计</dt>
                      <dd>
                        {currency} {data.totals[currency]}
                      </dd>
                    </div>
                  </dl>
                ))}
                <p>
                  已计入 {data.covered.toLocaleString()} 条
                  {data.excluded > 0
                    ? ` · 未计入 ${data.excluded.toLocaleString()} 条`
                    : ""}
                </p>
                {data.assumptions.input > 0 && (
                  <p>未记录的缓存拆分按普通输入价估算。</p>
                )}
                {data.assumptions.model > 0 && (
                  <p>
                    {data.assumptions.model.toLocaleString()}{" "}
                    条旧记录按请求模型估算。
                  </p>
                )}
                {data.missing.length > 0 && (
                  <details>
                    <summary>未计入记录</summary>
                    <ul>
                      {data.missing.map((m, i) => (
                        <li key={i}>
                          {m.model} · {m.reason} · {m.requests.toLocaleString()}{" "}
                          条
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
                <div className="mc-cost-source">
                  <span>
                    OpenAI 标准价 ·{" "}
                    {new Date(
                      data.catalog.checkedAt ?? data.catalog.fetchedAt,
                    ).toLocaleDateString("zh-CN")}
                  </span>
                  <ActionButton
                    variant="ghost"
                    size="sm"
                    disabled={refreshing}
                    onClick={() => void refresh()}
                  >
                    {refreshing ? "更新中…" : "更新价格"}
                  </ActionButton>
                </div>
              </>
            ) : (
              <p>等待读取用量</p>
            )}
            {problem && <p role="alert">{problem}</p>}
          </PopoverContent>
        </Popover>
      </div>
      <strong className="mc-cost-amount">
        {!data
          ? "—"
          : totals.length
            ? totals.map(([currency, value]) => (
                <span key={currency}>
                  <small>{currency}</small> {amount(value)}
                </span>
              ))
            : data.excluded > 0
              ? "未计价"
              : "—"}
      </strong>
      <p>
        {!data
          ? "等待读取"
          : problem
            ? "价格更新失败 · 使用缓存"
            : unpriced > 0
              ? `${unpriced} 个模型未定价`
              : data.excluded > 0
                ? `${data.excluded.toLocaleString()} 条用量不完整`
                : data.estimated > 0
                  ? "含估算"
                  : data.covered > 0
                    ? "默认单价"
                    : "暂无用量"}
      </p>
    </article>
  );
}
