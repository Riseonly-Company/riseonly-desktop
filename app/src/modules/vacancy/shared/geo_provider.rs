use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{GEO_CITIES_PREFIX, GEO_COUNTRIES};
use serde::{Deserialize, Serialize};

const BASE: &str = "https://countriesnow.space/api/v0.1";
const COUNTRIES_PATH: &str = "/countries/iso";
const CITIES_PATH: &str = "/countries/cities";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const HTTP_OK: i64 = 200;
const POST_METHOD: &str = "POST";
const JSON_CONTENT_TYPE: &str = "application/json";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const FLAG_NAME_SEPARATOR: &str = "  ";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GeoCountry {
    pub name: String,
    pub iso2: String,
}

impl GeoCountry {
    pub fn new(name: impl Into<String>, iso2: impl Into<String>) -> Self {
        Self { name: name.into(), iso2: iso2.into() }
    }

    pub fn id(&self) -> &str {
        if self.iso2.is_empty() { &self.name } else { &self.iso2 }
    }

    /// flag emoji of the country
    pub fn flag(&self) -> String {
        GeoProvider::flag_emoji(&self.iso2)
    }

    /// display name of the country
    pub fn display(&self) -> String {
        let flag = self.flag();
        if flag.is_empty() {
            self.name.clone()
        } else {
            format!("{flag}{FLAG_NAME_SEPARATOR}{}", self.name)
        }
    }
}

#[derive(Deserialize)]
struct GeoProviderISOResponse {
    data: Option<Vec<GeoProviderISOCountry>>,
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct GeoProviderISOCountry {
    name: String,
    Iso2: Option<String>,
}

#[derive(Deserialize)]
struct GeoProviderCitiesResponse {
    error: Option<bool>,
    data: Option<Vec<String>>,
}

#[derive(Serialize)]
struct GeoProviderCitiesRequest<'a> {
    country: &'a str,
}

pub struct GeoProvider {
    countries: Vec<GeoCountry>,
    is_loading_countries: bool,
    cities_cache: HashMap<String, Vec<String>>,
    loading_cities: HashSet<String>,
}

impl GeoProvider {
    /// the shared geo provider
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            countries: Self::load_cached_countries().unwrap_or_else(|| Self::fallback_countries().to_vec()),
            is_loading_countries: false,
            cities_cache: HashMap::new(),
            loading_cities: HashSet::new(),
        }
    }

    pub fn countries(&self) -> &[GeoCountry] {
        &self.countries
    }

    pub fn is_loading_countries(&self) -> bool {
        self.is_loading_countries
    }

    pub fn ensure_countries_loaded(&mut self, cx: &mut Context<Self>) {
        if self.is_loading_countries || self.countries.len() > Self::fallback_countries().len() + 1 {
            return;
        }
        self.is_loading_countries = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let fetched = RiseAccountRuntime::current().spawn(Self::fetch_countries()).await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                this.is_loading_countries = false;
                if let Some(fetched) = fetched
                    && fetched.len() > this.countries.len()
                {
                    Self::cache_countries(&fetched);
                    this.countries = fetched;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn cached_cities(&self, country: &str) -> &[String] {
        self.cities_cache.get(country).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn ensure_cities_loaded(&mut self, country: &str, cx: &mut Context<Self>) {
        let key = country.to_lowercase();
        if country.is_empty() || self.cities_cache.contains_key(country) || self.loading_cities.contains(&key) {
            return;
        }
        if let Some(disk) = Self::load_cached_cities(country) {
            self.cities_cache.insert(country.to_owned(), disk);
            cx.notify();
            return;
        }
        self.loading_cities.insert(key.clone());
        cx.notify();
        let country = country.to_owned();
        cx.spawn(async move |this, cx| {
            let cities = RiseAccountRuntime::current()
                .spawn(Self::fetch_cities(country.clone()))
                .await
                .ok()
                .flatten();
            let _ = this.update(cx, |this, cx| {
                this.loading_cities.remove(&key);
                if let Some(cities) = cities
                    && !cities.is_empty()
                {
                    Self::cache_cities(&country, &cities);
                    this.cities_cache.insert(country, cities);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn is_loading_cities(&self, country: &str) -> bool {
        self.loading_cities.contains(&country.to_lowercase())
    }

    pub fn country_name(&self, for_city: &str) -> Option<String> {
        if for_city.is_empty() {
            return None;
        }
        self.cities_cache
            .iter()
            .find(|(_, cities)| cities.iter().any(|city| city == for_city))
            .map(|(country, _)| country.clone())
    }

    pub fn flag_emoji(iso2: &str) -> String {
        rise_regions::flag_emoji(&iso2.to_uppercase()).unwrap_or_default()
    }

    async fn fetch_countries() -> Option<Vec<GeoCountry>> {
        let url = Url::from_string(&format!("{BASE}{COUNTRIES_PATH}"))?;
        let mut request = URLRequest::new(url);
        request.timeout_interval = REQUEST_TIMEOUT;
        let (data, response) = URLSession::shared().data_for(request).await.ok()?;
        if response.status_code != HTTP_OK {
            return None;
        }
        let decoded: GeoProviderISOResponse = serde_json::from_slice(&data).ok()?;
        let list = decoded.data?;
        let mut mapped: Vec<GeoCountry> = list
            .into_iter()
            .map(|country| GeoCountry::new(country.name, country.Iso2.unwrap_or_default().to_uppercase()))
            .filter(|country| !country.name.is_empty())
            .collect();
        mapped.sort_by(|a, b| a.name.cmp(&b.name));
        (!mapped.is_empty()).then_some(mapped)
    }

    async fn fetch_cities(country: String) -> Option<Vec<String>> {
        let url = Url::from_string(&format!("{BASE}{CITIES_PATH}"))?;
        let mut request = URLRequest::new(url);
        request.http_method = POST_METHOD.to_owned();
        request.timeout_interval = REQUEST_TIMEOUT;
        request.set_value(JSON_CONTENT_TYPE, CONTENT_TYPE_HEADER);
        request.http_body = serde_json::to_vec(&GeoProviderCitiesRequest { country: &country }).ok();
        let (data, response) = URLSession::shared().data_for(request).await.ok()?;
        if response.status_code != HTTP_OK {
            return None;
        }
        let decoded: GeoProviderCitiesResponse = serde_json::from_slice(&data).ok()?;
        if decoded.error == Some(true) {
            return None;
        }
        let cities = decoded.data?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut unique: Vec<String> =
            cities.into_iter().filter(|city| seen.insert(city.clone()) && !city.is_empty()).collect();
        unique.sort();
        Some(unique)
    }

    fn cache_countries(list: &[GeoCountry]) {
        let Ok(data) = serde_json::to_vec(list) else {
            return;
        };
        UserDefaults::standard().set(GEO_COUNTRIES, data);
    }

    fn load_cached_countries() -> Option<Vec<GeoCountry>> {
        let data = UserDefaults::standard().data(GEO_COUNTRIES)?;
        let list: Vec<GeoCountry> = serde_json::from_slice(&data).ok()?;
        (!list.is_empty()).then_some(list)
    }

    fn cache_cities(country: &str, cities: &[String]) {
        let Ok(data) = serde_json::to_vec(cities) else {
            return;
        };
        UserDefaults::standard().set(&Self::cities_key(country), data);
    }

    fn load_cached_cities(country: &str) -> Option<Vec<String>> {
        let data = UserDefaults::standard().data(&Self::cities_key(country))?;
        let list: Vec<String> = serde_json::from_slice(&data).ok()?;
        (!list.is_empty()).then_some(list)
    }

    fn cities_key(country: &str) -> String {
        format!("{GEO_CITIES_PREFIX}{}", country.to_lowercase())
    }

    /// built in countries sorted by name, used when the load fails
    pub fn fallback_countries() -> &'static [GeoCountry] {
        static FALLBACK: LazyLock<Vec<GeoCountry>> = LazyLock::new(|| {
            let mut list: Vec<GeoCountry> = [
                ("Kazakhstan", "KZ"),
                ("Russia", "RU"),
                ("Kyrgyzstan", "KG"),
                ("Uzbekistan", "UZ"),
                ("Tajikistan", "TJ"),
                ("Turkmenistan", "TM"),
                ("Azerbaijan", "AZ"),
                ("Armenia", "AM"),
                ("Georgia", "GE"),
                ("Belarus", "BY"),
                ("Ukraine", "UA"),
                ("Turkey", "TR"),
                ("United States", "US"),
                ("United Kingdom", "GB"),
                ("Germany", "DE"),
                ("France", "FR"),
                ("Spain", "ES"),
                ("Italy", "IT"),
                ("Netherlands", "NL"),
                ("Poland", "PL"),
                ("United Arab Emirates", "AE"),
                ("Saudi Arabia", "SA"),
                ("China", "CN"),
                ("Japan", "JP"),
                ("South Korea", "KR"),
                ("India", "IN"),
                ("Canada", "CA"),
                ("Brazil", "BR"),
                ("Australia", "AU"),
                ("Czechia", "CZ"),
            ]
            .into_iter()
            .map(|(name, iso2)| GeoCountry::new(name, iso2))
            .collect();
            list.sort_by(|a, b| a.name.cmp(&b.name));
            list
        });
        &FALLBACK
    }
}
