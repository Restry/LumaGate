//! Current public standard list prices, never provider invoices or subscription credits.
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock, RwLock},
    time::Duration,
};

pub const SOURCE: &str = "https://developers.openai.com/api/docs/pricing.md";
const LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
pub struct Rates {
    pub input: Decimal,
    pub read: Option<Decimal>,
    pub write: Option<Decimal>,
    pub output: Decimal,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Price {
    pub currency: String,
    pub short: Rates,
    pub long: Option<Rates>,
    pub threshold: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema: u32,
    pub source: String,
    pub fetched_at: String,
    pub unit: String,
    pub models: BTreeMap<String, Price>,
}
impl Catalog {
    pub fn bundled() -> Self {
        serde_json::from_str(include_str!("default-prices.json"))
            .expect("verified bundled price catalog")
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.source != SOURCE
            || self.unit != "per 1M tokens"
            || self.models.is_empty()
            || self.models.len() > 1000
            || chrono::DateTime::parse_from_rfc3339(&self.fetched_at).is_err()
        {
            return Err("价格目录格式已变化".into());
        }
        for (id, p) in &self.models {
            if id.is_empty() || id.len() > 160 || p.currency != "USD" || p.threshold != 272000 {
                return Err("价格目录模型或单位无效".into());
            }
            for r in std::iter::once(&p.short).chain(p.long.iter()) {
                for n in [Some(r.input), r.read, r.write, Some(r.output)]
                    .into_iter()
                    .flatten()
                {
                    if n < Decimal::ZERO || n > Decimal::from(1_000_000) || n.scale() > 9 {
                        return Err("价格数值无效".into());
                    }
                }
            }
        }
        Ok(())
    }
    pub fn parse(text: &str, at: String) -> Result<Self, String> {
        if !text.contains("Prices per 1M tokens.")
            || !text
                .contains("Short context: ≤272K input tokens. Long context: >272K input tokens.")
        {
            return Err("价格单位或长上下文规则已变化，保留上次目录".into());
        }
        let section = text
            .split_once("### Standard pricing data")
            .and_then(|(_, s)| s.split_once("### Batch pricing data"))
            .map(|(s, _)| s)
            .ok_or("未找到标准价格表")?;
        let mut models = BTreeMap::new();
        let amount = |s: &str| -> Result<Option<Decimal>, String> {
            if s == "-" {
                return Ok(None);
            }
            Decimal::from_str(s.strip_prefix('$').ok_or("未知货币")?)
                .map(Some)
                .map_err(|_| "无效价格".into())
        };
        for line in section.lines().filter(|s| s.starts_with('|')) {
            let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
            if cells
                .first()
                .is_some_and(|s| *s == "Model" || s.starts_with("---"))
            {
                continue;
            }
            if cells.len() != 9 {
                return Err("标准价格表列已变化".into());
            }
            let rates = |v: &[&str]| -> Result<Rates, String> {
                Ok(Rates {
                    input: amount(v[0])?.ok_or("缺少输入价格")?,
                    read: amount(v[1])?,
                    write: amount(v[2])?,
                    output: amount(v[3])?.ok_or("缺少输出价格")?,
                })
            };
            let id = cells[0]
                .strip_suffix(" (<272K context length)")
                .unwrap_or(cells[0])
                .to_owned();
            let long = if cells[5..].iter().all(|s| *s == "-") {
                None
            } else {
                Some(rates(&cells[5..])?)
            };
            if models
                .insert(
                    id,
                    Price {
                        currency: "USD".into(),
                        short: rates(&cells[1..5])?,
                        long,
                        threshold: 272000,
                    },
                )
                .is_some()
            {
                return Err("重复模型价格".into());
            }
        }
        if models.len() < 10 {
            return Err("标准价格表不完整".into());
        }
        let result = Self {
            schema: 1,
            source: SOURCE.into(),
            fetched_at: at,
            unit: "per 1M tokens".into(),
            models,
        };
        result.validate()?;
        Ok(result)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    pub catalog: Catalog,
    etag: Option<String>,
    checked_at: Option<String>,
}
#[derive(Clone)]
struct State {
    cache: Arc<Cache>,
    error: Option<String>,
}
pub struct Service {
    state: RwLock<State>,
    refresh: tokio::sync::Mutex<()>,
}
impl Service {
    fn new() -> Self {
        Self {
            state: RwLock::new(State {
                cache: Arc::new(Cache {
                    catalog: Catalog::bundled(),
                    etag: None,
                    checked_at: None,
                }),
                error: None,
            }),
            refresh: tokio::sync::Mutex::new(()),
        }
    }
    pub fn load(&self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        let loaded = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= LIMIT as u64)
            .and_then(|_| std::fs::read(path).ok())
            .and_then(|b| serde_json::from_slice::<Cache>(&b).ok())
            .filter(|c| c.catalog.validate().is_ok());
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        match loaded {
            Some(cache) => state.cache = Arc::new(cache),
            None => state.error = Some("本地价格缓存无效，使用随应用附带的来源目录".into()),
        }
    }
    pub fn snapshot(&self) -> (Arc<Cache>, Value) {
        let s = self.state.read().unwrap_or_else(|e| e.into_inner());
        (
            s.cache.clone(),
            json!({"source":s.cache.catalog.source,"fetchedAt":s.cache.catalog.fetched_at,"checkedAt":s.cache.checked_at,"error":s.error,"models":s.cache.catalog.models.len(),"unit":s.cache.catalog.unit}),
        )
    }
    fn accept(
        &self,
        text: Option<&str>,
        etag: Option<String>,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut cache = (*self.state.read().unwrap_or_else(|e| e.into_inner()).cache).clone();
        if let Some(text) = text {
            cache.catalog = Catalog::parse(text, now.clone())?;
            cache.etag = etag;
        }
        cache.checked_at = Some(now);
        let bytes = serde_json::to_vec(&cache).map_err(|_| "价格缓存序列化失败")?;
        crate::config::atomic_write(path, &bytes).map_err(|_| "价格缓存保存失败，保留上次目录")?;
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.cache = Arc::new(cache);
        state.error = None;
        Ok(())
    }
    async fn download(&self, path: &std::path::Path) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "价格连接初始化失败")?;
        let etag = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .cache
            .etag
            .clone();
        let mut request = client.get(SOURCE);
        if let Some(tag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, tag);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "价格来源暂时不可达，保留上次目录")?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return self.accept(None, None, path);
        }
        if !response.status().is_success() {
            return Err(format!(
                "价格来源返回 HTTP {}，保留上次目录",
                response.status().as_u16()
            ));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|h| h.to_str().ok())
            .filter(|s| s.len() <= 512)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "价格下载中断，保留上次目录")?
        {
            if bytes.len() + chunk.len() > LIMIT {
                return Err("价格来源过大，保留上次目录".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        self.accept(
            Some(std::str::from_utf8(&bytes).map_err(|_| "价格来源编码无效")?),
            etag,
            path,
        )
    }
    pub async fn refresh(&self, path: &std::path::Path) -> Result<Value, String> {
        let _guard = self.refresh.try_lock().map_err(|_| "价格正在刷新")?;
        if let Err(error) = self.download(path).await {
            self.state.write().unwrap_or_else(|e| e.into_inner()).error = Some(error.clone());
            return Err(error);
        }
        Ok(self.snapshot().1)
    }
}
pub static SERVICE: LazyLock<Service> = LazyLock::new(Service::new);
pub fn cache_path() -> PathBuf {
    crate::config::get_app_config_dir().join("default-prices.json")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    pub model: Option<String>,
    pub requested: Option<String>,
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub read: Option<u64>,
    pub write: Option<u64>,
    #[serde(default)]
    pub provider: Option<(String, String)>,
}
#[derive(Default)]
pub struct Estimate {
    totals: BTreeMap<String, Decimal>,
    parts: BTreeMap<String, [Decimal; 4]>,
    covered: u64,
    estimated: u64,
    model_assumptions: u64,
    input_assumptions: u64,
    excluded: u64,
    opted_out: u64,
    missing: BTreeMap<(String, String, String, String), u64>,
}
impl Estimate {
    pub fn exclude_provider(&mut self, meter: &Meter) {
        self.opted_out += 1;
        self.missing(meter, "已关闭费用估算");
    }
    pub fn add(&mut self, catalog: &Catalog, meter: &Meter) {
        // Historical request-model equivalent only when no final identity contradicts it.
        let model = meter
            .model
            .as_deref()
            .or(meter.requested.as_deref())
            .unwrap_or("");
        // Native Copilot dispatch sends the bare ID and the logger adds this reserved
        // namespace. Its stable provider ID survives deletion and legacy projections.
        let model = if meter
            .provider
            .as_ref()
            .is_some_and(|(id, _)| id == super::copilot::PROVIDER_ID)
        {
            model.strip_prefix("copilot/").unwrap_or(model)
        } else {
            model
        };
        let Some(price) = catalog.models.get(model) else {
            self.excluded += 1;
            self.missing(
                meter,
                if model.is_empty() {
                    "未记录最终模型"
                } else {
                    "未定价"
                },
            );
            return;
        };
        let (Some(input), Some(output)) = (meter.input, meter.output) else {
            self.excluded += 1;
            self.missing(meter, "缺少完整用量");
            return;
        };
        if meter.read.is_some_and(|n| n > input)
            || meter.write.is_some_and(|n| n > input)
            || meter
                .read
                .zip(meter.write)
                .is_some_and(|(a, b)| a.checked_add(b).is_none_or(|n| n > input))
        {
            self.excluded += 1;
            self.missing(meter, "缓存用量不一致");
            return;
        }
        let r = if input > price.threshold {
            price.long.as_ref().unwrap_or(&price.short)
        } else {
            &price.short
        };
        // Unknown splits are valued at ordinary input price, not treated as measured zero.
        // Only known, priced cache portions leave the ordinary-input bucket.
        let read = meter.read.filter(|_| r.read.is_some()).unwrap_or(0);
        let write = meter.write.filter(|_| r.write.is_some()).unwrap_or(0);
        let fresh = input - read - write;
        let input_assumed = meter.read.is_none()
            || (meter.write.is_none() && r.write.is_some())
            || (meter.read.is_some_and(|n| n > 0) && r.read.is_none())
            || (meter.write.is_some_and(|n| n > 0) && r.write.is_none());
        let model_assumed = meter.model.is_none();
        let charge = |tokens: u64, rate: Decimal| {
            Decimal::from(tokens)
                .checked_mul(rate)?
                .checked_div(Decimal::from(1_000_000))
        };
        let amounts = [
            charge(fresh, r.input),
            charge(output, r.output),
            charge(read, r.read.unwrap_or_default()),
            charge(write, r.write.unwrap_or_default()),
        ];
        if amounts.iter().any(Option::is_none) {
            self.excluded += 1;
            self.missing(meter, "金额超出精确范围");
            return;
        }
        let part = self.parts.entry(price.currency.clone()).or_default();
        let mut next = *part;
        for (i, amount) in amounts.into_iter().enumerate() {
            if let Some(amount) = amount {
                let Some(value) = next[i].checked_add(amount) else {
                    self.excluded += 1;
                    self.missing(meter, "金额超出精确范围");
                    return;
                };
                next[i] = value;
            }
        }
        let Some(total) = next
            .into_iter()
            .try_fold(Decimal::ZERO, |a, b| a.checked_add(b))
        else {
            self.excluded += 1;
            self.missing(meter, "金额超出精确范围");
            return;
        };
        *part = next;
        self.totals.insert(price.currency.clone(), total);
        self.covered += 1;
        self.estimated += u64::from(input_assumed || model_assumed);
        self.model_assumptions += u64::from(model_assumed);
        self.input_assumptions += u64::from(input_assumed);
    }
    fn missing(&mut self, meter: &Meter, reason: &str) {
        let model = meter
            .model
            .as_ref()
            .or(meter.requested.as_ref())
            .map(String::as_str)
            .unwrap_or("未识别模型");
        let (provider_id, provider_name) = meter
            .provider
            .as_ref()
            .map(|(id, name)| (id.as_str(), name.as_str()))
            .unwrap_or(("", "未知来源"));
        let key = if self.missing.len() < 10_000 {
            (
                provider_id.into(),
                provider_name.into(),
                model.into(),
                reason.into(),
            )
        } else {
            (
                String::new(),
                "其他来源".into(),
                "其他模型".into(),
                "缺口分组超过上限".into(),
            )
        };
        *self.missing.entry(key).or_default() += 1;
    }
    pub fn value(&self, status: Value) -> Value {
        let totals: BTreeMap<_, _> = self
            .totals
            .iter()
            .map(|(c, n)| (c, n.normalize().to_string()))
            .collect();
        let parts: BTreeMap<_, _> = self
            .parts
            .iter()
            .map(|(c, n)| (c, n.map(|v| v.normalize().to_string())))
            .collect();
        let missing: Vec<_> = self.missing.iter().map(|((provider_id,provider,model,reason),requests)|json!({"providerId":provider_id,"provider":provider,"model":model,"reason":reason,"requests":requests})).collect();
        json!({"totals":totals,"parts":parts,"covered":self.covered,"estimated":self.estimated,"assumptions":{"model":self.model_assumptions,"input":self.input_assumptions},"excluded":self.excluded,"optedOut":self.opted_out,"missing":missing,"catalog":status})
    }
}
#[cfg(test)]
mod tests;
