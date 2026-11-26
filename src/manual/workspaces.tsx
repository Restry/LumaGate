export { DashboardShell as Shell } from "./DashboardShell";

/**
 * 工作区标识。来源与模型合并成一个工作区：
 * 来源是模型的属性，不再是一个独立目的地。
 */
export type Workspace =
  | "overview"
  | "sources"
  | "agents"
  | "logs"
  | "settings"
  | "help";

export type ProviderFeedback = Record<string, { text: string; error: boolean }>;
