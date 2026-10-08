use super::*;
fn meter() -> Meter {
    Meter {
        model: Some("gpt-6-astra".into()),
        requested: Some("alias".into()),
        input: Some(1000),
        output: Some(200),
        read: Some(800),
        write: Some(0),
        provider: None,
    }
}
#[test]
fn exact_rates_split_cache_and_long_context_without_rounding() {
    let catalog = Catalog::bundled();
    let mut total = Estimate::default();
    total.add(&catalog, &meter());
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.0128");
    let mut request = meter();
    request.input = Some(272001);
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "5.45342");
}
#[test]
fn unknown_zero_estimated_and_currencies_remain_distinct() {
    let mut catalog = Catalog::bundled();
    let mut zero = catalog.models["gpt-6-astra"].clone();
    zero.short = Rates {
        input: Decimal::ZERO,
        output: Decimal::ZERO,
        read: Some(Decimal::ZERO),
        write: Some(Decimal::ZERO),
    };
    catalog.models.insert("free-fixture".into(), zero);
    let mut total = Estimate::default();
    let mut request = meter();
    request.model = Some("unknown".into());
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"], json!({}));
    request.model = Some("free-fixture".into());
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0");
    request = meter();
    request.write = None;
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["estimated"], 1);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.0128");
    let mut cny = catalog.models["gpt-6-astra"].clone();
    cny.currency = "CNY".into();
    catalog.models.insert("cny-fixture".into(), cny);
    request = meter();
    request.model = Some("cny-fixture".into());
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["CNY"], "0.0128");
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.0128");
}
#[test]
fn normalized_anthropic_input_deducts_reads_and_writes_once() {
    let mut catalog = Catalog::bundled();
    // Synthetic arithmetic fixture, not a published Anthropic price claim.
    let d = |s| Decimal::from_str(s).unwrap();
    catalog.models.insert(
        "anthropic-fixture".into(),
        Price {
            currency: "USD".into(),
            short: Rates {
                input: d("3"),
                output: d("15"),
                read: Some(d("0.3")),
                write: Some(d("3.75")),
            },
            long: None,
            threshold: 272000,
        },
    );
    let mut request = meter();
    request.model = Some("anthropic-fixture".into());
    request.input = Some(1500);
    request.read = Some(1000);
    request.write = Some(400);
    let mut total = Estimate::default();
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.0051");
    catalog
        .models
        .get_mut("anthropic-fixture")
        .unwrap()
        .short
        .write = None;
    let mut total = Estimate::default();
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["estimated"], 1);
    assert_eq!(total.value(Value::Null)["parts"]["USD"][0], "0.0015");
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.0048");
}
fn page() -> String {
    let mut page=String::from("Prices per 1M tokens.\nShort context: ≤272K input tokens. Long context: >272K input tokens.\n### Standard pricing data\n");
    for i in 0..10 {
        page.push_str(&format!(
            "| fixture-{i} | $1 | $0.1 | - | $2 | - | - | - | - |\n"
        ));
    }
    page.push_str("### Batch pricing data\n");
    page
}
#[test]
fn malformed_refresh_and_disk_failures_keep_last_good_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("prices.json");
    let service = Service::new();
    service
        .accept(Some(&page()), Some("fixture-etag".into()), &path)
        .unwrap();
    assert!(service
        .accept(Some(&page().replace("$1 |", "$-1 |")), None, &path)
        .is_err());
    assert!(service
        .accept(Some(&page().replace("272K", "273K")), None, &path)
        .is_err());
    assert!(service.accept(Some(&page()), None, temp.path()).is_err());
    let reloaded = Service::new();
    reloaded.load(&path);
    assert!(reloaded
        .snapshot()
        .0
        .catalog
        .models
        .contains_key("fixture-0"));
    assert_eq!(
        reloaded.snapshot().0.catalog.models["fixture-0"]
            .short
            .input,
        Decimal::ONE
    );
    service.accept(None, None, &path).unwrap();
    assert!(service
        .snapshot()
        .0
        .catalog
        .models
        .contains_key("fixture-0"));
}
#[test]
fn bundled_catalog_obeys_validation_contract() {
    Catalog::bundled().validate().unwrap();
}

#[test]
fn unresolved_cache_and_legacy_model_are_estimates_not_missing_spend() {
    let catalog = Catalog::bundled();
    let mut request = meter();
    request.model = None;
    request.requested = Some("gpt-6-astra".into());
    request.read = None;
    request.write = None;
    let mut total = Estimate::default();
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.02");
    assert_eq!(
        total.value(Value::Null)["assumptions"],
        json!({"model":1,"input":1})
    );
    request.model = Some("FW-gpt-6-astra".into());
    total.add(&catalog, &request);
    assert_eq!(total.value(Value::Null)["totals"]["USD"], "0.02");
    assert_eq!(total.value(Value::Null)["excluded"], 1);
}
