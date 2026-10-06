//! Admission and response-body lifetime share one lock. Closing admission never aborts work.
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use http_body::Body as HttpBody;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::watch;

#[derive(Default)]
struct Counts {
    closed: bool,
    active: usize,
}
pub struct Gate {
    counts: Mutex<Counts>,
    changed: watch::Sender<usize>,
}
impl Default for Gate {
    fn default() -> Self {
        Self {
            counts: Mutex::new(Counts::default()),
            changed: watch::channel(0).0,
        }
    }
}
impl Gate {
    pub fn enter(self: &Arc<Self>) -> Result<Lease, String> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if counts.closed {
            return Err("更新正在等待现有请求结束，请稍后重试".into());
        }
        counts.active += 1;
        self.changed.send_replace(counts.active);
        Ok(Lease(self.clone()))
    }
    pub fn close(self: &Arc<Self>) -> Result<Barrier, String> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if counts.closed {
            return Err("更新排空已在进行".into());
        }
        counts.closed = true;
        Ok(Barrier(self.clone()))
    }
    pub fn closed(&self) -> bool {
        self.counts.lock().unwrap_or_else(|e| e.into_inner()).closed
    }
    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.changed.subscribe()
    }
}
pub struct Lease(Arc<Gate>);
impl Drop for Lease {
    fn drop(&mut self) {
        let mut counts = self.0.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.active -= 1;
        self.0.changed.send_replace(counts.active);
    }
}
pub struct Barrier(Arc<Gate>);
impl Drop for Barrier {
    fn drop(&mut self) {
        self.0
            .counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = false;
    }
}
// Field drop order is intentional: finish/drop the logging body before releasing admission.
struct TrackedBody {
    body: Body,
    _lease: Lease,
}
impl HttpBody for TrackedBody {
    type Data = bytes::Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}
pub async fn admission(State(gate): State<Arc<Gate>>, request: Request, next: Next) -> Response {
    let lease = match gate.enter() {
        Ok(lease) => lease,
        Err(message) => {
            let mut response = super::http_error(
                http::StatusCode::SERVICE_UNAVAILABLE,
                "update_draining",
                &message,
            );
            response.headers_mut().insert(
                http::header::RETRY_AFTER,
                http::HeaderValue::from_static("5"),
            );
            return response;
        }
    };
    let (parts, body) = next.run(request).await.into_parts();
    Response::from_parts(
        parts,
        Body::new(TrackedBody {
            body,
            _lease: lease,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use futures::StreamExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[tokio::test]
    async fn drain_holds_full_stream_rejects_new_work_and_cancel_resumes() {
        let gate = Arc::new(Gate::default());
        let finish = Arc::new(tokio::sync::Notify::new());
        let stream_finish = finish.clone();
        let app = Router::new()
            .route(
                "/stream",
                get(move || {
                    let finish = stream_finish.clone();
                    async move {
                        Body::from_stream(async_stream::stream! {
                            yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"first\n"));
                            finish.notified().await;
                            yield Ok(bytes::Bytes::from_static(b"last\n"));
                        })
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                gate.clone(),
                admission,
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        let mut response = client.get(&url).send().await.unwrap().bytes_stream();
        assert_eq!(response.next().await.unwrap().unwrap(), "first\n");
        let barrier = gate.close().unwrap();
        let mut active = gate.subscribe();
        assert_eq!(*active.borrow_and_update(), 1);
        let rejected = client.get(&url).send().await.unwrap();
        assert_eq!(rejected.status(), 503);
        assert_eq!(rejected.headers()["retry-after"], "5");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), active.changed())
                .await
                .is_err()
        );
        finish.notify_one();
        assert_eq!(response.next().await.unwrap().unwrap(), "last\n");
        assert!(response.next().await.is_none());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while *active.borrow_and_update() != 0 {
                active.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(gate.enter().is_err());
        drop(barrier);
        let accepted = gate.enter().unwrap();
        drop(accepted);
        server.abort();
    }
    #[test]
    fn admission_cannot_race_past_closed_barrier() {
        let gate = Arc::new(Gate::default());
        let closed = Arc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let gate = gate.clone();
                let closed = closed.clone();
                std::thread::spawn(move || {
                    while !closed.load(Ordering::Acquire) {
                        let _ = gate.enter();
                    }
                    for _ in 0..1000 {
                        assert!(gate.enter().is_err());
                    }
                })
            })
            .collect();
        let barrier = gate.close().unwrap();
        closed.store(true, Ordering::Release);
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(*gate.subscribe().borrow(), 0);
        drop(barrier);
        assert!(gate.enter().is_ok());
    }
    #[test]
    fn unpolled_response_drop_releases_lease() {
        let gate = Arc::new(Gate::default());
        let body = TrackedBody {
            body: Body::empty(),
            _lease: gate.enter().unwrap(),
        };
        let barrier = gate.close().unwrap();
        assert_eq!(*gate.subscribe().borrow(), 1);
        drop(body);
        assert_eq!(*gate.subscribe().borrow(), 0);
        drop(barrier);
        assert!(gate.enter().is_ok());
    }
}
