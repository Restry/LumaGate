import { useState } from "react";
import {
  ArrowDownToLine,
  CheckCircle2,
  Copy,
  Play,
  Terminal,
} from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ActionButton } from "./ui";
import { checkedAtLabel } from "./catalog-view";
import type { Agent, Group } from "./types";

export interface SyncReceipt {
  appliedAt: string;
  files: { path: string; backup: string | null }[];
}
export const agentInfo: {
  id: Agent;
  name: string;
  summary: string;
  steps: string[];
  command?: string;
}[] = [
  {
    id: "pi",
    name: "Pi",
    summary: "合并网关模型目录，保留默认 Provider 与模型。",
    steps: [
      "预览将合并到 Pi 的模型目录，确认后写入。",
      "在 Pi 的模型选择器中选择本网关的模型；原默认选择不会被替换。",
    ],
  },
  {
    id: "claude",
    name: "Claude Code",
    summary: "同步连接信息与辅助模型清单，保留默认模型。",
    steps: [
      "预览并确认网关连接配置。辅助清单不是 Claude 的原生模型目录。",
      "在 Claude Code 中使用 /model 选择网关模型 ID；若默认模型不在目录中，需要手动选择。",
    ],
  },
  {
    id: "codex",
    name: "Codex",
    summary: "新增网关 profile，不更换默认 Provider 或模型。",
    steps: [
      "预览并确认 cc_switch_manual profile 和兼容模型目录。",
      "使用下方命令启用网关 profile，再选择目录中的模型。普通启动不会切换到网关。",
    ],
    command: "codex --profile cc_switch_manual",
  },
];

export function AgentWorkspace({
  disabled,
  canSync,
  selectedModel,
  receipts,
  onCopy,
  onModels,
  onLaunch,
  onSync,
}: {
  disabled: boolean;
  canSync: boolean;
  selectedModel?: Group;
  receipts: Partial<Record<Agent, SyncReceipt>>;
  onCopy: (text: string, label: string) => void;
  onModels: () => void;
  onLaunch: (agent: Agent, name: string) => void;
  onSync: (agent: Agent) => void;
}) {
  const [agent, setAgent] = useState<Agent>("pi");
  const item = agentInfo.find((entry) => entry.id === agent)!;
  const receipt = receipts[agent];
  return (
    <section className="mg-agents-workspace">
      <div className="mg-page-heading">
        <div>
          <h1>客户端</h1>
          <p>手动连接网关，保留你的默认选择。</p>
        </div>
      </div>
      {selectedModel && (
        <div className="mg-model-handoff">
          <div>
            <span>准备使用的模型</span>
            <strong>{selectedModel.publicId ?? selectedModel.model.id}</strong>
          </div>
          <ActionButton
            variant="outline"
            size="sm"
            onClick={() =>
              onCopy(selectedModel.publicId ?? selectedModel.id, "模型 ID")
            }
          >
            <Copy size={14} aria-hidden />
            复制模型 ID
          </ActionButton>
        </div>
      )}
      {!canSync && (
        <div className="mg-inline-notice">
          <p>还没有可同步的模型。先启用模型，再预览接入配置。</p>
          <ActionButton size="sm" variant="outline" onClick={onModels}>
            选择模型
          </ActionButton>
        </div>
      )}
      <Tabs
        orientation="vertical"
        value={agent}
        onValueChange={(value) => setAgent(value as Agent)}
      >
        <TabsList className="mg-view-tabs" aria-label="选择 Agent">
          {agentInfo.map((entry) => (
            <TabsTrigger
              className="mg-view-tab"
              key={entry.id}
              value={entry.id}
            >
              {entry.name}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value={agent} className="mg-agent-panel">
          <header>
            <Terminal size={24} aria-hidden />
            <div>
              <h2>{item.name}</h2>
              <p>{item.summary}</p>
            </div>
          </header>
          <ol className="mg-connection-steps">
            <li>
              <strong>预览并确认</strong>
              <p>{item.steps[0]}</p>
            </li>
            <li>
              <strong>在客户端使用</strong>
              <p>{item.steps[1]}</p>
              {item.command && (
                <div className="mg-command">
                  <code>{item.command}</code>
                  <ActionButton
                    variant="outline"
                    size="sm"
                    onClick={() => onCopy(item.command!, "网关启动命令")}
                    aria-label="复制网关启动命令"
                  >
                    <Copy size={14} aria-hidden />
                    复制命令
                  </ActionButton>
                </div>
              )}
            </li>
          </ol>
          <div className="mg-agent-panel-actions">
            <ActionButton
              disabled={disabled || !canSync}
              onClick={() => onSync(agent)}
            >
              <ArrowDownToLine size={15} aria-hidden />
              预览同步
            </ActionButton>
            <ActionButton
              variant="outline"
              disabled={disabled}
              onClick={() => onLaunch(agent, item.name)}
            >
              <Play size={14} aria-hidden />
              启动 {item.name}
            </ActionButton>
            <span>启动只打开客户端，不写入配置。</span>
          </div>
          {receipt && (
            <section
              className="mg-sync-receipt"
              aria-label={`${item.name} 同步结果`}
            >
              <div className="mg-result-heading">
                <CheckCircle2 size={17} aria-hidden />
                <h3>本次会话已同步</h3>
              </div>
              <p>
                <time dateTime={receipt.appliedAt}>
                  {checkedAtLabel(receipt.appliedAt)}
                </time>{" "}
                · 已写入 {receipt.files.length} 个文件，不代表客户端已连接。
              </p>
              <details>
                <summary>查看写入文件与备份</summary>
                <ul>
                  {receipt.files.map((file) => (
                    <li key={file.path}>
                      <code>{file.path}</code>
                      <span>
                        {file.backup
                          ? `备份：${file.backup}`
                          : "新建文件，无旧文件备份"}
                      </span>
                    </li>
                  ))}
                </ul>
              </details>
            </section>
          )}
        </TabsContent>
      </Tabs>
      <p className="mg-agent-footnote">
        只有确认同步才会写入 Agent 配置；不会改动 MCP、Skills 或
        Prompts。实际请求是否经过本网关，请在调用日志中核对。
      </p>
    </section>
  );
}
