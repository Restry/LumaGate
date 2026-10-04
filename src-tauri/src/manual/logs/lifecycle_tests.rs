use super::*;
use axum::{middleware, routing::post, Router};
use futures::StreamExt;
use http_body_util::BodyExt;
use serde_json::json;
use tower::Service;

async fn captured(
    body: String,
    ending: &'static str,
) -> (tempfile::TempDir, Arc<RequestLogs>, Body) {
    let temp = tempfile::tempdir().unwrap();
    let logs = Arc::new(RequestLogs::open(temp.path()).unwrap());
    let mut app = Router::new()
        .route(
            "/v1/responses",
            post(move || {
                let body = body.clone();
                async move {
                    let tail: Pin<
                        Box<
                            dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send,
                        >,
                    > = match ending {
                        "error" => Box::pin(futures::stream::once(async {
                            Err(std::io::Error::other("fixture connection reset"))
                        })),
                        "eof" => Box::pin(futures::stream::empty()),
                        _ => Box::pin(futures::stream::pending()),
                    };
                    let stream = futures::stream::once(async move { Ok(bytes::Bytes::from(body)) })
                        .chain(tail);
                    (
                        [("content-type", "text/event-stream")],
                        Body::from_stream(stream),
                    )
                }
            }),
        )
        .layer(middleware::from_fn_with_state(logs.clone(), capture));
    let response = app
        .call(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    (temp, logs, response.into_body())
}

#[tokio::test]
async fn oversized_terminal_then_body_drop_is_protocol_complete_in_history() {
    let event = format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":{"status":"completed","output":[{"text":"x".repeat(70_000)}]}})
    );
    let (_temp, logs, mut body) = captured(event.clone(), "drop").await;
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        event
    );
    drop(body);
    let result = logs.query_history(&history::Query::default()).unwrap();
    assert_eq!(
        result["stats"],
        json!({"total":1,"success":1,"failed":0,"pending":0})
    );
    assert_eq!(result["rows"][0]["result"]["state"], "success");
    assert_eq!(
        logs.query_history(&history::Query {
            status: "failed".into(),
            ..Default::default()
        })
        .unwrap()["matched"],
        0
    );
}

#[tokio::test]
async fn multiline_terminal_then_drop_keeps_connection_fact_without_failure() {
    let event = "event: response.completed\ndata: {\ndata: \"response\": {\"status\": \"completed\"}\ndata: }\n\n";
    let (_temp, logs, mut body) = captured(event.into(), "drop").await;
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        event
    );
    drop(body);
    let result = logs.query_history(&history::Query::default()).unwrap();
    assert_eq!(result["rows"][0]["result"]["state"], "success");
    assert_eq!(result["rows"][0]["delivery"]["transport"], "dropped");
    assert_eq!(result["rows"][0]["delivery"]["completion"], "completed");
}

#[tokio::test]
async fn unfinished_cancellation_and_early_eof_do_not_become_success() {
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    for (ending, expected) in [("drop", "failed"), ("eof", "pending")] {
        let (_temp, logs, mut body) = captured(partial.into(), ending).await;
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            partial
        );
        if ending == "eof" {
            assert!(body.frame().await.is_none());
        }
        drop(body);
        let result = logs.query_history(&history::Query::default()).unwrap();
        assert_eq!(result["rows"][0]["result"]["state"], expected);
        assert_eq!(result["stats"]["success"], 0);
    }
}

#[tokio::test]
async fn explicit_failure_incomplete_and_late_transport_error_outrank_completion() {
    let complete =
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n";
    for (suffix, ending) in [
        ("data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"upstream_error\"}}}\n\n", "drop"),
        ("data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\"}}\n\n", "drop"),
        ("", "error"),
    ] {
        let event = format!("{complete}{suffix}");
        let (_temp, logs, mut body) = captured(event.clone(), ending).await;
        assert_eq!(body.frame().await.unwrap().unwrap().into_data().unwrap(), event);
        if ending == "error" { assert!(body.frame().await.unwrap().is_err()); }
        drop(body);
        let result = logs.query_history(&history::Query::default()).unwrap();
        assert_eq!(result["stats"], json!({"total":1,"success":0,"failed":1,"pending":0}));
        assert_eq!(result["rows"][0]["result"]["state"], "failed");
    }
}

#[tokio::test]
async fn eof_then_drop_does_not_settle_twice_or_replace_transport_fact() {
    let event =
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n";
    let (_temp, logs, mut body) = captured(event.into(), "eof").await;
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        event
    );
    assert!(body.frame().await.is_none());
    let settled = logs.persisted_snapshot().unwrap();
    assert_eq!(settled[0]["delivery"]["transport"], "eof");
    drop(body);
    assert_eq!(logs.persisted_snapshot().unwrap(), settled);
    assert_eq!(
        logs.query_history(&history::Query::default()).unwrap()["stats"],
        json!({"total":1,"success":1,"failed":0,"pending":0})
    );
}
