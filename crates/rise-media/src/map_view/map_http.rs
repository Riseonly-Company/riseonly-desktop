use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::lottie::zlib::inflate_gzip_or_zlib;

pub const MAP_HTTP_USER_AGENT: &str = "MapLibreNative/1.0";

const ACCEPT_ENCODING: &str = "gzip, deflate";

pub const MAXIMUM_CONCURRENT_REQUESTS: usize = 20;

const DEFAULT_RATE_LIMIT_TIMEOUT_SECONDS: i64 = 5;

const CLOCK_SKEW_RETRY_TIMEOUT_SECONDS: i64 = 30;

const CONNECTION_RETRY_CEILING_SECONDS: u64 = 32;

const MAXIMUM_INFLATED_BODY_BYTES: usize = 64 * 1024 * 1024;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapHttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

/// server answer, body as it came over the wire
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapHttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// a transfer with no answer
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MapHttpFailure {
    Connection(String),
    Other(String),
}

pub type MapHttpAnswer = Result<MapHttpReply, MapHttpFailure>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapHttpErrorKind {
    NotFound,
    Server,
    Connection,
    RateLimit,
    Other,
}

/// what maplibre gets, epoch seconds instead of its time type
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MapHttpContent {
    Data(Vec<u8>),
    NoContent,
    NotModified,
    Failed {
        kind: MapHttpErrorKind,
        message: String,
        retry_after: Option<i64>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapHttpOutcome {
    pub content: MapHttpContent,
    pub modified: Option<i64>,
    pub expires: Option<i64>,
    pub etag: Option<String>,
    pub must_revalidate: bool,
}

/// urls the network source serves
/// mapbox:// was resolved before and the rest have their own sources
pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// builds the GET for a resource, cached validators go in (etag wins) plus byte range
/// accept-encoding is named so the body comes back raw and map_http_outcome inflates it
pub fn map_http_request(
    url: &str,
    prior_etag: Option<&str>,
    prior_modified: Option<i64>,
    data_range: Option<(u64, u64)>,
) -> MapHttpRequest {
    let mut headers = vec![
        ("User-Agent", MAP_HTTP_USER_AGENT.to_owned()),
        ("Accept-Encoding", ACCEPT_ENCODING.to_owned()),
    ];
    if let Some(etag) = prior_etag {
        headers.push(("If-None-Match", etag.to_owned()));
    } else if let Some(modified) = prior_modified.and_then(format_http_date) {
        headers.push(("If-Modified-Since", modified));
    }
    if let Some((start, end)) = data_range {
        headers.push(("Range", format!("bytes={start}-{end}")));
    }
    MapHttpRequest {
        url: url.to_owned(),
        headers,
    }
}

/// turns the answer into what maplibre reads, now in epoch seconds
/// a missing tile is empty not an error, the map draws around it
pub fn map_http_outcome(answer: MapHttpAnswer, is_tile: bool, now: i64) -> MapHttpOutcome {
    let reply = match answer {
        Ok(reply) => reply,
        Err(MapHttpFailure::Connection(message)) => return bare(failed(MapHttpErrorKind::Connection, message)),
        Err(MapHttpFailure::Other(message)) => return bare(failed(MapHttpErrorKind::Other, message)),
    };
    let header = |name: &str| header_value(&reply.headers, name);
    let content = match reply.status {
        200 | 206 => match decoded_body(&reply.body, header("Content-Encoding")) {
            Some(body) => MapHttpContent::Data(body),
            None => failed(MapHttpErrorKind::Other, "the compressed body could not be read".to_owned()),
        },
        204 => MapHttpContent::NoContent,
        304 => MapHttpContent::NotModified,
        404 if is_tile => MapHttpContent::NoContent,
        404 => failed(MapHttpErrorKind::NotFound, status_message(404)),
        429 => MapHttpContent::Failed {
            kind: MapHttpErrorKind::RateLimit,
            message: status_message(429),
            retry_after: retry_time(header("Retry-After"), header("X-Rate-Limit-Reset"), now),
        },
        status @ 500..=599 => failed(MapHttpErrorKind::Server, status_message(status)),
        status => failed(MapHttpErrorKind::Other, status_message(status)),
    };
    let (max_age, must_revalidate) = cache_control(header("Cache-Control"));
    MapHttpOutcome {
        content,
        modified: header("Last-Modified").and_then(parse_http_date),
        expires: max_age
            .map(|seconds| now.saturating_add(seconds))
            .or_else(|| header("Expires").and_then(parse_http_date)),
        etag: header("ETag").map(str::to_owned),
        must_revalidate,
    }
}

/// what the cache knew about the resource before this request
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct MapHttpPrior {
    pub modified: Option<i64>,
    pub expires: Option<i64>,
    pub etag: Option<String>,
}

/// wait before the next attempt after N consecutive failures
/// none when the failure is final
pub fn error_retry_delay(
    kind: MapHttpErrorKind,
    failed_requests: u32,
    retry_after: Option<i64>,
    now: i64,
) -> Option<Duration> {
    let doubled = |exponent: u32| 1_u64 << exponent.min(31);
    match kind {
        MapHttpErrorKind::Server => Some(Duration::from_secs(if failed_requests <= 3 {
            1
        } else {
            doubled(failed_requests - 3)
        })),
        MapHttpErrorKind::Connection => Some(Duration::from_secs(
            doubled(failed_requests.saturating_sub(1)).min(CONNECTION_RETRY_CEILING_SECONDS),
        )),
        MapHttpErrorKind::RateLimit => {
            let seconds = retry_after.map_or(DEFAULT_RATE_LIMIT_TIMEOUT_SECONDS, |time| time.saturating_sub(now));
            Some(Duration::from_secs(u64::try_from(seconds).unwrap_or(0)))
        }
        MapHttpErrorKind::NotFound | MapHttpErrorKind::Other => None,
    }
}

/// how long the first GET waits, at once with no cached lifetime
/// else until the cached copy expires and not before the minimum interval
pub fn first_request_delay(prior_expires: Option<i64>, minimum_update_interval: Duration, now: i64) -> Duration {
    let Some(expires) = prior_expires else {
        return Duration::ZERO;
    };
    let until_expiry = Duration::from_secs(u64::try_from(expires.saturating_sub(now)).unwrap_or(0));
    until_expiry.max(minimum_update_interval)
}

/// expiration timeout raised to the minimum interval, none when nothing is due
fn expiration_delay(expires: Option<i64>, expired_requests: u32, minimum_update_interval: Duration, now: i64) -> Option<Duration> {
    let timeout = if expired_requests > 0 {
        Duration::from_secs(1_u64 << (expired_requests - 1).min(31))
    } else {
        Duration::from_secs(u64::try_from(expires?.saturating_sub(now)).unwrap_or(0))
    };
    Some(timeout.max(minimum_update_interval))
}

/// what the request keeps between attempts of one request
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapHttpAttempts {
    pub prior: MapHttpPrior,
    prior_data: Option<Vec<u8>>,
    minimum_update_interval: Duration,
    failed_requests: u32,
    expired_requests: u32,
}

impl MapHttpAttempts {
    pub fn new(prior: MapHttpPrior, prior_data: Option<Vec<u8>>, minimum_update_interval: Duration) -> Self {
        Self { prior, prior_data, minimum_update_interval, failed_requests: 0, expired_requests: 0 }
    }

    /// gives the outcome the core requester reads, missing validators keep cached values
    /// a lifetime already over is fixed for a wrong clock
    /// also returns the wait before the next attempt, none if none is due
    pub fn completed(&mut self, mut outcome: MapHttpOutcome, now: i64) -> (MapHttpOutcome, Option<Duration>) {
        match outcome.modified {
            Some(modified) => self.prior.modified = Some(modified),
            None => outcome.modified = self.prior.modified,
        }
        if matches!(outcome.content, MapHttpContent::NotModified)
            && let Some(data) = self.prior_data.take()
        {
            outcome.content = MapHttpContent::Data(data);
        }
        let mut is_expired = false;
        if let Some(current) = outcome.expires {
            let prior = self.prior.expires.replace(current);
            let (expires, expired) = interpolate_expiration(current, prior, now);
            outcome.expires = Some(expires);
            is_expired = expired;
        }
        self.expired_requests = if is_expired { self.expired_requests.saturating_add(1) } else { 0 };
        match &outcome.etag {
            Some(etag) => self.prior.etag = Some(etag.clone()),
            None => outcome.etag.clone_from(&self.prior.etag),
        }
        let error_delay = match &outcome.content {
            MapHttpContent::Failed { kind, retry_after, .. } => {
                self.failed_requests = self.failed_requests.saturating_add(1);
                error_retry_delay(*kind, self.failed_requests, *retry_after, now)
            }
            _ => {
                self.failed_requests = 0;
                None
            }
        };
        let expiration_delay =
            expiration_delay(outcome.expires, self.expired_requests, self.minimum_update_interval, now);
        let next = match (error_delay, expiration_delay) {
            (Some(error), Some(expiration)) => Some(error.min(expiration)),
            (error, expiration) => error.or(expiration),
        };
        (outcome, next)
    }
}

fn interpolate_expiration(current: i64, prior: Option<i64>, now: i64) -> (i64, bool) {
    if current > now {
        return (current, false);
    }
    match prior {
        Some(prior) if current > prior => {
            (now.saturating_add((current - prior).max(CLOCK_SKEW_RETRY_TIMEOUT_SECONDS)), false)
        }
        _ => (current, true),
    }
}

/// decides which transfer runs next, at most MAXIMUM_CONCURRENT_REQUESTS at once
/// regular before low priority and fifo inside each
/// timed waits for retries, times are monotonic clock readings
pub struct MapHttpScheduler {
    regular: VecDeque<u64>,
    low: VecDeque<u64>,
    active: HashSet<u64>,
    waiting: Vec<MapHttpWaiter>,
}

struct MapHttpWaiter {
    id: u64,
    low_priority: bool,
    due: Duration,
}

impl MapHttpScheduler {
    pub fn new() -> Self {
        Self {
            regular: VecDeque::new(),
            low: VecDeque::new(),
            active: HashSet::new(),
            waiting: Vec::new(),
        }
    }

    /// transfer may start at due
    pub fn schedule(&mut self, id: u64, low_priority: bool, due: Duration) {
        self.waiting.push(MapHttpWaiter { id, low_priority, due });
    }

    /// moves every transfer whose time came into the queue, earliest first
    pub fn release_due(&mut self, now: Duration) {
        let (mut due, waiting): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|waiter| waiter.due <= now);
        self.waiting = waiting;
        due.sort_by_key(|waiter| waiter.due);
        for waiter in due {
            if waiter.low_priority {
                self.low.push_back(waiter.id);
            } else {
                self.regular.push_back(waiter.id);
            }
        }
    }

    /// next transfer to start, counted as active now
    /// none at the limit or with an empty queue
    pub fn next_start(&mut self) -> Option<u64> {
        if self.active.len() >= MAXIMUM_CONCURRENT_REQUESTS {
            return None;
        }
        let id = self.regular.pop_front().or_else(|| self.low.pop_front())?;
        self.active.insert(id);
        Some(id)
    }

    /// transfer ended and frees its slot
    pub fn finished(&mut self, id: u64) {
        self.active.remove(&id);
    }

    /// requester went away, works wherever the transfer stood
    pub fn cancel(&mut self, id: u64) {
        self.active.remove(&id);
        self.regular.retain(|candidate| *candidate != id);
        self.low.retain(|candidate| *candidate != id);
        self.waiting.retain(|waiter| waiter.id != id);
    }

    /// when the earliest timed wait ends
    pub fn next_due(&self) -> Option<Duration> {
        self.waiting.iter().map(|waiter| waiter.due).min()
    }
}

impl Default for MapHttpScheduler {
    fn default() -> Self {
        Self::new()
    }
}

fn bare(content: MapHttpContent) -> MapHttpOutcome {
    MapHttpOutcome {
        content,
        modified: None,
        expires: None,
        etag: None,
        must_revalidate: false,
    }
}

fn failed(kind: MapHttpErrorKind, message: String) -> MapHttpContent {
    MapHttpContent::Failed {
        kind,
        message,
        retry_after: None,
    }
}

fn status_message(status: u16) -> String {
    format!("HTTP status code {status}")
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| trimming_whitespaces_and_newlines(value))
}

fn decoded_body(body: &[u8], encoding: Option<&str>) -> Option<Vec<u8>> {
    let compressed = encoding.is_some_and(|name| {
        name.eq_ignore_ascii_case("gzip") || name.eq_ignore_ascii_case("x-gzip") || name.eq_ignore_ascii_case("deflate")
    });
    let plain = encoding.is_none_or(|name| name.is_empty() || name.eq_ignore_ascii_case("identity"));
    if body.is_empty() || plain {
        return Some(body.to_vec());
    }
    if compressed {
        return inflate_gzip_or_zlib(body, MAXIMUM_INFLATED_BODY_BYTES);
    }
    None
}

fn cache_control(value: Option<&str>) -> (Option<i64>, bool) {
    let mut max_age = None;
    let mut must_revalidate = false;
    for directive in value.unwrap_or_default().split(',') {
        let directive = trimming_whitespaces_and_newlines(directive);
        if directive.eq_ignore_ascii_case("must-revalidate") {
            must_revalidate = true;
        } else if let Some((name, seconds)) = directive.split_once('=')
            && name.eq_ignore_ascii_case("max-age")
        {
            max_age = seconds.trim_matches('"').parse::<i64>().ok().filter(|seconds| *seconds >= 0);
        }
    }
    (max_age, must_revalidate)
}

fn retry_time(retry_after: Option<&str>, rate_limit_reset: Option<&str>, now: i64) -> Option<i64> {
    if let Some(value) = retry_after {
        return match value.parse::<i64>() {
            Ok(seconds) => Some(now.saturating_add(seconds)),
            Err(_) => parse_http_date(value),
        };
    }
    rate_limit_reset?.parse().ok()
}

/// parses the imf fixdate form only, to epoch seconds
/// eg Sun 06 Nov 1994 08:49:37 GMT
/// none for anything else
pub fn parse_http_date(value: &str) -> Option<i64> {
    let (_, date) = value.split_once(", ")?;
    let mut parts = date.split(' ');
    let day: i64 = parts.next()?.parse().ok()?;
    let month_name = parts.next()?;
    let month = MONTHS.iter().position(|name| *name == month_name)? as i64 + 1;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if parts.next()? != "GMT" || parts.next().is_some() || clock.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day) || !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=60).contains(&second) {
        return None;
    }
    Some(days_from_civil(year, month, day) * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second)
}

/// formats epoch seconds as an imf fixdate, none before year 0
pub fn format_http_date(epoch: i64) -> Option<String> {
    let days = epoch.div_euclid(SECONDS_PER_DAY);
    let of_day = epoch.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    if year < 0 {
        return None;
    }
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    let month_name = MONTHS[(month - 1) as usize];
    Some(format!(
        "{weekday}, {day:02} {month_name} {year:04} {:02}:{:02}:{:02} GMT",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    ))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
