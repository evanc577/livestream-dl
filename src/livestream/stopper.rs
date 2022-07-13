use std::sync::Arc;

use futures::lock::Mutex;
use tokio::sync::Notify;

#[derive(Clone, Debug)]
pub struct Stopper(Arc<(Notify, Mutex<bool>)>);

/// Used to signal m3u8 fetcher task to quit
impl Stopper {
    pub fn new() -> Self {
        Self(Arc::new((Notify::new(), Mutex::new(false))))
    }

    /// Wait for stopper to be notified
    pub async fn wait(&self) {
        self.0 .0.notified().await;
    }

    /// Check if stopped
    pub async fn stopped(&self) -> bool {
        *self.0 .1.lock().await
    }

    /// Set to stopped and notify waiters
    pub async fn stop(&self) {
        *self.0 .1.lock().await = true;
        self.0 .0.notify_waiters();
    }
}
