use profileparser::{ProfileParser, ProfileParserImpl};
use types::PxlsResponse;
pub use types::{UserProfile, UserRank, PxlsPixelResponse, PixelUpdate, Pixel};
pub use wshandler::{WsHandler, WsHandlerImpl};

mod profileparser;
mod types;
mod wshandler;

const DEFAULT_STATS_URL: &str = "https://pxls.space/stats/stats.json";
const DEFAULT_PROFILE_URL: &str = "https://pxls.space/profile/{user_id}";
const DEFAULT_WS_URL: &str = "wss://pxls.space/ws";
const DEFAULT_LOOKUP_URL: &str = "https://pxls.space/lookup?x={x}&y={y}";

pub trait PxlsClient {
    fn get_canvas_user_ranks(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<UserRank>, anyhow::Error>> + Send;

    fn get_profile_for_user(
        &self,
        user_id: &str,
    ) -> impl std::future::Future<Output = Result<UserProfile, anyhow::Error>> + std::marker::Send;

    fn get_websocket_handler(
        &self,
    ) -> impl std::future::Future<Output = Result<impl WsHandler, anyhow::Error>> + Send;

    fn get_metadata_for_pixel(
        &self,
        x: u64,
        y: u64,
    ) -> impl std::future::Future<Output = Result<PxlsPixelResponse, anyhow::Error>> + Send;
}

pub struct PxlsEndpoints {
    stats: String,
    profile: String,
    ws: String,
    lookup: String,
}

pub struct PxlsReqwestClient {
    endpoints: PxlsEndpoints,
    token: String,
    cf_key: String,
}

impl Default for PxlsEndpoints {
    fn default() -> Self {
        Self {
            stats: DEFAULT_STATS_URL.to_string(),
            profile: DEFAULT_PROFILE_URL.to_string(),
            ws: DEFAULT_WS_URL.to_string(),
            lookup: DEFAULT_LOOKUP_URL.to_string(),
        }
    }
}

impl PxlsReqwestClient {
    pub fn new_with_token(token: &str, cf_key: &str) -> Self {
        PxlsReqwestClient::new_with_url_and_token(PxlsEndpoints::default(), token, cf_key)
    }

    pub fn new_with_url_and_token(endpoints: PxlsEndpoints, token: &str, cf_key: &str) -> Self {
        Self {
            endpoints,
            token: token.to_string(),
            cf_key: cf_key.to_string(),
        }
    }

    async fn perform_request(&self, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        reqwest::Client::new()
            .get(url)
            .header("Cookie", format!("pxls-token={}", self.token))
            .header("X-Pxls-CFAuth", self.cf_key.to_string())
            .send()
            .await
    }
}

impl PxlsClient for PxlsReqwestClient {
    async fn get_canvas_user_ranks(&self) -> Result<Vec<UserRank>, anyhow::Error> {
        Ok(self
            .perform_request(&self.endpoints.stats)
            .await
            .map(|result| result.json::<PxlsResponse>())?
            .await?
            .toplist
            .canvas)
    }

    async fn get_profile_for_user(&self, user_id: &str) -> Result<UserProfile, anyhow::Error> {
        ProfileParserImpl::get_profile_from_encoded_string(
            self.perform_request(&self.endpoints.profile.replace("{user_id}", user_id))
                .await
                .map(|result| result.text())?
                .await?
                .as_str(),
        )
        .await
    }

    async fn get_websocket_handler(&self) -> Result<WsHandlerImpl, anyhow::Error> {
        WsHandlerImpl::connect(&self.endpoints.ws, &self.cf_key).await
    }

    async fn get_metadata_for_pixel(
        &self,
        x: u64,
        y: u64,
    ) -> Result<PxlsPixelResponse, anyhow::Error> {
        Ok(self
            .perform_request(
                &self
                    .endpoints
                    .lookup
                    .replace("{x}", &x.to_string())
                    .replace("{y}", &y.to_string()),
            )
            .await
            .map(|result| result.json::<PxlsPixelResponse>())?
            .await?)
    }
}

#[cfg(test)]
pub use wshandler::MockWsHandler;

#[cfg(test)]
pub struct MockPxlsClient {
    pub user_ranks_ret_val: Result<Vec<UserRank>, anyhow::Error>,
    pub user_profiles_ret_val: Result<UserProfile, anyhow::Error>,
    pub username_for_pixel_ret_val: Result<PxlsPixelResponse, anyhow::Error>,
}

#[cfg(test)]
impl Default for MockPxlsClient {
    fn default() -> Self {
        Self {
            user_ranks_ret_val: Err(anyhow::anyhow!("default value")),
            user_profiles_ret_val: Err(anyhow::anyhow!("default value")),
            username_for_pixel_ret_val: Err(anyhow::anyhow!("default value")),
        }
    }
}

#[cfg(test)]
impl PxlsClient for MockPxlsClient {
    async fn get_canvas_user_ranks(&self) -> Result<Vec<UserRank>, anyhow::Error> {
        self.user_ranks_ret_val
            .as_ref()
            .map_err(|e| anyhow::anyhow!("mock error {}", e))
            .cloned()
    }

    async fn get_profile_for_user(&self, _user_id: &str) -> Result<UserProfile, anyhow::Error> {
        self.user_profiles_ret_val
            .as_ref()
            .map_err(|e| anyhow::anyhow!("mock error {}", e))
            .cloned()
    }

    async fn get_websocket_handler(&self) -> Result<impl WsHandler, anyhow::Error> {
        Ok(MockWsHandler::default())
    }

    async fn get_metadata_for_pixel(
        &self,
        _x: u64,
        _y: u64,
    ) -> Result<PxlsPixelResponse, anyhow::Error> {
        self.username_for_pixel_ret_val
            .as_ref()
            .map_err(|e| anyhow::anyhow!("mock error {}", e))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use crate::pxlsclient::{PxlsClient, PxlsReqwestClient};

    #[ignore = "This is a live environment test, which requires live credentials"]
    #[tokio::test]
    async fn test_live_perform_stats_request() {
        let pxls_auth_token = env::var("PXLS_AUTH_TOKEN").expect("please set PXLS_AUTH_TOKEN");
        let pxls_cf_token = env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");
        let client = PxlsReqwestClient::new_with_token(&pxls_auth_token, &pxls_cf_token);
        let result = client
            .get_canvas_user_ranks()
            .await
            .expect("should be able to perform the call");

        assert!(result.len() > 0)
    }

    #[ignore = "This is a live environment test, which requires live credentials"]
    #[tokio::test]
    async fn test_live_perform_profile_request() {
        let pxls_auth_token = env::var("PXLS_AUTH_TOKEN").expect("please set PXLS_AUTH_TOKEN");
        let pxls_cf_token = env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");
        let client = PxlsReqwestClient::new_with_token(&pxls_auth_token, &pxls_cf_token);
        let result = client
            .get_profile_for_user("superbox2147")
            .await
            .expect("should be able to perform the call");

        assert_eq!(result.discord_tag, Some("superbox2147".to_string()));
        assert_eq!(result.faction_id, Some(3680));
    }

    #[ignore = "This is a live environment test, which requires live credentials"]
    #[tokio::test]
    async fn test_username_for_pixel_profile_request() {
        let pxls_auth_token = env::var("PXLS_AUTH_TOKEN").expect("please set PXLS_AUTH_TOKEN");
        let pxls_cf_token = env::var("PXLS_CF_TOKEN").expect("please set PXLS_CF_TOKEN");
        let client = PxlsReqwestClient::new_with_token(&pxls_auth_token, &pxls_cf_token);
        let result = client
            .get_metadata_for_pixel(1907, 528)
            .await
            .expect("should be able to perform the call");

        println!("result: {:#?}", result);
    }
}
