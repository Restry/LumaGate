import type { RequestLog } from "./RequestLogs";

type Outcome = { state: "success" | "failed" | "pending"; label: string };
function object(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}
/** Only inspect protocol-envelope fields, never model text/tool output. Also works for saved v3.20.6 records. */
export function requestOutcome(row: RequestLog): Outcome {
  if (row.result && ["success", "failed", "pending"].includes(row.result.state))
    return row.result;
  const root = object(row.response);
  const response = object(root?.response) ?? root;
  const error = response?.error;
  const code = object(error)?.code ?? object(error)?.type;
  if (
    row.status === 429 ||
    ["rate_limit_exceeded", "rate_limit_error", "too_many_requests"].includes(
      String(code),
    )
  )
    return { state: "failed", label: "调用失败 · 限流（429）" };
  if (response?.status === "cancelled")
    return { state: "failed", label: "调用失败 · 已取消" };
  if (response?.status === "incomplete" || row.completion === "incomplete")
    return { state: "failed", label: "调用失败 · 输出未完成" };
  if (
    row.completion === "failed" ||
    row.status < 200 ||
    row.status >= 300 ||
    response?.status === "failed" ||
    (error !== undefined && error !== null) ||
    root?.type === "error" ||
    root?.type === "response.failed" ||
    row.responseState?.startsWith("调用失败") ||
    row.responseState?.startsWith("已中断") ||
    row.responseState === "传输错误"
  )
    return { state: "failed", label: "调用失败" };
  if (row.responseState === "接收中")
    return { state: "pending", label: "接收中" };
  if (row.completion === "unknown")
    return { state: "pending", label: "结束未确认" };
  const choices = Array.isArray(response?.choices) ? response.choices : [];
  const legacyFinish =
    choices.length > 0 &&
    choices.every((choice) =>
      ["stop", "tool_calls", "function_call"].includes(
        String(object(choice)?.finish_reason),
      ),
    );
  if (
    row.completion === "completed" ||
    response?.status === "completed" ||
    legacyFinish ||
    (!row.streaming && (!row.responseState || row.responseState === "已结束"))
  )
    return { state: "success", label: "调用成功" };
  return { state: "pending", label: "结果待确认" };
}
