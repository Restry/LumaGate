import { ActionButton } from "./ui";
import type { LogStorage } from "./log-history";
export function LogStorageRecovery({
  storage,
  running,
  busy,
  error,
  onRetry,
  onOpen,
}: {
  storage: LogStorage;
  running: boolean;
  busy: boolean;
  error: string | null;
  onRetry: () => void;
  onOpen: () => void;
}) {
  return (
    <section className="mg-log-recovery" aria-labelledby="log-storage-heading">
      <h2 id="log-storage-heading">日志存储需要处理</h2>
      <p role="alert">
        {storage.message} 应用配置仍可查看；不会自动删除、清空或重建原始日志。
      </p>
      <p>
        日志位置：<code>{storage.path}</code>
      </p>
      <ol>
        <li>
          检查是否还有另一个使用同一数据目录的应用实例，或磁盘已满、目录不可写。
        </li>
        <li>
          如怀疑文件损坏，先退出应用，保留数据库以及同目录的 WAL/SHM
          文件，再从可信备份恢复或寻求协助；不要直接删除文件。
        </li>
        <li>
          处理好原因后，选择“重试打开日志库”。此操作不重新启动网关，也不发送模型请求。
        </li>
      </ol>
      {running && (
        <p>
          网关仍在运行，但日志可能未能完整保存。请先在顶部手动停止网关，再重试打开日志库。
        </p>
      )}
      {error && <p role="alert">{error}</p>}
      <div className="mg-log-actions">
        <ActionButton disabled={busy || running} onClick={onRetry}>
          {busy ? "正在重新打开…" : "重试打开日志库"}
        </ActionButton>
        <ActionButton variant="outline" onClick={onOpen}>
          打开日志目录
        </ActionButton>
      </div>
    </section>
  );
}
