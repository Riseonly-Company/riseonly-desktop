use futures::future::BoxFuture;
use rise_engine::{HttpCall, HttpReply, HttpSender, HttpVerb, WireError};

pub struct ReqwestHttpSender {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestHttpSender {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    fn url_for(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

impl HttpSender for ReqwestHttpSender {
    fn send(&self, call: HttpCall) -> BoxFuture<'static, Result<HttpReply, WireError>> {
        let url = self.url_for(call.path);
        let client = self.client.clone();

        Box::pin(async move {
            let mut request = match call.verb {
                HttpVerb::Get => client.get(url),
                HttpVerb::Post => client.post(url).body(call.body),
            }
            .header("Content-Type", "application/json")
            .timeout(call.timeout);

            if let Some(authorization) = call.authorization {
                request = request.header("Authorization", authorization);
            }

            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    WireError::Timeout(call.timeout)
                } else {
                    WireError::NotConnected
                }
            })?;

            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .map_err(|error| WireError::Decode(error.to_string()))?;

            Ok(HttpReply { status, body })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_and_the_path_join_with_exactly_one_slash() {
        let sender = ReqwestHttpSender::new("http://127.0.0.1:8080/api/");
        assert_eq!(
            sender.url_for("/auth/refresh"),
            "http://127.0.0.1:8080/api/auth/refresh"
        );
        assert_eq!(
            sender.url_for("auth/refresh"),
            "http://127.0.0.1:8080/api/auth/refresh"
        );
    }
}
