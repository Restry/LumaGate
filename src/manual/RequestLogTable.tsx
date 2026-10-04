import { useId, useRef, useState } from "react";
import { Check, ChevronRight, Clock3, X } from "lucide-react";
import { ActionButton } from "./ui";
import { TokenUsageCell, TokenUsageDetails } from "./TokenUsagePanel";
import { requestOutcome } from "./log-outcome";
import type { RequestLog } from "./RequestLogs";
const preview = (value: unknown) =>
  value == null
    ? "暂无内容"
    : typeof value === "string"
      ? value
      : JSON.stringify(value, null, 2);
function caller(row: RequestLog) {
  return row.caller?.kind === "key"
    ? row.caller.name
    : row.caller?.kind === "local"
      ? "本机免鉴权"
      : row.caller?.kind === "rejected"
        ? "鉴权拒绝"
        : "未记录（旧日志）";
}
function endpointLabel(value: string) {
  return (
    {
      "/v1/responses": "Responses",
      "/v1/responses/compact": "Responses · compact",
      "/v1/chat/completions": "Chat",
      "/v1/messages": "Messages",
      "/v1/models": "模型目录",
    }[value] ?? value
  );
}
function resultLabel(value: string) {
  return value === "调用成功"
    ? "成功"
    : value.replace(/^调用失败(?: · )?/, "") || "失败";
}
function Outcome({ row }: { row: RequestLog }) {
  const outcome = requestOutcome(row);
  return (
    <span
      className={`ls-outcome ${outcome.state === "success" ? "mg-log-ok" : outcome.state === "failed" ? "mg-inline-error" : "mg-token-muted"}`}
      title={outcome.label}
    >
      {outcome.state === "success" ? (
        <Check size={12} />
      ) : outcome.state === "failed" ? (
        <X size={12} />
      ) : (
        <Clock3 size={12} />
      )}
      {resultLabel(outcome.label)}
    </span>
  );
}
export function RequestLogTable({ rows }: { rows: RequestLog[] }) {
  const prefix = useId(),
    trigger = useRef<HTMLButtonElement | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const key = (row: RequestLog) => `${row.id}:${row.startedAt}`;
  const detail = rows.find((row) => key(row) === selected);
  const close = () => {
    setSelected(null);
    trigger.current?.focus({ preventScroll: true });
  };
  const toggle = (row: RequestLog) =>
    setSelected((current) => (current === key(row) ? null : key(row)));
  const outcome = detail ? requestOutcome(detail) : null;
  return (
    <div className={`mc-log-layout ${detail ? "has-inspector" : ""}`}>
      <div
        className="mg-table-wrap"
        tabIndex={0}
        role="region"
        aria-label="请求表格，可左右滚动"
      >
        <table className="mg-log-table" aria-label="网关调用日志">
          <thead>
            <tr>
              <th scope="col">时间 / ID</th>
              <th scope="col">模型 / 接口</th>
              <th scope="col">密钥</th>
              <th scope="col">来源</th>
              <th scope="col">结果</th>
              <th scope="col">Token</th>
              <th scope="col" title="响应头耗时，不代表完整生成时长">
                头耗时
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const open = key(row) === selected,
                date = new Date(row.startedAt),
                valid = Number.isFinite(date.getTime());
              return (
                <tr
                  key={key(row)}
                  className="mg-log-entry"
                  data-expanded={open}
                  onClick={() => {
                    if (!window.getSelection()?.toString()) toggle(row);
                  }}
                >
                  <td>
                    <div className="mg-log-time">
                      <ActionButton
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="mg-log-toggle"
                        aria-label={`${open ? "收起" : "展开"}请求 ${row.id} 的详情`}
                        aria-expanded={open}
                        aria-controls={open ? `${prefix}-detail` : undefined}
                        onClick={(event) => {
                          event.stopPropagation();
                          trigger.current = event.currentTarget;
                          toggle(row);
                        }}
                      >
                        <ChevronRight
                          size={14}
                          aria-hidden
                          className={open ? "rotate-90" : undefined}
                        />
                      </ActionButton>
                      <time dateTime={row.startedAt} title={row.startedAt}>
                        {valid ? (
                          <>
                            <span>
                              {date.toLocaleTimeString("zh-CN", {
                                hour12: false,
                              })}
                            </span>
                            <small>
                              #{row.id} · {date.toLocaleDateString("zh-CN")}
                            </small>
                          </>
                        ) : (
                          "时间未记录"
                        )}
                      </time>
                    </div>
                  </td>
                  <td className="mg-log-model" title={row.model ?? undefined}>
                    {row.model ?? "—"}
                    <span className="ls-endpoint" title={row.endpoint}>
                      {endpointLabel(row.endpoint)}
                    </span>
                  </td>
                  <td
                    className="mg-log-caller"
                    title={
                      row.caller?.kind === "key" ? row.caller.keyId : undefined
                    }
                  >
                    {row.caller?.kind === "local" ? "本机" : caller(row)}
                  </td>
                  <td>
                    {row.providers?.at(-1)?.name ??
                      (row.providers ? "未发出" : "—")}
                  </td>
                  <td className="mg-log-result">
                    <Outcome row={row} />
                    <small>
                      HTTP {row.status}
                      {row.streaming ? " · 流式" : ""}
                    </small>
                  </td>
                  <td className="mg-log-usage">
                    <TokenUsageCell row={row} />
                  </td>
                  <td title={`${row.responseMs} ms · 响应头耗时`}>
                    {row.responseMs < 1000
                      ? `${row.responseMs} ms`
                      : `${(row.responseMs / 1000).toFixed(2)} s`}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {detail && (
        <section
          id={`${prefix}-detail`}
          className="mg-log-details mc-log-inspector"
          role="region"
          aria-label={`请求 ${detail.id} 的详情`}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              close();
            }
          }}
        >
          <header>
            <div>
              <span>请求详情</span>
              <h2>#{detail.id}</h2>
            </div>
            <ActionButton
              variant="ghost"
              size="icon"
              aria-label="关闭请求详情"
              onClick={close}
            >
              <X size={17} />
            </ActionButton>
          </header>
          <div className="mc-inspector-status">
            <Outcome row={detail} />
            <span>
              HTTP {detail.status}
              {detail.streaming ? " · 流式" : ""}
            </span>
          </div>
          <div className="mc-log-inspector-body">
            <p>
              请求 ID：#{detail.id} · 访问来源：{caller(detail)}
              {detail.caller?.kind === "key" ? ` · ${detail.caller.keyId}` : ""}
            </p>
            <p>
              模型：{detail.model ?? "—"} · 接口：{detail.endpoint} · 响应头：
              {detail.responseMs} ms
            </p>
            {detail.routeNote && <p>{detail.routeNote}</p>}
            {!!detail.providers?.length && (
              <p aria-label="Provider 尝试顺序">
                {detail.providers
                  .map((p) => `${p.name} (${p.id.slice(0, 8)}) · ${p.outcome}`)
                  .join(" → ")}
              </p>
            )}
            {outcome?.state !== "success" && (
              <div className="mc-notice">
                {outcome?.state === "failed"
                  ? "请求失败；HTTP 200 也可能在响应中报告模型错误。"
                  : "未确认模型终态，不计入成功率。"}
              </div>
            )}
            <TokenUsageDetails row={detail} />
            <details open>
              <summary>Input · 最后一轮</summary>
              <pre>{preview(detail.input)}</pre>
            </details>
            <details open>
              <summary>Response · {outcome?.label}</summary>
              <p>
                传输状态：
                {detail.delivery
                  ? { eof: "已结束", dropped: "已中断", error: "传输错误" }[
                      detail.delivery.transport
                    ]
                  : (detail.responseState ?? "旧日志未记录")}
                {outcome?.state === "pending" &&
                detail.responseState !== "接收中"
                  ? " · 未记录或未确认模型终态，未计入成功率"
                  : ""}
              </p>
              {detail.delivery && (
                <p>
                  下游协议：
                  {
                    {
                      completed: "已完成",
                      incomplete: "输出未完成",
                      failed: "失败",
                      unknown: "结束未确认",
                    }[detail.delivery.completion]
                  }
                  。仅为网关观察结果，不代表客户端确认接收。
                </p>
              )}
              <pre>
                {detail.responseState === "接收中" && detail.response == null
                  ? "正在接收响应…"
                  : preview(detail.response)}
              </pre>
            </details>
          </div>
        </section>
      )}
    </div>
  );
}
