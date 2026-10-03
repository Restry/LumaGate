import { useId, useState } from "react";
import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { ActionButton } from "./ui";
import type { Model } from "./types";

export function ProviderModelPicker({
  models,
  disabled,
  onChange,
}: {
  models: Model[];
  disabled: boolean;
  onChange: (models: Model[]) => void;
}) {
  const prefix = useId();
  const [query, setQuery] = useState("");
  const needle = query.trim().toLocaleLowerCase();
  const matches = models
    .map((model, index) => ({ model, index }))
    .filter(({ model }) =>
      `${model.id} ${model.name ?? ""}`.toLocaleLowerCase().includes(needle),
    );
  const enabled = models.filter((model) => model.enabled).length;
  function setMatches(value: boolean) {
    const ids = new Set(matches.map(({ model }) => model.id));
    onChange(
      models.map((model) =>
        ids.has(model.id) ? { ...model, enabled: value } : model,
      ),
    );
  }
  return (
    <section
      className="mg-provider-models mg-field--wide"
      aria-labelledby={`${prefix}-title`}
    >
      <div className="mg-model-picker-heading">
        <h3 id={`${prefix}-title`}>启用模型</h3>
        <span role="status">
          已启用 {enabled} / {models.length}
        </span>
      </div>
      {models.length > 0 ? (
        <>
          <div className="mg-search">
            <Search size={15} aria-hidden />
            <Input
              type="search"
              aria-label="搜索 Provider 模型"
              placeholder="搜索模型名称…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                // 搜索的 Enter 不应意外提交整个 Provider 表单。
                if (event.key === "Enter" && !event.nativeEvent.isComposing)
                  event.preventDefault();
              }}
            />
          </div>
          <div className="mg-model-picker-actions">
            <span>匹配 {matches.length} 个</span>
            <ActionButton
              type="button"
              size="sm"
              variant="ghost"
              disabled={disabled || matches.length === 0}
              onClick={() => setMatches(true)}
            >
              启用筛选结果
            </ActionButton>
            <ActionButton
              type="button"
              size="sm"
              variant="ghost"
              disabled={disabled || matches.length === 0}
              onClick={() => setMatches(false)}
            >
              停用筛选结果
            </ActionButton>
          </div>
          <ul className="mg-model-picks" aria-label="Provider 模型选择">
            {matches.map(({ model, index }) => (
              <li key={model.id}>
                <label
                  className="mg-model-pick"
                  htmlFor={`${prefix}-${index}`}
                  data-disabled={disabled}
                >
                  <span className="mg-model-pick-name">
                    <strong>{model.id}</strong>
                    {model.name && model.name !== model.id && (
                      <small>{model.name}</small>
                    )}
                  </span>
                  <span className="mg-model-pick-state" aria-hidden>
                    {model.enabled ? "已启用" : "已停用"}
                  </span>
                  <Switch
                    id={`${prefix}-${index}`}
                    aria-label={`启用模型 ${model.id}`}
                    checked={model.enabled}
                    disabled={disabled}
                    onCheckedChange={(enabled) =>
                      onChange(
                        models.map((item) =>
                          item.id === model.id ? { ...item, enabled } : item,
                        ),
                      )
                    }
                  />
                </label>
              </li>
            ))}
          </ul>
          {matches.length === 0 && (
            <p className="mg-empty-inline">没有匹配的模型。</p>
          )}
        </>
      ) : (
        <p className="mg-empty-inline">
          暂无模型。保存 Provider 后先拉取模型。
        </p>
      )}
    </section>
  );
}
