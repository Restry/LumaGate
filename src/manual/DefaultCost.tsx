import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Info } from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ActionButton } from "./ui";

export interface DefaultCostData {
  totals: Record<string, string>;
  parts: Record<
    string,
    [string | null, string | null, string | null, string | null]
  >;
  covered: number;
  partial: number;
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
function amount(currency: string, value: string) {
  const number = Number(value);
  if (number > 0 && number < 0.000001) return `${currency} <0.000001`;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
    currencyDisplay: "code",
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
    <section className="mc-default-cost" aria-label="默认单价估算费用">
      <div>
        <span>默认单价估算费用</span>
        <strong>
          {!data
            ? "—"
            : totals.length
              ? totals
                  .map(([currency, value]) => amount(currency, value))
                  .join(" + ")
              : "未计价"}
        </strong>
        <span>
          {data
            ? `完整 ${data.covered} · 部分 ${data.partial} · 未计入 ${data.excluded} 条`
            : "等待本机用量统计"}
        </span>
        {data && (data.partial > 0 || data.excluded > 0) && (
          <span>仅含已定价部分</span>
        )}
      </div>
      <Popover>
        <PopoverTrigger asChild>
          <ActionButton
            variant="ghost"
            size="sm"
            aria-label="费用口径与价格来源"
          >
            <Info size={14} />
            价格与覆盖
          </ActionButton>
        </PopoverTrigger>
        <PopoverContent className="ls-popover mc-cost-details" align="end">
          <h2>默认单价估算费用</h2>
          <p>
            按当前目录的标准单价重估本机所选范围内全部已记录最终上游用量，不是历史账单、订阅额度或节省金额。不含未记录的重试、工具费、区域附加费及特殊服务档位。
          </p>
          <p>
            缓存读写从输入中拆分，推理已包含在输出中。缓存计数或写入档位不完整时，只计入可确定部分；未定价和未知最终模型不填零。长上下文按单次输入超过
            272,000 Token 分档。
          </p>
          {data && (
            <>
              <p>
                完整 {data.covered} · 部分 {data.partial} · 未计入{" "}
                {data.excluded} 条推理记录
              </p>
              {Object.entries(data.parts).map(([currency, parts]) => (
                <dl key={currency} className="mc-cost-breakdown">
                  {parts.map((value, i) => (
                    <div key={i}>
                      <dt>
                        {
                          [
                            "非缓存输入",
                            "输出（含推理）",
                            "缓存读取",
                            "缓存写入",
                          ][i]
                        }
                      </dt>
                      <dd>
                        {value === null ? "未计入" : `${currency} ${value}`}
                      </dd>
                    </div>
                  ))}
                </dl>
              ))}
              {totals.map(([currency, value]) => (
                <p key={currency}>
                  精确合计：{currency} {value}
                </p>
              ))}
              {data.missing.length > 0 && (
                <details>
                  <summary>查看未计入或部分计价的模型</summary>
                  <ul>
                    {data.missing.map((m, i) => (
                      <li key={i}>
                        {m.model} · {m.reason} · {m.requests} 条
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              <p>
                价格来源：
                <a href={data.catalog.source} target="_blank" rel="noreferrer">
                  OpenAI 官方标准价表
                </a>{" "}
                · USD / 1M tokens · {data.catalog.models} 个精确模型
                ID；其他模型未收录。
              </p>
              <p>
                目录获取于{" "}
                {new Date(data.catalog.fetchedAt).toLocaleString("zh-CN")}。
                {data.catalog.checkedAt &&
                  `最近核验 ${new Date(data.catalog.checkedAt).toLocaleString("zh-CN")}。`}
                离线使用上次有效目录。
              </p>
            </>
          )}
          {(error || data?.catalog.error) && (
            <p role="alert">{error || data?.catalog.error}</p>
          )}
          <ActionButton
            variant="outline"
            size="sm"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {refreshing ? "正在获取默认单价…" : "更新默认单价"}
          </ActionButton>
          <p>仅下载公开价表，不上传模型目录、日志或凭据。</p>
        </PopoverContent>
      </Popover>
    </section>
  );
}
