import {
  CheckCircle2,
  CircleHelp,
  AlertCircle,
  Play,
  Pencil,
} from "lucide-react";
import { ActionButton } from "./ui";
import { checkedAtLabel } from "./catalog-view";
import {
  declaredInput,
  modalityLabel,
  testKey,
  type Group,
  type Source,
  type TestResult,
} from "./types";

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
    <ul className="mg-source-tests" aria-label={`${group.model.id} 的来源测试`}>
      {order.map(({ providerId, index }) => {
        const source = sources.find((item) => item.id === providerId);
        const model = source?.models.find((item) => item.id === group.model.id);
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
              <ActionButton
                variant="outline"
                size="sm"
                disabled={busy}
                aria-label={`测试 ${group.model.id} · ${label}`}
                onClick={() => onTest(providerId)}
              >
                <Play size={12} aria-hidden />
                {testing ? "测试中…" : result ? "重新测试" : "测试来源"}
              </ActionButton>
            </div>
            <p className="mg-source-address">{source?.baseUrl ?? providerId}</p>
            {model && (declaredInput(model) || model.outputModalities) && (
              <p className="mg-source-capabilities">
                输入：{modalityLabel(declaredInput(model))} · 输出：
                {modalityLabel(model.outputModalities)}
              </p>
            )}
            <div
              className={`mg-source-result ${result ? (result.success ? "is-passed" : "is-failed") : ""}`}
            >
              <div className="mg-result-heading">
                <Icon size={15} aria-hidden />
                <strong>
                  {result
                    ? result.success
                      ? `上次通过 · ${result.latencyMs} ms`
                      : "上次测试失败"
                    : "尚未测试"}
                </strong>
              </div>
              {result ? (
                <>
                  <time dateTime={result.checkedAt}>
                    {checkedAtLabel(result.checkedAt)}
                  </time>
                  {!result.success && (
                    <p className="mg-test-error">{result.detail}</p>
                  )}
                </>
              ) : (
                <p>目录已收录，不代表已验证可调用。</p>
              )}
              {result && !result.success && (
                <div className="mg-recovery-actions">
                  <p>可重试此来源，或检查连接地址、密钥和上游部署。</p>
                  {source && (
                    <ActionButton
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() => onEdit(source)}
                      aria-label={`编辑来源 ${label}`}
                    >
                      <Pencil size={13} aria-hidden />
                      编辑来源
                    </ActionButton>
                  )}
                </div>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
