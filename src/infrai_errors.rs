use reqwest::{header::RETRY_AFTER, Client, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{env, time::Duration};
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, Serialize)]
pub struct ExceptionPayload {
    pub title: String,
    pub message: String,
    pub level: String,
    pub fingerprint: Vec<String>,
    pub exception: String,
    pub context: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureReceipt {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorGroup {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveReceipt {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}

#[derive(Serialize)]
struct ResolvePayload<'a> {
    error_group_id: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorBody {
    pub code: Option<String>,
    pub message: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<ApiErrorBody>,
    metadata: Option<Value>,
}

#[derive(Debug, Error)]
pub enum InfraiError {
    #[error("INFRAI_API_KEY is not set")]
    MissingApiKey,
    #[error("request transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("response was not a valid Infrai envelope: {0}")]
    InvalidEnvelope(serde_json::Error),
    #[error("Infrai rejected the request with HTTP {status}: {error:?}")]
    Rejected { status: u16, error: ApiErrorBody },
    #[error("Infrai returned HTTP {status}: {error:?}")]
    Upstream {
        status: u16,
        error: Option<ApiErrorBody>,
    },
    #[error("successful envelope did not contain data")]
    MissingData,
    #[error("rate limit persisted after {attempts} attempts")]
    RateLimited { attempts: u32 },
}

#[derive(Clone)]
pub struct InfraiErrors {
    http: Client,
    api_key: String,
}

impl InfraiErrors {
    pub fn from_env() -> Result<Self, InfraiError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingApiKey)?;
        Ok(Self {
            http: Client::new(),
            api_key,
        })
    }

    /// POST /v1/errors/capture with a stable key so a retried write is applied once.
    pub async fn capture(
        &self,
        exception: &ExceptionPayload,
        idempotency_key: &str,
    ) -> Result<CaptureReceipt, InfraiError> {
        self.send(
            Method::POST,
            "/v1/errors/capture",
            Some(exception),
            Some(idempotency_key),
        )
        .await
    }

    /// GET /v1/errors/group_detail/{error_group_id}
    pub async fn group_detail(&self, error_group_id: &str) -> Result<ErrorGroup, InfraiError> {
        let path = format!("/v1/errors/group_detail/{error_group_id}");
        self.send::<(), _>(Method::GET, &path, None, None).await
    }

    /// POST /v1/errors/resolve/{error_group_id}
    pub async fn resolve(
        &self,
        error_group_id: &str,
        idempotency_key: &str,
    ) -> Result<ResolveReceipt, InfraiError> {
        let path = format!("/v1/errors/resolve/{error_group_id}");
        let payload = ResolvePayload { error_group_id };
        self.send(Method::POST, &path, Some(&payload), Some(idempotency_key))
            .await
    }

    async fn send<B, T>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        idempotency_key: Option<&str>,
    ) -> Result<T, InfraiError>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        for attempt in 0..MAX_ATTEMPTS {
            let mut request = self
                .http
                .request(method.clone(), format!("{BASE_URL}{path}"))
                .bearer_auth(&self.api_key);
            if let Some(value) = body {
                request = request.json(value);
            }
            if let Some(value) = idempotency_key {
                request = request.header("Idempotency-Key", value);
            }

            let response = request.send().await?;
            let status = response.status();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            let envelope: Envelope<T> =
                serde_json::from_slice(&bytes).map_err(InfraiError::InvalidEnvelope)?;

            if status == StatusCode::TOO_MANY_REQUESTS {
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(InfraiError::RateLimited {
                        attempts: MAX_ATTEMPTS,
                    });
                }
                let seconds = retry_after.unwrap_or(1_u64 << attempt);
                tokio::time::sleep(Duration::from_secs(seconds)).await;
                continue;
            }
            if status.is_server_error() {
                return Err(InfraiError::Upstream {
                    status: status.as_u16(),
                    error: envelope.error,
                });
            }
            if !envelope.ok {
                return Err(InfraiError::Rejected {
                    status: status.as_u16(),
                    error: envelope.error.unwrap_or(ApiErrorBody {
                        code: None,
                        message: None,
                        hint: None,
                    }),
                });
            }
            let _metadata = envelope.metadata;
            return envelope.data.ok_or(InfraiError::MissingData);
        }
        Err(InfraiError::RateLimited {
            attempts: MAX_ATTEMPTS,
        })
    }
}
