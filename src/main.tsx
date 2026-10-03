import React from "react";
import ReactDOM from "react-dom/client";
import ManualApp from "./manual/ManualApp";
import "./index.css";

// 手动版不导入上游的更新、自动同步、设备配置初始化等启动副作用。
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ManualApp initialWorkspace="overview" />
  </React.StrictMode>,
);
