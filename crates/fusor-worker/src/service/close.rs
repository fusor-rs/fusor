use crate::{WorkerError, endpoint::Endpoint};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

type Closing = Pin<Box<dyn Future<Output = Result<(), WorkerError>>>>;
#[must_use]
pub struct Close {
    endpoint: Option<(Rc<Endpoint>, u64)>,
    duration: Duration,
    future: Option<Closing>,
    pub(super) closing: Rc<RefCell<Option<js_sys::Promise>>>,
}
impl Close {
    pub(crate) fn new(endpoint: Rc<Endpoint>, instance: u64) -> Self {
        Self {
            closing: Rc::clone(&endpoint.closing),
            endpoint: Some((endpoint, instance)),
            duration: Duration::from_secs(5),
            future: None,
        }
    }
    pub fn timeout(mut self, duration: Duration) -> Self {
        if self.future.is_some() {
            self.future = Some(Box::pin(async {
                Err(crate::error::configuration(
                    "close deadline must be configured before polling",
                ))
            }));
        } else {
            self.duration = duration;
        }
        self
    }
}
impl Future for Close {
    type Output = Result<(), WorkerError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.future.is_none() {
            let (endpoint, instance) = self.endpoint.take().expect("close starts once");
            let duration = self.duration;
            let closing = Rc::clone(&self.closing);
            self.future = Some(Box::pin(async move {
                endpoint.close(instance, duration, &closing).await
            }));
        }
        self.future
            .as_mut()
            .expect("close was started before polling")
            .as_mut()
            .poll(cx)
    }
}
