pub trait RentryClient {
    fn get_users_from_url(
        &self,
        url: &str,
    ) -> impl std::future::Future<Output = Result<Vec<String>, anyhow::Error>> + Send;
}

#[derive(Default)]
pub struct RentryClientImpl {}

impl RentryClient for RentryClientImpl {
    async fn get_users_from_url(&self, url: &str) -> Result<Vec<String>, anyhow::Error> {
        Ok(reqwest::Client::new()
            .get(url)
            .header("Accept", "text/plain")
            .send()
            .await?
            .text()
            .await?
            .split("\n")
            .map(|s| s.trim().to_string())
            .collect())
    }
}

#[cfg(test)]
pub struct MockRentryClientImpl {
    pub get_users_from_url_response: Result<Vec<String>, anyhow::Error>,
}

#[cfg(test)]
impl Default for MockRentryClientImpl {
    fn default() -> Self {
        Self {
            get_users_from_url_response: Err(anyhow::anyhow!("default error")),
        }
    }
}

#[cfg(test)]
impl RentryClient for MockRentryClientImpl {
    async fn get_users_from_url(&self, _url: &str) -> Result<Vec<String>, anyhow::Error> {
        self.get_users_from_url_response
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{:#?}", e))
            .map(|x| x.clone())
    }
}
