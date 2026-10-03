import { useEffect, useRef } from "react";
import {
  ArrowLeft,
  ArrowUpRight,
  Copy,
  EyeOff,
  FileJson,
  Settings2,
} from "lucide-react";
import { ActionButton, Choice } from "./ui";
import { DeploymentTests } from "./DeploymentTests";
import { capabilitySummary } from "./catalog-view";
import {
  declaredInput,
  modalityLabel,
  protocolLabels,
  protocolPaths,
  reasoningLabel,
  type Group,
  type Protocol,
  type Snapshot,
  type Source,
} from "./types";

export interface ModelActions {
  busy: boolean;
  testingKey: string | null;
  onTest: (group: Group, providerId: string) => void;
  onRoute: (group: Group) => void;
  onMetadata: (group: Group) => void;
  onCopy: (group: Group) => void;
  onProtocol: (group: Group, protocol: Protocol | null) => void;
  onBlock: (modelId: string, blocked: boolean) => void;
  onEditSource: (source: Source) => void;
  onUse: () => void;
}

export function ModelDetail({
  group,
  snapshot,
  onClose,
  ...actions
}: ModelActions & {
  group: Group;
  snapshot: Snapshot;
  onClose: () => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus({ preventScroll: true });
  }, [group.id]);
  return (
    <section
      className="mg-model-detail"
      aria-label="模型详情"
      id="manual-model-detail"
    >
      <div className="mg-detail-top">
        <ActionButton variant="ghost" size="sm" onClick={onClose}>
          <ArrowLeft size={14} aria-hidden />
          返回列表
        </ActionButton>
        <ActionButton
          variant="ghost"
          size="sm"
          onClick={() => actions.onCopy(group)}
          aria-label={`复制模型 ID ${group.publicId ?? group.model.id}`}
        >
          <Copy size={14} aria-hidden />
          复制 ID
        </ActionButton>
      </div>
      <div className="mg-detail-body">
        <header className="mg-detail-title">
          <h2 ref={heading} tabIndex={-1}>
            {group.publicId ?? group.model.id}
          </h2>
          {group.model.name &&
            group.model.name !== (group.publicId ?? group.model.id) && (
              <p>{group.model.name}</p>
            )}
          <p>{capabilitySummary(group.model)}</p>
          <ActionButton onClick={actions.onUse} size="sm">
            在 Agent 中使用
            <ArrowUpRight size={14} aria-hidden />
          </ActionButton>
        </header>
        <section className="mg-detail-section">
          <div className="mg-detail-section-heading">
            <h3>来源与测试</h3>
            <span>{group.providerIds.length} 个来源</span>
          </div>
          <p className="mg-detail-hint">
            按来源发送一次文本请求。结果仅代表上次测试。
          </p>
          <DeploymentTests
            group={group}
            sources={snapshot.document.providers}
            tests={snapshot.document.tests}
            busy={actions.busy}
            testingKey={actions.testingKey}
            onTest={(id) => actions.onTest(group, id)}
            onEdit={actions.onEditSource}
          />
        </section>
        {group.routingError && (
          <div className="mg-routing-error" role="alert">
            <strong>路由需要检查</strong>
            <p>{group.routingError}</p>
            <ActionButton
              variant="outline"
              size="sm"
              disabled={actions.busy}
              onClick={() => actions.onRoute(group)}
            >
              检查路由策略
            </ActionButton>
          </div>
        )}
        <details className="mg-detail-disclosure">
          <summary>调用与路由设置</summary>
          <div className="mg-detail-settings">
            <label htmlFor="model-endpoint">调用 Endpoint</label>
            <Choice
              id="model-endpoint"
              label={`Endpoint ${group.model.id}`}
              disabled={actions.busy || group.model.id.startsWith("copilot/")}
              value={
                group.protocolMode === "mixed"
                  ? "mixed"
                  : group.protocolMode === "manual"
                    ? group.protocol
                    : "auto"
              }
              onChange={(value) => {
                if (value !== "mixed")
                  actions.onProtocol(
                    group,
                    value === "auto" ? null : (value as Protocol),
                  );
              }}
              options={[
                ...(group.protocolMode === "mixed"
                  ? [
                      {
                        value: "mixed",
                        label: "来源设置不一致",
                        disabled: true,
                      },
                    ]
                  : []),
                {
                  value: "auto",
                  label: `自动 · ${protocolLabels[group.automaticProtocol ?? group.protocol]}`,
                },
                ...Object.entries(protocolLabels).map(([value, label]) => ({
                  value,
                  label,
                })),
              ]}
            />
            <code>{protocolPaths[group.protocol as Protocol]}</code>
            {group.model.id.startsWith("copilot/") && (
              <p>
                Endpoint 与限额来自 Copilot
                目录；使用独立路由，不参与跨来源或跨模型回退。
              </p>
            )}
            {group.protocolMode === "mixed" && (
              <p>来源设置不一致，请选择统一规则或检查各来源。</p>
            )}
            <div className="mg-route-summary">
              <div>
                <strong>
                  {group.policy.balance === "priority"
                    ? "按优先顺序"
                    : "轮询分配"}
                </strong>
                <p>
                  {group.policy.failover
                    ? "允许同模型故障切换"
                    : "不进行故障切换"}{" "}
                  · {group.policy.fallbacks.length} 个后备模型
                </p>
              </div>
              <ActionButton
                variant="outline"
                size="sm"
                disabled={actions.busy || group.model.id.startsWith("copilot/")}
                onClick={() => actions.onRoute(group)}
                aria-label={`路由策略 ${group.model.id}`}
              >
                <Settings2 size={14} aria-hidden />
                设置
              </ActionButton>
            </div>
          </div>
        </details>
        <details className="mg-detail-disclosure">
          <summary>能力与原始信息</summary>
          <div className="mg-detail-settings">
            <dl className="mg-capability-grid">
              <dt>上下文</dt>
              <dd>{group.model.contextWindow?.toLocaleString() ?? "未知"}</dd>
              <dt>最大输出</dt>
              <dd>{group.model.maxOutputTokens?.toLocaleString() ?? "未知"}</dd>
              <dt>输入</dt>
              <dd>{modalityLabel(declaredInput(group.model))}</dd>
              <dt>输出</dt>
              <dd>{modalityLabel(group.model.outputModalities)}</dd>
              <dt>思考</dt>
              <dd>{reasoningLabel(group.model)}</dd>
              <dt>工具调用</dt>
              <dd>
                {group.model.tools == null
                  ? "未知"
                  : group.model.tools
                    ? "支持"
                    : "不支持"}
              </dd>
              <dt>请求模型 ID</dt>
              <dd>
                <code>{group.publicId ?? group.id}</code>
              </dd>
              {group.publicId && group.publicId !== group.id && (
                <>
                  <dt>兼容路由 ID</dt>
                  <dd>
                    <code>{group.id}</code>
                  </dd>
                </>
              )}
            </dl>
            <p>
              {group.model.limitsSource === "user_override"
                ? "上下文与最大输出按用户指定配置；实际可用上限仍受 Provider 限制，原始声明保留在元数据中。其他能力来自上游声明。"
                : "能力来自上游声明；未知不等于不支持。"}
            </p>
            <ActionButton
              variant="outline"
              size="sm"
              onClick={() => actions.onMetadata(group)}
              aria-label="查看元数据"
            >
              <FileJson size={14} aria-hidden />
              查看原始元数据
            </ActionButton>
          </div>
        </details>
        <div className="mg-detail-bottom">
          <ActionButton
            variant="ghost"
            size="sm"
            disabled={actions.busy}
            onClick={() => actions.onBlock(group.model.id, true)}
            aria-label={`屏蔽 ${group.model.id}`}
          >
            <EyeOff size={14} aria-hidden />
            屏蔽模型
          </ActionButton>
          <p>从路由、公开目录和同步列表移除，可在“已屏蔽”中恢复。</p>
        </div>
      </div>
    </section>
  );
}
