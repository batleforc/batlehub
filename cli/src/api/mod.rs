pub mod admin;
pub mod auth;
pub mod owner;
pub mod package;
pub mod publish;
pub mod registry;
pub mod version;

use anyhow::{bail, Result};
use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone)]
pub struct BatleHubClient {
    inner: reqwest::Client,
    pub base_url: String,
    pub token: Option<String>,
}

impl BatleHubClient {
    pub fn new(base_url: &str, token: Option<&str>) -> Result<Self> {
        let inner = reqwest::Client::builder()
            .user_agent("batlehub-cli/0.1")
            .build()?;
        Ok(Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.map(str::to_string),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth_header(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let mut req = self.inner.request(Method::GET, self.url(path));
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_ok(resp).await
    }

    pub async fn get_with_params<T: DeserializeOwned, P: Serialize>(
        &self,
        path: &str,
        params: &P,
    ) -> Result<T> {
        let mut req = self
            .inner
            .request(Method::GET, self.url(path))
            .query(params);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_ok(resp).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let mut req = self.inner.request(Method::POST, self.url(path)).json(body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_ok(resp).await
    }

    pub async fn post_no_body(&self, path: &str) -> Result<()> {
        let mut req = self.inner.request(Method::POST, self.url(path));
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_no_content(resp).await
    }

    pub async fn post_void<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let mut req = self.inner.request(Method::POST, self.url(path)).json(body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_no_content(resp).await
    }

    pub async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let mut req = self.inner.request(Method::PUT, self.url(path)).json(body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_no_content(resp).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let mut req = self.inner.request(Method::DELETE, self.url(path));
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_no_content(resp).await
    }

    pub async fn post_multipart(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<serde_json::Value> {
        let mut req = self
            .inner
            .request(Method::PUT, self.url(path))
            .multipart(form);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        expect_ok(resp).await
    }
}

async fn expect_ok<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if status.is_success() {
        Ok(resp.json::<T>().await?)
    } else {
        let body = resp.text().await.unwrap_or_default();
        bail!("HTTP {status}: {body}")
    }
}

async fn expect_no_content(resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() || status == StatusCode::NO_CONTENT {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        bail!("HTTP {status}: {body}")
    }
}
