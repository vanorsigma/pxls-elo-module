use anyhow::anyhow;
use profileparser::{ProfileParser, ProfileParserImpl};
pub use types::{UserRank, UserProfile};
use types::PxlsResponse;

mod profileparser;
mod types;

const DEFAULT_STATS_URL: &str = "https://pxls.space/stats/stats.json";
const DEFAULT_PROFILE_URL: &str = "https://pxls.space/profile/{user_id}";

pub trait PxlsClient {
    fn get_canvas_user_ranks(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<UserRank>, anyhow::Error>> + Send;
    fn get_profile_for_user(
        &self,
        user_id: &str,
    ) -> impl std::future::Future<Output = Result<UserProfile, anyhow::Error>> + std::marker::Send;
}

struct PxlsEndpoints {
    stats: String,
    profile: String,
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
        Ok(ProfileParserImpl::get_profile_from_encoded_string(
            self.perform_request(&self.endpoints.profile.replace("{user_id}", user_id))
                .await
                .map(|result| result.text())?
                .await?
                .as_str(),
        )
        .await?)
    }
}

#[cfg(test)]
pub struct MockPxlsClient {
    pub user_ranks_ret_val: Result<Vec<UserRank>, anyhow::Error>,
    pub user_profiles_ret_val: Result<UserProfile, anyhow::Error>,
}

#[cfg(test)]
impl Default for MockPxlsClient {
    fn default() -> Self {
        Self {
            user_ranks_ret_val: Err(anyhow!("default value")),
            user_profiles_ret_val: Err(anyhow!("default value")),
        }
    }
}

#[cfg(test)]
impl PxlsClient for MockPxlsClient {
    async fn get_canvas_user_ranks(&self) -> Result<Vec<UserRank>, anyhow::Error> {
        self.user_ranks_ret_val
            .as_ref()
            .map_err(|e| anyhow!("mock error {}", e))
            .cloned()
    }

    async fn get_profile_for_user(&self, user_id: &str) -> Result<UserProfile, anyhow::Error> {
        self.user_profiles_ret_val
            .as_ref()
            .map_err(|e| anyhow!("mock error {}", e))
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
}
