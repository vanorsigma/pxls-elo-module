use futures::StreamExt;
use tokio_tungstenite::connect_async;

use super::types::PixelUpdate;

pub trait WsHandler {
    type Receiver;

    fn resusbcribe(&self) -> Self::Receiver;

    fn try_recv(&mut self) -> impl std::future::Future<Output = Result<PixelUpdate, anyhow::Error>> + std::marker::Send;
}

pub struct WsHandlerImpl {
    handle: tokio::task::JoinHandle<()>,
    rx: tokio::sync::broadcast::Receiver<PixelUpdate>,
}

impl WsHandlerImpl {
    pub async fn connect(ws_url: &str, cf_key: &str) -> Result<Self, anyhow::Error> {
        let request = http::Request::builder()
            .uri(ws_url)
            .header(
                "host",
                ws_url
                    .parse::<http::Uri>()?
                    .host()
                    .ok_or(anyhow::anyhow!("cannot parse host from uri"))?,
            )
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .header("sec-websocket-key", "")
            .header("sec-websocket-version", 13)
            .header("X-Pxls-CFAuth", cf_key)
            .body(())
            .unwrap();
        let (ws_stream, _) = connect_async(request).await?;
        let (tx, rx) = tokio::sync::broadcast::channel(10);

        let handle = tokio::spawn(async move {
            ws_stream
                .for_each(|msg_maybe| async {
                    let _ = msg_maybe
                        .map(|msg| {
                            let _ = msg
                                .into_text()
                                .map(|text| {
                                    let _ = serde_json::from_str::<PixelUpdate>(&text)
                                        .map(|update| {
                                            let _ = tx.send(update).inspect_err(|e| {
                                                log::warn!("can't send pixel update to channel {e}")
                                            });
                                        })
                                        .inspect_err(|e| log::warn!("Not a pixel update JSON {e}"));
                                })
                                .inspect_err(|e| {
                                    log::warn!("message is not text (probably a ping) {e}")
                                });
                        })
                        .inspect_err(|e| log::error!("can't get message due to {e}"));
                })
                .await;
        });

        Ok(Self { handle, rx })
    }
}

impl WsHandler for WsHandlerImpl {
    type Receiver = tokio::sync::broadcast::Receiver<PixelUpdate>;

    fn resusbcribe(&self) -> Self::Receiver {
        self.rx.resubscribe()
    }

    async fn try_recv(&mut self) -> Result<PixelUpdate, anyhow::Error> {
        self.rx.try_recv().map_err(|e| anyhow::anyhow!(e))
    }
}

impl Drop for WsHandlerImpl {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(test)]
pub struct MockWsHandler {
    pub tx: crossbeam::channel::Sender<PixelUpdate>,
    pub rx: crossbeam::channel::Receiver<PixelUpdate>,
}

#[cfg(test)]
impl Default for MockWsHandler {
    fn default() -> Self {
        let (tx, rx) = crossbeam::channel::unbounded();
        Self { tx, rx }
    }
}

#[cfg(test)]
impl WsHandler for MockWsHandler {
    type Receiver = crossbeam::channel::Receiver<PixelUpdate>;

    fn resusbcribe(&self) -> Self::Receiver {
        self.rx.clone()
    }

    async fn try_recv(&mut self) -> Result<PixelUpdate, anyhow::Error> {
        self.rx.try_recv().map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::{WsHandler, WsHandlerImpl};

    #[ignore = "this is a live test"]
    #[tokio::test]
    async fn test_ws_handler() {
        let pxls_cf_token = std::env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");
        let ws_handler = WsHandlerImpl::connect("wss://pxls.space/ws", &pxls_cf_token)
            .await
            .expect("can get ws handler");

        let mut rx_channel = ws_handler.resusbcribe();
        let item = rx_channel.recv().await.expect("can get at least one item");

        println!("item: {:#?}", item);
    }
}
