import { CheckCircle2, CircleHelp, AlertCircle, Play, Pencil } from "lucide-react";
import { ActionButton } from "./ui";
import {
  displayModel,
  protocolLabels,
  testKey,
  type Group,
  type Source,
  type TestResult,
} from "./types";

/**
 * 详情默认只回答「从哪几个来源、什么 Endpoint、能不能通」。
 * 每个来源一行：名字 + Endpoint + 测试按钮 + 上次结果。
 * 展开能力与原始信息才看限额与元数据。
 */
export function DeploymentTests({
  group,
  sources,
  tests,
  busy,
  testingKey,
  onTest,
  onEdit,
}: {
  group: Group;
  sources: Source[];
  tests: Record<string, TestResult>;
  busy: boolean;
  testingKey: string | null;
  onTest: (providerId: string) => void;
  onEdit: (source: Source) => void;
}) {
  const order = group.providerIds
    .map((providerId, index) => ({ providerId, index }))
    .sort(
      (a, b) =>
        Number(tests[testKey(group.id, b.providerId)]?.success === false) -
        Number(tests[testKey(group.id, a.providerId)]?.success === false),
    );
  return (
    <ul
      className="mg-source-tests"
      aria-label={`${displayModel(group)} 的来源测试`}
    >
      {order.map(({ providerId, index }) => {
        const source = sources.find((item) => item.id === providerId);
        const name = source?.name ?? group.providerNames[index] ?? providerId;
        const label =
          group.providerNames.filter((other) => other === name).length > 1
            ? `${name} · ${providerId.slice(0, 8)}`
            : name;
        const result = tests[testKey(group.id, providerId)];
        const testing = testingKey === testKey(group.id, providerId);
        const Icon = result
          ? result.success
            ? CheckCircle2
            : AlertCircle
          : CircleHelp;
        return (
          <li key={providerId} data-provider-id={providerId}>
            <div className="mg-source-heading">
              <h4>{label}</h4>
              <span className="mg-source-endpoint">
                {protocolLabels[group.protocol]}
              </span>
              <ActionButton
                variant="outline"
                size="sm"
                disabled={busy}
                aria-label={`测试 ${displayModel(group)} · ${label}`}
                onClick={() => onTest(providerId)}
              >
                <Play size={12} aria-hidden />
                {testing ? "测试中…" : result ? "重新测试" : "测试"}
              </ActionButton>
            </div>
            <div
              className={`mg-source-result ${result ? (result.success ? "is-passed" : "is-failed") : ""}`}
            >
              <Icon size={13} aria-hidden />
              <span>
                {result
                  ? result.success
                    ? `上次通过 · ${result.latencyMs} ms`
                    : "上次测试失败"
                  : "尚未测试"}
              </span>
            </div>
            {result && !result.success && (
              <div className="mg-recovery-actions">
                <p className="mg-test-error">{result.detail}</p>
                {source && (
                  <ActionButton
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() => onEdit(source)}
                    aria-label={`编辑来源 ${label}`}
                  >
                    <Pencil size={12} aria-hidden />
                    编辑来源
                  </ActionButton>
                )}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
