# LumaGate 安全政策

请通过 [Restry/LumaGate 私密安全报告](https://github.com/Restry/LumaGate/security/advisories/new) 报告漏洞，避免公开可利用细节与用户凭据。如果私密报告入口不可用，可创建不含漏洞细节的 Issue 请求维护者提供安全联系渠道。

报告应包含受影响版本、操作系统、最小重现步骤、影响及脱敏证据。不要上传真实 API Key、Copilot token、访问密钥、完整数据库或用户请求正文。

LumaGate 是本机与可信 IPv4 内网工具，不是公网多租户服务。回环请求免鉴权，不得通过反向代理绕过网络边界；内网 HTTP 无传输加密。上游和内网密钥、Copilot 登录以私有权限文件保存，未做磁盘加密，备份同样需要私密保护。

安全维护范围以当前 LumaGate 代码和发行包为准。归档上游文档与旧版本记录不是当前安全承诺。
