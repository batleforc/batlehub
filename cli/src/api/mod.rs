pub mod admin;
pub mod auth;
pub mod owner;
pub mod package;
pub mod publish;
pub mod registry;
pub mod setup;
pub mod suggest;
pub mod version;

use anyhow::{bail, Result};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone)]
pub struct BatleHubClient {
    inner: reqwest::Client,
    pub base_url: String,
    pub token: Option<String>,
}

impl BatleHubClient {
    pub fn new(base_url: &str, token: Option<&str>) -> Result<Self> {
        // No total `.timeout()`: this client also streams artifact downloads,
        // which may legitimately be large. Bound connection setup and per-read
        // inactivity instead so an unresponsive server can't hang the CLI forever.
        let inner = reqwest::Client::builder()
            .user_agent("batlehub-cli/0.1")
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(60))
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

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.inner.request(method, self.url(path))
    }

    /// Attach the auth header (if any) and send. Every HTTP-verb method funnels
    /// through here so the auth-attach step exists in exactly one place.
    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        let mut req = req;
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        Ok(req.send().await?)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.send(self.request(Method::GET, path)).await?;
        expect_ok(resp).await
    }

    pub async fn get_with_params<T: DeserializeOwned, P: Serialize>(
        &self,
        path: &str,
        params: &P,
    ) -> Result<T> {
        let req = self.request(Method::GET, path).query(params);
        let resp = self.send(req).await?;
        expect_ok(resp).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let req = self.request(Method::POST, path).json(body);
        let resp = self.send(req).await?;
        expect_ok(resp).await
    }

    pub async fn post_no_body(&self, path: &str) -> Result<()> {
        let resp = self.send(self.request(Method::POST, path)).await?;
        expect_no_content(resp).await
    }

    pub async fn post_void<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::POST, path).json(body);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }

    pub async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::PUT, path).json(body);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }

    /// PUT a raw binary body (e.g. a package upload) and expect a 2xx response.
    pub async fn put_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let req = self
            .request(Method::PUT, path)
            .header("Content-Type", "application/octet-stream")
            .body(body);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }

    /// POST a raw binary body (e.g. a package upload) and expect a 2xx response.
    pub async fn post_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let req = self
            .request(Method::POST, path)
            .header("Content-Type", "application/octet-stream")
            .body(body);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self.send(self.request(Method::DELETE, path)).await?;
        expect_no_content(resp).await
    }

    pub async fn delete_with_params_json<T: DeserializeOwned, P: Serialize>(
        &self,
        path: &str,
        params: &P,
    ) -> Result<T> {
        let req = self.request(Method::DELETE, path).query(params);
        let resp = self.send(req).await?;
        expect_ok(resp).await
    }

    /// GET a proxy path (relative to the server base URL) or an absolute URL and
    /// stream the response body into `dest`, returning the number of bytes written.
    ///
    /// The auth token is attached **only** when the target resolves to the
    /// configured server origin, so RBAC-protected registries on that server stay
    /// reachable. An arbitrary absolute URL (e.g. a redirect or a manifest-sourced
    /// download link to another host) is fetched **without** the token — sending
    /// the BatleHub credential to an unrelated host would be a leak.
    pub async fn download_to<W: std::io::Write>(
        &self,
        path_or_url: &str,
        dest: &mut W,
    ) -> Result<u64> {
        use futures::StreamExt;

        let (url, is_own_origin) =
            if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
                let own = path_or_url == self.base_url
                    || path_or_url.starts_with(&format!("{}/", self.base_url));
                (path_or_url.to_string(), own)
            } else if let Some(rest) = path_or_url.strip_prefix('/') {
                (self.url(&format!("/{rest}")), true)
            } else {
                (self.url(&format!("/{path_or_url}")), true)
            };

        let mut req = self.inner.request(Method::GET, url.as_str());
        if is_own_origin {
            if let Some(auth) = self.auth_header() {
                req = req.header("Authorization", auth);
            }
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("HTTP {status}: {body}");
        }

        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            dest.write_all(&chunk)?;
            total += chunk.len() as u64;
        }
        Ok(total)
    }

    pub async fn put_multipart_void(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<()> {
        let req = self.request(Method::PUT, path).multipart(form);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }

    pub async fn post_multipart_void(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<()> {
        let req = self.request(Method::POST, path).multipart(form);
        let resp = self.send(req).await?;
        expect_no_content(resp).await
    }
}

pub(crate) async fn expect_ok<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
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
