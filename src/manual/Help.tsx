import { Bot } from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

export function HelpContent() {
  return (
    <>
      <h2>使用说明</h2>
      <dl>
        <dt>模型与高可用</dt>
        <dd>
          同模型、同 Endpoint
          自动合并，默认轮询和故障切换；不同版本或前缀不混并。模态展示已有声明的合集，图片只走声明支持的来源，不会去图重试。原始条目可单独查看。
        </dd>
        <dt>上下文与输出上限</dt>
        <dd>
          GPT 5.6、GPT 6、Kimi、GLM 系列按你指定的规则配置为 1,000,000
          上下文、128,000 最大输出 Token，支持 FW
          前缀和命名空间。原始声明保留；这不是上游能力验证，也不会让每次请求自动输出到上限。
          升级后目录自动生效，Agent 文件仍需预览并确认同步。
        </dd>
        <dt>Endpoint</dt>
        <dd>
          GPT 5.5 及以上默认 Responses；Gemini 和其他模型默认 Chat
          Completions。可在模型详情的“调用与路由设置”中覆盖。
        </dd>
        <dt>测试与目录</dt>
        <dd>
          模型详情中优先展示失败记录，不改变路由顺序。点击某个来源的测试按钮，只向该
          Provider
          发送一次文本请求，可能计费，不重试或切换。结果按来源保存，不代表已验证图像或工具能力。目录来自
          /v1/models。
        </dd>
        <dt>屏蔽模型</dt>
        <dd>
          屏蔽后退出路由和可同步列表，在“已屏蔽”视图中恢复；恢复保留各来源的原有选择。已同步的客户端文件不会自动修改。
        </dd>
        <dt>Agent 配置</dt>
        <dd>
          启动不改配置。只有同步预览后的确认会写入受管字段，并保留备份、默认模型及其他设置。
        </dd>
        <dt>监听与内网访问</dt>
        <dd>
          在“设置”里选择仅本机或允许内网访问，并配置端口。内网请求必须携带独立密钥；HTTP
          不加密，只用于可信内网。监听地址/端口变更需手动停止并启动才生效，不会自动重启或修改
          Agent。停止会中断请求，但保留已保存日志。端口变化后，请重新预览并确认
          Agent 同步。
        </dd>
        <dt>访问密钥</dt>
        <dd>
          支持多把命名密钥，保存后可随时复制。明文保存在私有 access-keys
          文件夹，不放进日志。旧版密钥需补录原值一次才能复制。新增、停用和删除对后续请求即时生效，无需重启；已经开始的响应不会被主动打断。
        </dd>
        <dt>本机访问与日志</dt>
        <dd>
          本机进程调用无需连接标记，网关拒绝外部网页请求。日志查询整个本地数据库，支持全部时间、今天、最近七天、自选日期；按本机时区解释日期并包含结束当天。可搜索模型、请求序号/上游响应
          ID 和错误码，按 Provider、接口、访问密钥与结果组合筛选，每页 25/50/100
          条。在“日志 /
          分析”间切换，视图和图表选择会记住。接口、密钥、结果与请求检索在“筛选”中，详细规则在“口径”中。日志显示密钥名称；未使用有效密钥的本机请求单独标注。
          展开可看最后一轮 Input、限长脱敏 Response 和 Provider
          尝试顺序；日志与用量保存在
          ~/.lumagate/logs/requests.sqlite3，停止或重启后仍可查看。更早记录仍在数据库，暂不自动清理。只保存限长脱敏预览，隐藏图片内容和已知凭据；数据库未加密，预览仍可能包含业务内容。
          普通可重放 Chat
          会话优先原来源，开启故障回退时可尝试同模型的其他来源；成功后更新偏好。Responses、加密或签名状态续轮仍固定来源，缺少旧绑定时不能猜账号。来源偏好与绑定在重启后保留；已开始输出不切换。耗时到返回响应头，流式
          200
          不代表生成完成或调用成功。日志会另外识别响应中的失败和限流；失败筛选包含
          HTTP 200
          内的模型错误。传输结束和模型终态分别记录；缺少模型终态证据的旧流式记录显示待确认，与进行中记录一样不计入成功率。
        </dd>
        <dt>Token 统计</dt>
        <dd>
          汇总和图表覆盖当前筛选的全部历史记录，不受分页影响，只累计完整报告的上游用量，缺失不当成
          0。Provider
          匹配任一实际尝试，成功率仍按请求最终结果，不代表单家来源的尝试成功率。概览和列表用
          K/M
          简写，悬浮查看完整数字；统计数字也可用键盘聚焦，展开明细保留完整整数。输入含缓存，推理
          Token
          包含在输出中，不重复累加。流式用量需可靠的结束信息；未完成记录单独提示。
          这不是完整账单，不包含未报告的失败重试消耗，也不会按正文长度估算。
        </dd>
        <dt>预估费用</dt>
        <dd>
          选择“全部”查看累计费用；24 小时与7天只统计各自范围。 默认单价来自{" "}
          <a
            href="https://developers.openai.com/api/docs/pricing.md"
            target="_blank"
            rel="noreferrer"
          >
            OpenAI 官方标准价表
          </a>
          ，USD / 百万 Token，按当前目录重估历史。
          优先使用最终路由模型、其次响应模型；旧记录缺失时按精确请求模型估算，不代表已确认实际服务模型。
          已报告缓存单独计价，未记录的拆分按普通输入价估算；推理包含在输出中。
          未收录模型和缺少用量的记录不计入。费用明细可更新价格；失败保留缓存，不上传本机日志。
          估算不含未记录重试、工具费或额外档位费用，不是实际账单。
        </dd>
        <dt>日志库故障恢复</dt>
        <dd>
          日志库打不开时，应用仍可查看配置，日志页显示故障和路径；不会自动删除或重建原始日志。先检查其他实例、磁盘和权限，疑似损坏时退出应用并保留数据库及
          WAL/SHM
          文件，再从可信备份恢复。手动停止网关后可重试打开日志库；恢复不会自动启动网关。
        </dd>
        <dt>GitHub Copilot 登录</dt>
        <dd>
          点击 Provider 页的 GitHub Copilot 即生成设备码，再点“复制代码并打开
          GitHub”完成浏览器授权。授权页可能显示 VS
          Code，这是编辑器兼容接入身份，不是 GitHub
          官方网关产品。成功后只获取目录，不自动测试或同步客户端；模型权限以
          GitHub 返回为准。
        </dd>
        <dt>枢光名称与数据兼容</dt>
        <dd>
          LumaGate 数据保存在 ~/.lumagate；自身旧目录经备份与核验后迁移，
          不读取独立上游目录。稳定路由、访问密钥和已同步的 cc_switch_manual
          profile 保持有效，不需要重配客户端。
        </dd>
        <dt>Provider 密钥</dt>
        <dd>
          明文保存在 ~/.lumagate/keys/，不使用钥匙串、不写入 Agent
          配置。编辑时留空保留。
        </dd>
      </dl>
    </>
  );
}
export function Help() {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="mg-help-trigger"
          aria-label="Bot 使用说明"
        >
          <Bot size={18} aria-hidden />
          <span>使用说明</span>
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="start"
        sideOffset={12}
        className="mg-help"
        aria-label="使用说明"
      >
        <HelpContent />
      </PopoverContent>
    </Popover>
  );
}
export function HelpPage() {
  return (
    <section className="mc-help-workspace">
      <header className="mg-page-heading">
        <h1>帮助</h1>
      </header>
      <p className="mc-help-intro">
        先接入
        Provider，再选择模型、预览客户端同步。配置、目录、测试和真实请求结果分别呈现。
      </p>
      <div className="mc-help-content">
        <HelpContent />
      </div>
    </section>
  );
}
