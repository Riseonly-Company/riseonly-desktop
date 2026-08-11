use std::path::Path;

use serde::Deserialize;

use super::app_environment::AppEnvironment;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Endpoints {
    pub environment: AppEnvironment,
    pub api_base_url: String,
    pub ws_url: String,
    pub public_site_url: String,
    pub source: EndpointSource,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointSource {
    Compiled,
    LocalFile,
    ProcessEnvironment,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocalOverride {
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub ws_url: Option<String>,
    #[serde(default)]
    pub public_site_url: Option<String>,
}

impl Endpoints {
    pub fn compiled_for(environment: AppEnvironment) -> Self {
        let (api, ws, site) = match environment {
            AppEnvironment::Dev => (
                "http://127.0.0.1:8080/api",
                "ws://127.0.0.1:8085",
                "http://127.0.0.1:3000",
            ),
            AppEnvironment::Staging => (
                "https://staging.riseonly.net/api",
                "wss://staging.riseonly.net",
                "https://staging.riseonly.net",
            ),
            AppEnvironment::Prod => (
                "https://riseonly.net/api",
                "wss://riseonly.net",
                "https://riseonly.net",
            ),
        };

        Self {
            environment,
            api_base_url: api.to_owned(),
            ws_url: ws.to_owned(),
            public_site_url: site.to_owned(),
            source: EndpointSource::Compiled,
        }
    }

    pub fn resolve(
        environment: AppEnvironment,
        local_file: Option<&Path>,
        read_env: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let mut endpoints = Self::compiled_for(environment);

        if !environment.allows_local_override() {
            return endpoints;
        }

        if let Some(path) = local_file
            && let Some(local) = read_local_override(path)
        {
            {
                let mut touched = false;
                if let Some(value) = normalized(local.api_base_url) {
                    endpoints.api_base_url = value;
                    touched = true;
                }
                if let Some(value) = normalized(local.ws_url) {
                    endpoints.ws_url = value;
                    touched = true;
                }
                if let Some(value) = normalized(local.public_site_url) {
                    endpoints.public_site_url = value;
                    touched = true;
                }
                if touched {
                    endpoints.source = EndpointSource::LocalFile;
                }
            }
        }

        let mut from_env = false;
        if let Some(value) = normalized(read_env("RISE_API_URL")) {
            endpoints.api_base_url = value;
            from_env = true;
        }
        if let Some(value) = normalized(read_env("RISE_WS_URL")) {
            endpoints.ws_url = value;
            from_env = true;
        }
        if let Some(value) = normalized(read_env("RISE_SITE_URL")) {
            endpoints.public_site_url = value;
            from_env = true;
        }
        if from_env {
            endpoints.source = EndpointSource::ProcessEnvironment;
        }

        endpoints
    }

    pub fn from_process(environment: AppEnvironment, local_file: Option<&Path>) -> Self {
        Self::resolve(environment, local_file, &|key| std::env::var(key).ok())
    }

    pub fn web_host(&self) -> &str {
        self.public_site_url
            .split_once("://")
            .map_or(self.public_site_url.as_str(), |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or_default()
    }

    pub fn describe(&self) -> String {
        format!(
            "env={} api={} ws={} source={:?}",
            self.environment, self.api_base_url, self.ws_url, self.source
        )
    }
}

fn normalized(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_end_matches('/').to_owned())
}

fn read_local_override(path: &Path) -> Option<LocalOverride> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_web_host_keeps_the_port_and_drops_the_scheme_and_path() {
        assert_eq!(
            Endpoints::compiled_for(AppEnvironment::Prod).web_host(),
            "riseonly.net"
        );
        assert_eq!(
            Endpoints::compiled_for(AppEnvironment::Staging).web_host(),
            "staging.riseonly.net"
        );
        assert_eq!(
            Endpoints::compiled_for(AppEnvironment::Dev).web_host(),
            "127.0.0.1:3000",
            "a dev link that dropped the port would match any service on loopback"
        );
    }

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    fn no_env() -> impl Fn(&str) -> Option<String> {
        |_: &str| None
    }

    #[test]
    fn dev_points_at_the_local_stack_on_the_reference_ports() {
        let endpoints = Endpoints::compiled_for(AppEnvironment::Dev);
        assert_eq!(endpoints.api_base_url, "http://127.0.0.1:8080/api");
        assert_eq!(endpoints.ws_url, "ws://127.0.0.1:8085");
    }

    #[test]
    fn staging_and_prod_are_distinct_and_encrypted() {
        let staging = Endpoints::compiled_for(AppEnvironment::Staging);
        let prod = Endpoints::compiled_for(AppEnvironment::Prod);

        assert_ne!(staging.api_base_url, prod.api_base_url);
        assert!(staging.api_base_url.starts_with("https://"));
        assert!(prod.api_base_url.starts_with("https://"));
        assert!(staging.ws_url.starts_with("wss://"));
        assert!(prod.ws_url.starts_with("wss://"));
    }

    #[test]
    fn the_process_environment_beats_the_compiled_default() {
        let endpoints = Endpoints::resolve(
            AppEnvironment::Dev,
            None,
            &env_from(&[("RISE_API_URL", "http://192.168.1.5:8080/api")]),
        );
        assert_eq!(endpoints.api_base_url, "http://192.168.1.5:8080/api");
        assert_eq!(endpoints.source, EndpointSource::ProcessEnvironment);
        assert_eq!(
            endpoints.ws_url, "ws://127.0.0.1:8085",
            "an unset variable must not clear the default"
        );
    }

    #[test]
    fn a_local_file_beats_the_compiled_default_and_loses_to_the_environment() {
        let dir = std::env::temp_dir().join("rise-endpoints-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("local.json");
        std::fs::write(
            &path,
            r#"{"api_base_url":"http://10.0.0.2:8080/api","ws_url":"ws://10.0.0.2:8085"}"#,
        )
        .unwrap();

        let from_file = Endpoints::resolve(AppEnvironment::Dev, Some(&path), &no_env());
        assert_eq!(from_file.api_base_url, "http://10.0.0.2:8080/api");
        assert_eq!(from_file.source, EndpointSource::LocalFile);

        let overridden = Endpoints::resolve(
            AppEnvironment::Dev,
            Some(&path),
            &env_from(&[("RISE_API_URL", "http://172.16.0.9:8080/api")]),
        );
        assert_eq!(overridden.api_base_url, "http://172.16.0.9:8080/api");
        assert_eq!(
            overridden.ws_url, "ws://10.0.0.2:8085",
            "the file still supplies what the environment did not"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn production_ignores_both_override_channels() {
        let dir = std::env::temp_dir().join("rise-endpoints-prod-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("local.json");
        std::fs::write(&path, r#"{"api_base_url":"http://evil.example/api"}"#).unwrap();

        let endpoints = Endpoints::resolve(
            AppEnvironment::Prod,
            Some(&path),
            &env_from(&[("RISE_API_URL", "http://evil.example/api")]),
        );

        assert_eq!(endpoints.api_base_url, "https://riseonly.net/api");
        assert_eq!(endpoints.source, EndpointSource::Compiled);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_missing_or_malformed_local_file_is_ignored_rather_than_fatal() {
        let missing = Path::new("/nonexistent/local.json");
        let endpoints = Endpoints::resolve(AppEnvironment::Dev, Some(missing), &no_env());
        assert_eq!(endpoints.source, EndpointSource::Compiled);

        let dir = std::env::temp_dir().join("rise-endpoints-bad-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("local.json");
        std::fs::write(&path, "not json at all").unwrap();

        let endpoints = Endpoints::resolve(AppEnvironment::Dev, Some(&path), &no_env());
        assert_eq!(endpoints.source, EndpointSource::Compiled);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn blank_and_trailing_slash_values_are_cleaned_up() {
        let endpoints = Endpoints::resolve(
            AppEnvironment::Dev,
            None,
            &env_from(&[
                ("RISE_API_URL", "  http://10.0.0.3:8080/api/  "),
                ("RISE_WS_URL", "   "),
            ]),
        );
        assert_eq!(endpoints.api_base_url, "http://10.0.0.3:8080/api");
        assert_eq!(
            endpoints.ws_url, "ws://127.0.0.1:8085",
            "a blank variable must not blank the endpoint"
        );
    }

    #[test]
    fn the_description_names_the_target_without_leaking_a_secret() {
        let description = Endpoints::compiled_for(AppEnvironment::Dev).describe();
        assert!(description.contains("env=dev"));
        assert!(description.contains("127.0.0.1:8085"));
    }
}
