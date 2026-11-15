#![allow(clippy::too_many_arguments)] // Generated WIT ABI.
wit_bindgen::generate!({ path: "cardbe-plugin-api/wit", world: "plugin" });

use cardbe::plugin::board::HostError;
use cardbe::plugin::types::{
    Changeset, ColumnReference, CreateColumn, ExternalTask, HttpMethod, HttpRequest, HttpResponse,
    LogLevel, Operation, Task, TaskInfo, TaskPage, TaskPriority, UpsertExternalTask,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

mod wiki;

struct Jira;
export!(Jira);

/// Retries per run after `http-send` reports `NETWORK_FAILED`, which leaves the run usable.
const NETWORK_RETRIES: u32 = 2;
/// Issues per search page; keeps pages with long descriptions under the 2 MiB response limit.
const PAGE_SIZE: usize = 50;
/// Smaller pages when comments are requested; 100 pages still cover the 2,000-ticket limit.
const COMMENT_PAGE_SIZE: usize = 20;
const COMMENT_FIELD: &str = "comment";
const COMMENTS_PLACEHOLDER: &str = "{comments}";
const SEARCH_FIELDS: [&str; 10] = [
    "summary",
    "description",
    "status",
    "fixVersions",
    "versions",
    "updated",
    "priority",
    "labels",
    "assignee",
    "creator",
];
/// Archive and restore calls share the changeset's 200-operation budget.
const MAX_OPERATIONS: usize = 200;
/// Board tasks per `list-tasks` page while looking for cards to archive.
const TASK_PAGE: u32 = 50;
/// The cursor tracks IDs this plugin archived, within the host's 4 KiB cursor limit.
const CURSOR_PREFIX: &str = "archived:";
const MAX_CURSOR: usize = 4000;
// Host limits for external tasks, in UTF-8 bytes.
const MAX_TITLE: usize = 4096;
const MAX_DESCRIPTION: usize = 262_144;
const MAX_STATE: usize = 64;
const MAX_LABEL: usize = 256;
const MAX_LABELS: usize = 100;
/// Keeps `jira:{site}:{id}` within the 512-byte external key limit.
const MAX_SITE_URL: usize = 480;

#[derive(Deserialize)]
struct Config {
    base_url: String,
    site_url: String,
    jql: String,
    api: String,
    #[serde(default = "default_description_template")]
    description_template: String,
    /// Archive cards whose ticket no longer matches the JQL, and restore them when it does again.
    #[serde(default)]
    archive_missing: bool,
}
fn default_description_template() -> String {
    "{url}\n\n{description}".into()
}
struct Placeholders<'a> {
    url: &'a str,
    description: &'a str,
    description_code: &'a str,
    key: &'a str,
    summary: &'a str,
    comments: &'a str,
}
fn render_description(template: &str, values: &Placeholders) -> String {
    let mut out = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        out.push_str(match &remaining[..=end] {
            "{url}" => values.url,
            "{description}" => values.description,
            "{description_code}" => values.description_code,
            "{key}" => values.key,
            "{summary}" => values.summary,
            COMMENTS_PLACEHOLDER => values.comments,
            other => other,
        });
        remaining = &remaining[end + 1..];
    }
    out.push_str(remaining);
    out
}
struct Column {
    id: i64,
    name: String,
}
#[derive(Deserialize)]
struct Page {
    issues: Vec<Issue>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "isLast")]
    is_last: Option<bool>,
    total: Option<usize>,
    #[serde(rename = "startAt")]
    start_at: Option<usize>,
}
#[derive(Deserialize)]
struct Issue {
    id: String,
    key: String,
    fields: Fields,
}
#[derive(Deserialize)]
struct Fields {
    summary: String,
    description: Option<serde_json::Value>,
    status: Named,
    /// Absent when the field is hidden or unavailable in a project; may also be null.
    #[serde(default, rename = "fixVersions")]
    fix_versions: Option<Vec<Named>>,
    /// Affects versions; absent or null like `fixVersions`.
    #[serde(default, rename = "versions")]
    affects_versions: Option<Vec<Named>>,
    updated: String,
    priority: Option<Named>,
    /// Requested only when the description template uses `{comments}`.
    #[serde(default)]
    comment: Option<CommentPage>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    assignee: Option<Author>,
    #[serde(default)]
    creator: Option<Author>,
}
#[derive(Deserialize)]
struct Named {
    name: String,
}
#[derive(Deserialize)]
struct CommentPage {
    #[serde(default)]
    comments: Option<Vec<Comment>>,
}
#[derive(Deserialize)]
struct Comment {
    #[serde(default)]
    author: Option<Author>,
    #[serde(default)]
    body: Option<serde_json::Value>,
    #[serde(default)]
    created: Option<String>,
}
#[derive(Deserialize)]
struct Author {
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
}
/// Formats comments in Jira order as separate Markdown block quotes: an
/// `Author · YYYY-MM-DD HH:MM` header, an empty quote line, then the body.
fn comments(page: Option<CommentPage>, wiki_markup: bool) -> String {
    let mut out = String::new();
    for comment in page.and_then(|p| p.comments).unwrap_or_default() {
        let mut text = String::new();
        let (body, _) = comment
            .body
            .as_ref()
            .map(|value| text_field(value, wiki_markup))
            .unwrap_or_default();
        let author = comment
            .author
            .and_then(|a| a.display_name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Unknown".into());
        text.push_str(author.trim());
        if let Some(created) = comment.created.filter(|c| !c.is_empty()) {
            text.push_str(" · ");
            // ISO timestamps become `YYYY-MM-DD HH:MM`; other formats are kept as-is.
            match created
                .get(..16)
                .filter(|t| t.as_bytes().get(10) == Some(&b'T'))
            {
                Some(time) => text.push_str(&time.replacen('T', " ", 1)),
                None => text.push_str(&created),
            }
        }
        // The empty quote line keeps the header and body as separate Markdown paragraphs.
        if !body.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(body.trim());
        }
        // A blank line between comments ends each quote, so every comment is its own block.
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let quoted: Vec<_> = text
            .lines()
            .map(|line| {
                let line = line.trim_end();
                if line.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {line}")
                }
            })
            .collect();
        out.push_str(&quoted.join("\n"));
    }
    out
}

fn priority(value: Option<&Named>) -> Option<Option<TaskPriority>> {
    Some(
        match value.map(|v| v.name.trim().to_ascii_lowercase()).as_deref() {
            None => None,
            Some("highest" | "blocker") => Some(TaskPriority::Urgent),
            Some("high" | "critical") => Some(TaskPriority::High),
            Some("medium" | "major") => Some(TaskPriority::Medium),
            Some("low" | "lowest" | "minor" | "trivial") => Some(TaskPriority::Low),
            // shortcut: custom/localized priorities stay unchanged, add explicit mapping when needed.
            Some(_) => return None,
        },
    )
}

fn report_priority(key: &str, priority: Option<TaskPriority>) -> Result<(), PluginError> {
    #[cfg(not(test))]
    cardbe::plugin::board::set_external_task_priority(key, priority).map_err(board_error)?;
    #[cfg(test)]
    let _ = (key, priority);
    Ok(())
}

fn board_error(cause: cardbe::plugin::board::HostError) -> PluginError {
    use cardbe::plugin::board::HostError;
    error(
        match cause {
            HostError::InvalidArgument => "BOARD_INVALID_ARGUMENT",
            HostError::PermissionDenied => "BOARD_PERMISSION_DENIED",
            HostError::NotFound => "BOARD_NOT_FOUND",
            HostError::ResourceLimit => "BOARD_RESOURCE_LIMIT",
            HostError::InternalError => "BOARD_INTERNAL_ERROR",
        },
        None,
    )
}

impl Guest for Jira {
    fn run(context: RunContext) -> Result<RunOutput, PluginError> {
        log("SYNC_STARTED", 1);
        let snapshot = cardbe::plugin::board::list_columns()
            .map_err(|cause| failed(board_error(cause), log))?;
        if snapshot.version != context.board_version {
            return Err(failed(error("BOARD_SNAPSHOT_MISMATCH", None), log));
        }
        log("BOARD_COLUMNS_READ", snapshot.columns.len() as u32);
        let columns: Vec<_> = snapshot
            .columns
            .into_iter()
            .map(|c| Column {
                id: c.id,
                name: c.name,
            })
            .collect();
        let changes = synchronize(&context, &columns, |url, body| {
            cardbe::plugin::host::http_send(&HttpRequest {
                method: HttpMethod::Post,
                url: url.into(),
                headers: vec![
                    ("Content-Type".into(), "application/json".into()),
                    // Jira skips its XSRF check for REST calls carrying this header.
                    ("X-Atlassian-Token".into(), "no-check".into()),
                ],
                body: Some(body.to_vec()),
            })
            .map_err(|cause| {
                let network = cause == "NETWORK_FAILED";
                message(
                    if network {
                        LogLevel::Warn
                    } else {
                        LogLevel::Error
                    },
                    &format!("Host HTTP request failed: {}.", host_code(&cause)),
                );
                if network {
                    error("NETWORK_FAILED", Some(60))
                } else {
                    error("HTTP_FAILED", None)
                }
            })
        })?;
        let operations = changes
            .changeset
            .as_ref()
            .map(|c| c.operations.as_slice())
            .unwrap_or(&[]);
        let created = operations
            .iter()
            .filter(|op| matches!(op, Operation::CreateColumn(_)))
            .count();
        let upserts = operations
            .iter()
            .filter(|op| matches!(op, Operation::UpsertExternalTask(_)))
            .count();
        log("TICKETS_SYNCED", (changes.tasks.len() + upserts) as u32);
        log("COLUMNS_CREATED", created as u32);
        log("SYNC_COMPLETED", 1);
        message(
            LogLevel::Info,
            &format!(
                "Synchronized {} tickets ({} in existing columns, {} via changeset); created {} columns.",
                changes.tasks.len() + upserts,
                changes.tasks.len(),
                upserts,
                created
            ),
        );
        Ok(changes)
    }
}
fn log(code: &str, count: u32) {
    #[cfg(not(test))]
    cardbe::plugin::host::log(code, count);
    #[cfg(test)]
    let _ = (code, count);
}
fn message(level: LogLevel, text: &str) {
    #[cfg(not(test))]
    cardbe::plugin::host::log_message(level, text);
    #[cfg(test)]
    let _ = (level, text);
}
/// Host failures are safe codes; keep only code characters so nothing else is echoed.
fn host_code(cause: &str) -> String {
    if !cause.is_empty()
        && cause.len() <= 64
        && cause
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        cause.into()
    } else {
        "UNKNOWN".into()
    }
}
fn describe(code: &str) -> &'static str {
    match code {
        "CONFIG_JSON_INVALID" => "Configuration is malformed or a required field is missing.",
        "CONFIG_BASE_URL_INVALID" => {
            "The API base URL must use HTTPS with a valid hostname, optional port (1-65535) and context path, without credentials, query parameters or duplicate slashes."
        }
        "CONFIG_BASE_URL_API_PATH" => {
            "Remove /rest/api/... from the API base URL; enter only the Jira root, including any context path and port."
        }
        "CONFIG_SITE_URL_INVALID" => {
            "The Jira site URL must be the HTTPS site root (optional port and context path, at most 480 bytes), not a /browse link."
        }
        "CONFIG_JQL_EMPTY" => "Enter a JQL query.",
        "CONFIG_API_INVALID" => "Select data-center or cloud as the Jira API.",
        "INVALID_JQL" => "Jira returned HTTP 400; check the JQL and search parameters.",
        "AUTHENTICATION_FAILED" => "Jira returned HTTP 401; check or re-enter the token.",
        "PERMISSION_DENIED" => "Jira returned HTTP 403; check the token's Jira access rights.",
        "API_ENDPOINT_NOT_FOUND" | "API_ENDPOINT_REMOVED" => {
            "Jira returned HTTP 404/410; check the API base URL, context path and API selection."
        }
        "PROXY_AUTHENTICATION_REQUIRED" => "HTTP 407; the proxy requires authentication.",
        "JIRA_INTERNAL_SERVER_ERROR" | "JIRA_BAD_GATEWAY" | "JIRA_SERVICE_UNAVAILABLE"
        | "JIRA_GATEWAY_TIMEOUT" => "Jira or its reverse proxy returned HTTP 5xx; retry later.",
        "RATE_LIMITED" => "Jira returned HTTP 429; the next run waits for the Retry-After hint.",
        "API_FAILED" => "Jira returned an unexpected HTTP status.",
        "NETWORK_FAILED" => {
            "DNS, connection, TLS or request timeout failed after retries; check connectivity to Jira."
        }
        "HTTP_FAILED" => "The host rejected or failed the HTTP request; inspect the execution error.",
        "RESPONSE_JSON_INVALID" | "RESPONSE_JSON_TRUNCATED" | "RESPONSE_SCHEMA_INVALID" => {
            "The Jira response was not the expected JSON; check the API selection and proxy."
        }
        "ISSUE_REQUIRED_FIELD_EMPTY" => {
            "An issue has an empty ID, key, summary, status or update timestamp."
        }
        "PAGE_TOKEN_MISSING" | "PAGE_TOKEN_REPEATED" | "PAGE_TOTAL_MISSING"
        | "PAGE_OFFSET_MISMATCH" | "PAGE_EMPTY_BEFORE_TOTAL" => {
            "Jira returned inconsistent pagination metadata; no partial updates were returned."
        }
        "TICKET_LIMIT_EXCEEDED" | "COLUMN_LIMIT_EXCEEDED" | "PAGE_LIMIT_EXCEEDED" => {
            "More than 2,000 tickets, 200 new columns or 100 pages; narrow the JQL."
        }
        "STATUS_COLUMN_DUPLICATED" => "Several board columns match one Jira status; rename them.",
        "STATUS_COLUMN_ID_INVALID" => "A matching board column has an invalid ID.",
        "STATUS_NAME_TOO_LONG" => "A Jira status name exceeds 64 bytes; shorten it in Jira.",
        "BOARD_INVALID_ARGUMENT" | "BOARD_PERMISSION_DENIED" | "BOARD_NOT_FOUND"
        | "BOARD_RESOURCE_LIMIT" | "BOARD_INTERNAL_ERROR" => "Reading the board columns failed.",
        "BOARD_SNAPSHOT_MISMATCH" => "The board changed during the run; run the sync again.",
        _ => "See the plugin README for this error code.",
    }
}
fn failed(failure: PluginError, mut record: impl FnMut(&str, u32)) -> PluginError {
    record("SYNC_FAILED", 1);
    record(&failure.code, 1);
    message(
        LogLevel::Error,
        &format!(
            "Synchronization failed with {}: {}",
            failure.code,
            describe(&failure.code)
        ),
    );
    failure
}
fn error(code: &str, retry_after_seconds: Option<u32>) -> PluginError {
    PluginError {
        code: code.into(),
        retry_after_seconds,
    }
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
/// Returns the canonical `https://host[:port][/path]` form: lowercase host, no default
/// port and no trailing slash. Links and identities use it, so equivalent spellings match.
fn normalize_url(value: &str) -> Option<String> {
    let rest = value.trim().strip_prefix("https://")?.trim_end_matches('/');
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let host = host.to_ascii_lowercase();
    let valid = port.is_none_or(|port| {
        !port.starts_with('0')
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok()
    }) && host.contains('.')
        && host.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && (path.is_empty()
            || path.split('/').all(|part| {
                !matches!(part, "" | "." | "..")
                    && part.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
                    })
            }));
    if !valid {
        return None;
    }
    let mut url = format!("https://{host}");
    if let Some(port) = port.filter(|port| *port != "443") {
        url.push(':');
        url.push_str(port);
    }
    if !path.is_empty() {
        url.push('/');
        url.push_str(path);
    }
    Some(url)
}
fn path_segments(url: &str) -> Vec<&str> {
    url.trim_start_matches("https://")
        .split('/')
        .skip(1)
        .collect()
}
/// Offset pagination needs a stable order, so JQL without ORDER BY is sorted by issue key.
fn ordered_jql(jql: &str) -> String {
    let jql = jql.trim();
    let words: Vec<_> = jql
        .split_whitespace()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    if words
        .windows(2)
        .any(|pair| pair[0] == "order" && pair[1] == "by")
    {
        jql.into()
    } else {
        format!("{jql} ORDER BY key ASC")
    }
}
/// Truncates at a UTF-8 boundary to `max` bytes and records whether anything was cut.
fn limit(mut value: String, max: usize, cut: &mut bool) -> String {
    if value.len() > max {
        let mut end = max;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        *cut = true;
    }
    value
}
/// Returns a Jira text field as (Markdown, plain text). Wiki markup strings from
/// self-hosted Jira are converted to Markdown; the plain text keeps the original.
fn text_field(value: &serde_json::Value, wiki_markup: bool) -> (String, String) {
    let mut plain = String::new();
    description(value, &mut plain);
    let markdown = if wiki_markup && value.is_string() {
        wiki::to_markdown(&plain)
    } else {
        plain.clone()
    };
    (markdown, plain)
}
fn description(value: &serde_json::Value, out: &mut String) {
    if let Some(text) = value.as_str() {
        out.push_str(text);
        return;
    }
    if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
        out.push_str(text);
        if let Some(marks) = value.get("marks").and_then(|v| v.as_array()) {
            for mark in marks {
                if mark["type"] == "link" {
                    if let Some(href) = mark["attrs"]["href"].as_str() {
                        out.push_str(&format!(" ({href})"));
                    }
                }
            }
        }
    }
    if let Some(children) = value.get("content").and_then(|v| v.as_array()) {
        for child in children {
            description(child, out);
        }
    }
    if matches!(
        value["type"].as_str(),
        Some("paragraph" | "heading" | "listItem" | "codeBlock" | "hardBreak")
    ) && !out.ends_with('\n')
    {
        out.push('\n');
    }
}
fn synchronize(
    context: &RunContext,
    columns: &[Column],
    mut get: impl FnMut(&str, &[u8]) -> Result<HttpResponse, PluginError>,
) -> Result<RunOutput, PluginError> {
    synchronize_logged(context, columns, &mut get, log)
}
fn synchronize_logged(
    context: &RunContext,
    columns: &[Column],
    mut get: impl FnMut(&str, &[u8]) -> Result<HttpResponse, PluginError>,
    mut record: impl FnMut(&str, u32),
) -> Result<RunOutput, PluginError> {
    let mut requests = 0;
    let mut responses = 0;
    let mut retries = 0;
    let mut status = 0;
    let result = synchronize_pages(
        context,
        columns,
        |url, body| {
            requests += 1;
            let response = loop {
                match get(url, body) {
                    Err(failure)
                        if failure.code == "NETWORK_FAILED" && retries < NETWORK_RETRIES =>
                    {
                        retries += 1;
                        requests += 1;
                        message(
                            LogLevel::Warn,
                            &format!("Retrying Jira request after a network failure ({retries}/{NETWORK_RETRIES})."),
                        );
                    }
                    result => break result?,
                }
            };
            responses += 1;
            status = u32::from(response.status);
            Ok(response)
        },
        &mut record,
        report_priority,
        &mut HostBoard,
    );
    record("HTTP_REQUESTS", requests);
    record("HTTP_RESPONSES", responses);
    record("HTTP_RETRIES", retries);
    record("HTTP_LAST_STATUS", status);
    result.map_err(|failure| failed(failure, record))
}
fn synchronize_pages(
    context: &RunContext,
    columns: &[Column],
    mut get: impl FnMut(&str, &[u8]) -> Result<HttpResponse, PluginError>,
    mut record: impl FnMut(&str, u32),
    mut report: impl FnMut(&str, Option<TaskPriority>) -> Result<(), PluginError>,
    board: &mut impl BoardTasks,
) -> Result<RunOutput, PluginError> {
    let config: Config = serde_json::from_str(&context.config_json)
        .map_err(|_| error("CONFIG_JSON_INVALID", None))?;
    let Some(base) = normalize_url(&config.base_url) else {
        return Err(error("CONFIG_BASE_URL_INVALID", None));
    };
    let base_path = path_segments(&base);
    if base_path.last() == Some(&"rest") || base_path.windows(2).any(|w| w == ["rest", "api"]) {
        return Err(error("CONFIG_BASE_URL_API_PATH", None));
    }
    let Some(site) = normalize_url(&config.site_url)
        .filter(|site| site.len() <= MAX_SITE_URL && !path_segments(site).contains(&"browse"))
    else {
        return Err(error("CONFIG_SITE_URL_INVALID", None));
    };
    if config.jql.trim().is_empty() {
        return Err(error("CONFIG_JQL_EMPTY", None));
    }
    let cloud = match config.api.as_str() {
        "cloud" => true,
        "data-center" => false,
        _ => return Err(error("CONFIG_API_INVALID", None)),
    };
    // POST keeps long JQL out of the 4,096-byte URL limit.
    let (url, jql) = if cloud {
        (
            format!("{base}/rest/api/3/search/jql"),
            config.jql.trim().to_string(),
        )
    } else {
        (
            format!("{base}/rest/api/2/search"),
            ordered_jql(&config.jql),
        )
    };
    let with_comments = config.description_template.contains(COMMENTS_PLACEHOLDER);
    let mut fields = SEARCH_FIELDS.to_vec();
    if with_comments {
        fields.push(COMMENT_FIELD);
    }
    let page_size = if with_comments {
        COMMENT_PAGE_SIZE
    } else {
        PAGE_SIZE
    };
    let mut tasks: BTreeMap<String, ExternalTask> = BTreeMap::new();
    // Jira issue key -> status, for matching board cards when archiving.
    let mut issue_keys: BTreeMap<String, String> = BTreeMap::new();
    let mut priorities = BTreeMap::new();
    let mut truncated = 0;
    let mut finished = false;
    let mut start = 0;
    let mut token: Option<String> = None;
    let mut tokens = BTreeSet::new();
    for _ in 0..100 {
        let mut search = serde_json::json!({
            "jql": jql,
            "fields": fields,
            "maxResults": page_size,
        });
        if cloud {
            if let Some(token) = &token {
                search["nextPageToken"] = token.as_str().into();
            }
        } else {
            search["startAt"] = start.into();
        }
        let body = serde_json::to_vec(&search).expect("search request serializes");
        let response = get(&url, &body)?;
        match response.status {
            200 => {}
            400 => return Err(error("INVALID_JQL", None)),
            401 => return Err(error("AUTHENTICATION_FAILED", None)),
            403 => return Err(error("PERMISSION_DENIED", None)),
            404 => return Err(error("API_ENDPOINT_NOT_FOUND", None)),
            407 => return Err(error("PROXY_AUTHENTICATION_REQUIRED", None)),
            410 => return Err(error("API_ENDPOINT_REMOVED", None)),
            500 => return Err(error("JIRA_INTERNAL_SERVER_ERROR", Some(60))),
            502 => return Err(error("JIRA_BAD_GATEWAY", Some(60))),
            503 => return Err(error("JIRA_SERVICE_UNAVAILABLE", Some(60))),
            504 => return Err(error("JIRA_GATEWAY_TIMEOUT", Some(60))),
            429 => {
                return Err(error(
                    "RATE_LIMITED",
                    Some(
                        response
                            .headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                            .and_then(|(_, v)| v.parse::<u32>().ok())
                            .unwrap_or(60)
                            .clamp(60, 86400),
                    ),
                ))
            }
            other => {
                message(
                    LogLevel::Error,
                    &format!("Jira returned unexpected HTTP status {other}."),
                );
                return Err(error("API_FAILED", Some(60)));
            }
        }
        let page: Page = serde_json::from_slice(&response.body).map_err(|cause| {
            record("RESPONSE_JSON_ERROR_LINE", cause.line() as u32);
            record("RESPONSE_JSON_ERROR_COLUMN", cause.column() as u32);
            let (code, kind) = match cause.classify() {
                serde_json::error::Category::Data => {
                    ("RESPONSE_SCHEMA_INVALID", "unexpected field")
                }
                serde_json::error::Category::Eof => ("RESPONSE_JSON_TRUNCATED", "truncated JSON"),
                _ => ("RESPONSE_JSON_INVALID", "invalid JSON"),
            };
            // The serde message can quote response content, so only its position is reported.
            message(
                LogLevel::Error,
                &format!(
                    "Jira response has {kind} at line {}, column {}; the body is not logged.",
                    cause.line(),
                    cause.column()
                ),
            );
            error(code, None)
        })?;
        let count = page.issues.len();
        for issue in page.issues {
            if issue.id.is_empty()
                || issue.key.is_empty()
                || issue.fields.summary.trim().is_empty()
                || issue.fields.status.name.trim().is_empty()
                || issue.fields.updated.is_empty()
            {
                return Err(error("ISSUE_REQUIRED_FIELD_EMPTY", None));
            }
            // The status becomes a column name, so it cannot be truncated without merging columns.
            if issue.fields.status.name.len() > MAX_STATE {
                return Err(error("STATUS_NAME_TOO_LONG", None));
            }
            let link = format!("{site}/browse/{}", encode(&issue.key));
            // Self-hosted Jira returns wiki markup strings; Cloud returns ADF documents.
            let (body, plain) = issue
                .fields
                .description
                .as_ref()
                .map(|value| text_field(value, !cloud))
                .unwrap_or_default();
            let plain = plain.trim();
            let code = if plain.is_empty() {
                String::new()
            } else {
                wiki::code_block(plain, "")
            };
            let notes = comments(issue.fields.comment, !cloud);
            let mut cut = false;
            // Assignee and creator come first so they survive the label limit.
            let people = [
                ("assignee", issue.fields.assignee),
                ("creator", issue.fields.creator),
            ]
            .into_iter()
            .filter_map(|(role, user)| {
                let name = user?.display_name?;
                let name = name.trim();
                (!name.is_empty()).then(|| format!("{role}:{name}"))
            });
            // Version tags are prefixed so fix and affects versions stay distinguishable.
            let versions = |prefix: &'static str, list: Option<Vec<Named>>| {
                list.unwrap_or_default()
                    .into_iter()
                    .filter(|v| !v.name.trim().is_empty())
                    .map(move |v| format!("{prefix}:{}", v.name.trim()))
            };
            let tags: BTreeSet<_> = versions("fix", issue.fields.fix_versions)
                .chain(versions("affects", issue.fields.affects_versions))
                .chain(issue.fields.labels.unwrap_or_default())
                .collect();
            let mut labels: Vec<String> = vec![];
            for label in people.chain(tags) {
                let label = limit(label, MAX_LABEL, &mut cut);
                if !label.trim().is_empty() && !labels.contains(&label) {
                    labels.push(label);
                }
            }
            if labels.len() > MAX_LABELS {
                labels.truncate(MAX_LABELS);
                cut = true;
            }
            let task = ExternalTask {
                external_key: format!("jira:{site}:{}", issue.id),
                title: limit(
                    format!("{} {}", issue.key, issue.fields.summary),
                    MAX_TITLE,
                    &mut cut,
                ),
                description: limit(
                    render_description(
                        &config.description_template,
                        &Placeholders {
                            url: &link,
                            description: body.trim(),
                            description_code: &code,
                            key: &issue.key,
                            summary: &issue.fields.summary,
                            comments: &notes,
                        },
                    ),
                    MAX_DESCRIPTION,
                    &mut cut,
                ),
                url: link,
                state: issue.fields.status.name,
                labels,
                updated_at: issue.fields.updated,
                column_id: 0,
            };
            truncated += u32::from(cut);
            if tasks
                .get(&task.external_key)
                .is_none_or(|old| task.updated_at >= old.updated_at)
            {
                priorities.insert(
                    task.external_key.clone(),
                    priority(issue.fields.priority.as_ref()),
                );
                issue_keys.insert(issue.key.clone(), task.state.trim().to_string());
                tasks.insert(task.external_key.clone(), task);
            }
            if tasks.len() > 2000 {
                return Err(error("TICKET_LIMIT_EXCEEDED", None));
            }
        }
        if cloud {
            if page.is_last == Some(true) {
                finished = true;
                break;
            }
            let next = page
                .next_page_token
                .filter(|v| !v.is_empty())
                .ok_or_else(|| error("PAGE_TOKEN_MISSING", None))?;
            if !tokens.insert(next.clone()) {
                return Err(error("PAGE_TOKEN_REPEATED", None));
            }
            token = Some(next);
        } else {
            let total = page
                .total
                .ok_or_else(|| error("PAGE_TOTAL_MISSING", None))?;
            if page.start_at != Some(start) {
                return Err(error("PAGE_OFFSET_MISMATCH", None));
            }
            start += count;
            if start >= total {
                finished = true;
                break;
            }
            if count == 0 {
                return Err(error("PAGE_EMPTY_BEFORE_TOTAL", None));
            }
        }
    }
    if !finished {
        return Err(error("PAGE_LIMIT_EXCEEDED", None));
    }
    if truncated > 0 {
        record("FIELDS_TRUNCATED", truncated);
        message(
            LogLevel::Warn,
            &format!("Truncated the title, description or labels of {truncated} tickets to fit Cardbe limits."),
        );
    }
    let (mut result, destinations) = output(context, columns, tasks)?;
    for (key, value) in priorities {
        if let Some(value) = value {
            report(&key, value)?;
        }
    }
    result.cursor = if config.archive_missing {
        if !config.description_template.contains("{url}") {
            message(
                LogLevel::Warn,
                "Archiving finds cards by their ticket link; add {url} to the description template.",
            );
        }
        let issues = issue_keys
            .into_iter()
            .map(|(key, status)| (key, destinations[&status].clone()))
            .collect();
        let used = result.changeset.as_ref().map_or(0, |c| c.operations.len());
        reconcile(
            &site,
            &issues,
            context.cursor.as_deref(),
            MAX_OPERATIONS.saturating_sub(used),
            &context.board_version,
            board,
            &mut record,
        )?
    } else {
        context.cursor.clone()
    };
    Ok(result)
}

/// Board task access for archiving; tests replace the host implementation.
trait BoardTasks {
    fn list_active(&mut self, offset: u32, limit: u32) -> Result<TaskPage, HostError>;
    /// None when `id` is not an archived task.
    fn archived(&mut self, id: i64) -> Result<Option<TaskInfo>, HostError>;
    fn archive(&mut self, id: i64) -> Result<(), HostError>;
    fn restore(&mut self, id: i64, column: &ColumnReference) -> Result<(), HostError>;
}
struct HostBoard;
#[cfg(not(test))]
impl BoardTasks for HostBoard {
    fn list_active(&mut self, offset: u32, limit: u32) -> Result<TaskPage, HostError> {
        use cardbe::plugin::types::{DueFilter, TaskQuery, TaskSort, TaskStatus};
        cardbe::plugin::board::list_tasks(&TaskQuery {
            query: String::new(),
            column_id: None,
            status: TaskStatus::Active,
            sort: TaskSort::Title,
            due: DueFilter::All,
            due_start: None,
            due_end: None,
            archive_start: None,
            archive_end: None,
            search_options: None,
            offset,
            limit,
        })
    }
    fn archived(&mut self, id: i64) -> Result<Option<TaskInfo>, HostError> {
        match cardbe::plugin::board::get_task(id, true) {
            Ok(snapshot) => Ok(Some(snapshot.task).filter(|info| info.archived)),
            Err(HostError::NotFound) => Ok(None),
            Err(cause) => Err(cause),
        }
    }
    fn archive(&mut self, id: i64) -> Result<(), HostError> {
        cardbe::plugin::board::archive_task(&cardbe::plugin::types::TaskReference::Id(id))
    }
    fn restore(&mut self, id: i64, column: &ColumnReference) -> Result<(), HostError> {
        cardbe::plugin::board::restore_task(id, column)
    }
}
#[cfg(test)]
impl BoardTasks for HostBoard {
    fn list_active(&mut self, _: u32, _: u32) -> Result<TaskPage, HostError> {
        unreachable!("tests archive through a mock board")
    }
    fn archived(&mut self, _: i64) -> Result<Option<TaskInfo>, HostError> {
        unreachable!("tests archive through a mock board")
    }
    fn archive(&mut self, _: i64) -> Result<(), HostError> {
        unreachable!("tests archive through a mock board")
    }
    fn restore(&mut self, _: i64, _: &ColumnReference) -> Result<(), HostError> {
        unreachable!("tests archive through a mock board")
    }
}

/// The Jira key of a card this source manages: its title starts with the key and its
/// description links to that key on this site. Cards edited by the user stop matching
/// and are never archived.
fn source_key<'a>(site: &str, task: &'a Task) -> Option<&'a str> {
    let key = task.title.split(' ').next()?;
    let (project, number) = key.rsplit_once('-')?;
    let valid = project
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_uppercase())
        && project
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit());
    if !valid {
        return None;
    }
    let link = format!("{site}/browse/{key}");
    let mut rest = task.description.as_str();
    while let Some(i) = rest.find(&link) {
        let after = &rest[i + link.len()..];
        if !after
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Some(key);
        }
        rest = after;
    }
    None
}

fn encode_cursor(ids: &[i64]) -> Option<String> {
    // Oldest IDs are dropped first when the list outgrows the cursor.
    for start in 0..ids.len() {
        let list: Vec<_> = ids[start..].iter().map(i64::to_string).collect();
        let cursor = format!("{CURSOR_PREFIX}{}", list.join(","));
        if cursor.len() <= MAX_CURSOR {
            return Some(cursor);
        }
    }
    None
}

/// Restores cards this plugin archived whose ticket matches again, then archives active
/// cards of this source whose ticket is no longer returned. Returns the new cursor.
fn reconcile(
    site: &str,
    issues: &BTreeMap<String, ColumnReference>,
    cursor: Option<&str>,
    mut budget: usize,
    version: &str,
    board: &mut impl BoardTasks,
    record: &mut impl FnMut(&str, u32),
) -> Result<Option<String>, PluginError> {
    if issues.is_empty() {
        // Jira returns no results, rather than an error, for queries the token cannot see.
        message(
            LogLevel::Warn,
            "The JQL returned no tickets; archiving was skipped so an emptied query cannot archive every card.",
        );
        return Ok(cursor.map(str::to_string));
    }
    let tracked: Vec<i64> = cursor
        .and_then(|c| c.strip_prefix(CURSOR_PREFIX))
        .map(|ids| ids.split(',').filter_map(|id| id.parse().ok()).collect())
        .unwrap_or_default();
    let mut kept = vec![];
    let mut restored = 0;
    for id in tracked {
        // Cards restored or removed by the user are no longer tracked.
        let Some(info) = board.archived(id).map_err(board_error)? else {
            continue;
        };
        if let Some(column) = source_key(site, &info.task).and_then(|key| issues.get(key)) {
            if budget > 0 {
                board.restore(id, column).map_err(board_error)?;
                budget -= 1;
                restored += 1;
                continue;
            }
        }
        kept.push(id);
    }
    let mut archived = 0;
    let mut deferred = 0;
    let mut offset = 0;
    loop {
        let page = match board.list_active(offset, TASK_PAGE) {
            Ok(page) => page,
            Err(HostError::ResourceLimit) => {
                message(
                    LogLevel::Warn,
                    "Listing board cards reached the host query budget; remaining cards are checked next run.",
                );
                break;
            }
            Err(cause) => return Err(board_error(cause)),
        };
        if page.version != version {
            return Err(error("BOARD_SNAPSHOT_MISMATCH", None));
        }
        let count = page.tasks.len();
        for info in page.tasks {
            // Recurring cards would continue with a new occurrence instead of leaving.
            if info.archived || info.task.recurrence.is_some() {
                continue;
            }
            match source_key(site, &info.task) {
                Some(key) if !issues.contains_key(key) => {}
                _ => continue,
            }
            if budget == 0 {
                deferred += 1;
                continue;
            }
            board.archive(info.task.id).map_err(board_error)?;
            budget -= 1;
            archived += 1;
            kept.push(info.task.id);
        }
        if count < TASK_PAGE as usize {
            break;
        }
        offset += TASK_PAGE;
    }
    record("TICKETS_ARCHIVED", archived);
    record("TICKETS_RESTORED", restored);
    if deferred > 0 {
        record("ARCHIVE_DEFERRED", deferred);
        message(
            LogLevel::Warn,
            &format!("Archiving {deferred} more cards exceeds this run's 200-operation limit; they are archived next run."),
        );
    }
    message(
        LogLevel::Info,
        &format!("Archived {archived} cards whose tickets no longer match; restored {restored}."),
    );
    Ok(encode_cursor(&kept))
}

fn output(
    context: &RunContext,
    columns: &[Column],
    tasks: BTreeMap<String, ExternalTask>,
) -> Result<(RunOutput, BTreeMap<String, ColumnReference>), PluginError> {
    let mut destinations = BTreeMap::new();
    let mut operations = vec![];
    let mut existing = vec![];
    let mut created = 0;
    for mut task in tasks.into_values() {
        let status = task.state.trim();
        if !destinations.contains_key(status) {
            let matching: Vec<_> = columns.iter().filter(|c| c.name.trim() == status).collect();
            let destination = match matching.as_slice() {
                [] => {
                    if created >= 200 {
                        return Err(error("COLUMN_LIMIT_EXCEEDED", None));
                    }
                    let key = format!("status:{created}");
                    created += 1;
                    operations.push(Operation::CreateColumn(CreateColumn {
                        key: key.clone(),
                        name: status.into(),
                        color: String::new(),
                    }));
                    ColumnReference::Temporary(key)
                }
                [column] if column.id >= 0 => ColumnReference::Id(column.id),
                [_] => return Err(error("STATUS_COLUMN_ID_INVALID", None)),
                _ => return Err(error("STATUS_COLUMN_DUPLICATED", None)),
            };
            destinations.insert(status.to_string(), destination);
        }
        match &destinations[status] {
            ColumnReference::Id(id) => {
                task.column_id = *id;
                existing.push(task);
            }
            destination => operations.push(Operation::UpsertExternalTask(UpsertExternalTask {
                task,
                column: Some(destination.clone()),
            })),
        }
    }
    let output = RunOutput {
        tasks: existing,
        changeset: if operations.is_empty() {
            None
        } else {
            Some(Changeset {
                version: context.board_version.clone(),
                operations,
            })
        },
        cursor: None,
    };
    Ok((output, destinations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn priority_reports_require_valid_output_and_propagate_host_errors() {
        let mut item = issue();
        item["fields"]["priority"] = json!({"name":"High"});
        let invalid_columns = vec![Column {
            id: -1,
            name: "In Progress".into(),
        }];
        let failure = synchronize_pages(
            &context("cloud"),
            &invalid_columns,
            |_, _| Ok(response(json!({"issues":[item.clone()],"isLast":true}))),
            |_, _| {},
            |_, _| panic!("invalid output must not stage priorities"),
            &mut HostBoard,
        )
        .unwrap_err();
        assert_eq!(failure.code, "STATUS_COLUMN_ID_INVALID");
        let failure = synchronize_pages(
            &context("cloud"),
            &columns(),
            |_, _| Ok(response(json!({"issues":[item.clone()],"isLast":true}))),
            |_, _| {},
            |_, _| {
                Err(board_error(
                    cardbe::plugin::board::HostError::PermissionDenied,
                ))
            },
            &mut HostBoard,
        )
        .unwrap_err();
        assert_eq!(failure.code, "BOARD_PERMISSION_DENIED");
    }
    #[test]
    fn source_priorities_follow_latest_issue_and_include_new_columns() {
        for api in ["cloud", "data-center"] {
            let mut first = issue();
            first["fields"]["priority"] = json!({"name":"Low"});
            let mut latest = first.clone();
            latest["fields"]["updated"] = "2026-10-20".into();
            latest["fields"]["priority"] = json!({"name":"Highest"});
            let mut cleared = issue();
            cleared["id"] = "2".into();
            cleared["fields"]["priority"] = serde_json::Value::Null;
            let mut custom = issue();
            custom["id"] = "3".into();
            custom["fields"]["priority"] = json!({"name":"Custom"});
            let mut reported = vec![];
            let result = synchronize_pages(
                &context(api),
                &[],
                |_, _| {
                    Ok(response(json!({
                        "issues":[first,latest,cleared,custom],"isLast":true,"startAt":0,"total":4
                    })))
                },
                |_, _| {},
                |key, value| {
                    reported.push((key.to_string(), value));
                    Ok(())
                },
                &mut HostBoard,
            )
            .unwrap();
            assert!(result.changeset.is_some());
            assert_eq!(
                reported,
                vec![
                    (
                        "jira:https://jira.example.com:1".into(),
                        Some(TaskPriority::Urgent)
                    ),
                    ("jira:https://jira.example.com:2".into(), None),
                ]
            );
        }
    }

    #[test]
    fn priority_mapping_uses_names_not_site_specific_ids() {
        for (name, expected) in [
            ("Highest", TaskPriority::Urgent),
            ("Blocker", TaskPriority::Urgent),
            ("High", TaskPriority::High),
            ("Critical", TaskPriority::High),
            ("Medium", TaskPriority::Medium),
            ("Major", TaskPriority::Medium),
            ("Low", TaskPriority::Low),
            ("Lowest", TaskPriority::Low),
            ("Minor", TaskPriority::Low),
            (" trivial ", TaskPriority::Low),
        ] {
            assert_eq!(
                priority(Some(&Named { name: name.into() })),
                Some(Some(expected))
            );
        }
        assert_eq!(priority(None), Some(None));
        assert_eq!(
            priority(Some(&Named {
                name: "自訂".into()
            })),
            None
        );
    }
    fn context(api: &str) -> RunContext {
        RunContext { instance_id:"i".into(),run_id:"r".into(),cursor:None,
            config_json:json!({"api":api,"base_url":"https://jira.example.com","site_url":"https://jira.example.com","jql":"project = APP"}).to_string(),
            board_id: 1, board_version: "snapshot".into(), trigger: "manual".into() }
    }
    fn columns() -> Vec<Column> {
        vec![Column {
            id: 0,
            name: "In Progress".into(),
        }]
    }
    fn issue() -> serde_json::Value {
        json!({"id":"1","key":"APP-1","fields":{"summary":"Fix","description":"Details","status":{"name":"In Progress"},"fixVersions":[{"name":"1.2"},{"name":"1.2"}],"updated":"2026-10-10T12:00:00.000+0000"}})
    }
    fn response(value: serde_json::Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&value).unwrap(),
        }
    }
    #[test]
    fn diagnostics_report_status_and_failures_without_response_content() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../manifest.json")).unwrap();
        for (status, expected) in [
            (401, "AUTHENTICATION_FAILED"),
            (404, "API_ENDPOINT_NOT_FOUND"),
            (503, "JIRA_SERVICE_UNAVAILABLE"),
            (200, "RESPONSE_JSON_INVALID"),
        ] {
            let mut logs = vec![];
            let failure = synchronize_logged(
                &context("cloud"),
                &columns(),
                |_, _| {
                    Ok(HttpResponse {
                        status,
                        headers: vec![],
                        body: b"<html>private data</html>".to_vec(),
                    })
                },
                |code, count| logs.push((code.to_string(), count)),
            )
            .unwrap_err();
            assert_eq!(failure.code, expected);
            assert!(manifest["error_codes"]
                .as_array()
                .unwrap()
                .contains(&json!(expected)));
            let mut expected_logs: Vec<(String, u32)> = vec![];
            if status == 200 {
                expected_logs.push(("RESPONSE_JSON_ERROR_LINE".into(), 1));
                expected_logs.push(("RESPONSE_JSON_ERROR_COLUMN".into(), 1));
            }
            expected_logs.extend([
                ("HTTP_REQUESTS".into(), 1),
                ("HTTP_RESPONSES".into(), 1),
                ("HTTP_RETRIES".into(), 0),
                ("HTTP_LAST_STATUS".into(), u32::from(status)),
                ("SYNC_FAILED".into(), 1),
                (expected.into(), 1),
            ]);
            assert_eq!(logs, expected_logs);
            for (code, _) in logs {
                assert!(manifest["log_codes"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(code)));
            }
        }
        let mut logs = vec![];
        synchronize_logged(
            &context("cloud"),
            &columns(),
            |_, _| Err(error("HTTP_FAILED", None)),
            |code, count| logs.push((code.to_string(), count)),
        )
        .unwrap_err();
        assert_eq!(
            logs,
            vec![
                ("HTTP_REQUESTS".into(), 1),
                ("HTTP_RESPONSES".into(), 0),
                ("HTTP_RETRIES".into(), 0),
                ("HTTP_LAST_STATUS".into(), 0),
                ("SYNC_FAILED".into(), 1),
                ("HTTP_FAILED".into(), 1),
            ]
        );
        for code in manifest["error_codes"].as_array().unwrap() {
            let code = code.as_str().unwrap();
            let legacy = matches!(
                code,
                "INVALID_CONFIG"
                    | "INVALID_RESPONSE"
                    | "CHANGESET_TOO_LARGE"
                    | "STATUS_COLUMN_NOT_FOUND"
                    | "BOARD_QUERY_FAILED"
            );
            assert_eq!(
                describe(code) == describe(""),
                legacy,
                "description for {code}"
            );
        }
        assert_eq!(host_code("NETWORK_FAILED"), "NETWORK_FAILED");
        assert_eq!(host_code("bad https://x/?q"), "UNKNOWN");
    }
    #[test]
    fn retries_network_failures_with_a_bounded_budget() {
        let mut logs = vec![];
        let mut calls = 0;
        let changes = synchronize_logged(
            &context("cloud"),
            &columns(),
            |_, _| {
                calls += 1;
                if calls == 1 {
                    Err(error("NETWORK_FAILED", Some(60)))
                } else {
                    Ok(response(json!({"issues":[issue()],"isLast":true})))
                }
            },
            |code, count| logs.push((code.to_string(), count)),
        )
        .unwrap();
        assert_eq!(changes.tasks.len(), 1);
        assert!(logs.contains(&("HTTP_REQUESTS".into(), 2)));
        assert!(logs.contains(&("HTTP_RETRIES".into(), 1)));
        let mut logs = vec![];
        let mut calls = 0;
        let failure = synchronize_logged(
            &context("cloud"),
            &columns(),
            |_, _| {
                calls += 1;
                Err(error("NETWORK_FAILED", Some(60)))
            },
            |code, count| logs.push((code.to_string(), count)),
        )
        .unwrap_err();
        assert_eq!(failure.code, "NETWORK_FAILED");
        assert_eq!(calls, 1 + NETWORK_RETRIES);
        assert!(logs.contains(&("HTTP_RETRIES".into(), NETWORK_RETRIES)));
    }
    fn configured(api: &str, changes: serde_json::Value) -> RunContext {
        let mut ctx = context(api);
        let mut config: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap();
        for (key, value) in changes.as_object().unwrap() {
            config[key] = value.clone();
        }
        ctx.config_json = config.to_string();
        ctx
    }
    #[test]
    fn urls_are_normalized_and_keep_custom_ports() {
        assert_eq!(
            normalize_url(" https://Jira.Example.com:8443/jira/ ").as_deref(),
            Some("https://jira.example.com:8443/jira")
        );
        assert_eq!(
            normalize_url("https://jira.example.com:443").as_deref(),
            Some("https://jira.example.com")
        );
        assert_eq!(
            normalize_url("https://jira.example.com:65535").as_deref(),
            Some("https://jira.example.com:65535")
        );
        for invalid in [
            ":", ":0", ":08443", ":65536", ":abc", ":1:2", ":-1", ":8443:", "//jira", "/a//b",
        ] {
            assert_eq!(
                normalize_url(&format!("https://jira.example.com{invalid}")),
                None,
                "{invalid}"
            );
        }
        let ctx = configured(
            "data-center",
            json!({"base_url":"https://Jira.Example.com:8443/jira/","site_url":"https://jira.example.com:8443/jira"}),
        );
        let mut urls = vec![];
        let changes = synchronize(&ctx, &columns(), |url, _| {
            urls.push(url.to_string());
            Ok(response(json!({"issues":[issue()],"startAt":0,"total":1})))
        })
        .unwrap();
        assert_eq!(
            urls,
            ["https://jira.example.com:8443/jira/rest/api/2/search"]
        );
        let task = &changes.tasks[0];
        assert_eq!(task.url, "https://jira.example.com:8443/jira/browse/APP-1");
        assert_eq!(
            task.external_key,
            "jira:https://jira.example.com:8443/jira:1"
        );
        // An explicit default port keeps the same identity as the plain URL.
        let ctx = configured(
            "data-center",
            json!({"site_url":"https://JIRA.example.com:443/"}),
        );
        let changes = synchronize(&ctx, &columns(), |_, _| {
            Ok(response(json!({"issues":[issue()],"startAt":0,"total":1})))
        })
        .unwrap();
        assert_eq!(
            changes.tasks[0].external_key,
            "jira:https://jira.example.com:1"
        );
    }
    #[test]
    fn rejects_api_paths_and_browse_links_in_url_settings() {
        let long_site = format!("https://jira.example.com/{}", "a".repeat(480));
        for (key, value, expected) in [
            (
                "base_url",
                "https://jira.example.com:8443/jira/rest/api/2",
                "CONFIG_BASE_URL_API_PATH",
            ),
            (
                "base_url",
                "https://jira.example.com/rest",
                "CONFIG_BASE_URL_API_PATH",
            ),
            (
                "base_url",
                "https://jira.example.com:8080x",
                "CONFIG_BASE_URL_INVALID",
            ),
            (
                "site_url",
                "https://jira.example.com/browse/APP-1",
                "CONFIG_SITE_URL_INVALID",
            ),
            ("site_url", long_site.as_str(), "CONFIG_SITE_URL_INVALID"),
        ] {
            let mut changes = serde_json::Map::new();
            changes.insert(key.into(), value.into());
            let ctx = configured("data-center", changes.into());
            let failure = synchronize(&ctx, &columns(), |_, _| unreachable!()).unwrap_err();
            assert_eq!(failure.code, expected, "{key}={value}");
        }
    }
    #[test]
    fn data_center_search_is_ordered_unless_jql_sorts() {
        assert_eq!(
            ordered_jql(" project = APP "),
            "project = APP ORDER BY key ASC"
        );
        assert_eq!(
            ordered_jql("project = APP order\n by updated"),
            "project = APP order\n by updated"
        );
        for (api, expected) in [
            ("data-center", "project = APP ORDER BY key ASC"),
            ("cloud", "project = APP"),
        ] {
            let mut jql = String::new();
            synchronize(&context(api), &columns(), |_, body| {
                let search: serde_json::Value = serde_json::from_slice(body).unwrap();
                jql = search["jql"].as_str().unwrap().into();
                Ok(response(
                    json!({"issues":[],"startAt":0,"total":0,"isLast":true}),
                ))
            })
            .unwrap();
            assert_eq!(jql, expected);
        }
    }
    #[test]
    fn missing_versions_and_oversized_fields_do_not_fail_the_sync() {
        let mut hidden = issue();
        hidden["fields"]
            .as_object_mut()
            .unwrap()
            .remove("fixVersions");
        let mut nulled = issue();
        nulled["id"] = "2".into();
        nulled["fields"]["fixVersions"] = serde_json::Value::Null;
        let mut large = issue();
        large["id"] = "3".into();
        large["fields"]["description"] = "界".repeat(100_000).into();
        large["fields"]["summary"] = "s".repeat(5000).into();
        large["fields"]["fixVersions"] = (0..150)
            .map(|i| json!({"name": format!("{i:03}{}", "版".repeat(100))}))
            .collect();
        let changes = synchronize(&context("cloud"), &columns(), |_, _| {
            Ok(response(
                json!({"issues":[hidden, nulled, large],"isLast":true}),
            ))
        })
        .unwrap();
        assert!(changes.tasks[0].labels.is_empty());
        assert!(changes.tasks[1].labels.is_empty());
        let large = &changes.tasks[2];
        assert!(large.description.len() <= MAX_DESCRIPTION);
        assert!(large.description.ends_with('界'));
        assert_eq!(large.title.len(), MAX_TITLE);
        assert_eq!(large.labels.len(), MAX_LABELS);
        assert!(large.labels.iter().all(|label| label.len() <= MAX_LABEL));

        let mut long_status = issue();
        long_status["fields"]["status"]["name"] = "狀".repeat(22).into();
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| Ok(response(
                json!({"issues":[long_status],"isLast":true})
            )))
            .unwrap_err()
            .code,
            "STATUS_NAME_TOO_LONG"
        );
    }
    #[test]
    fn creates_missing_columns_once_and_reuses_existing_columns() {
        let mut second = issue();
        second["id"] = "2".into();
        let mut third = issue();
        third["id"] = "3".into();
        third["fields"]["status"]["name"] = "Done".into();
        let changes = synchronize(&context("cloud"), &columns(), |_, _| {
            Ok(response(
                json!({"issues":[issue(),second,third],"isLast":true}),
            ))
        })
        .unwrap();
        assert_eq!(changes.tasks.len(), 2);
        let changeset = changes.changeset.unwrap();
        assert_eq!(changeset.version, "snapshot");
        assert_eq!(changeset.operations.len(), 2);
        let Operation::CreateColumn(column) = &changeset.operations[0] else {
            panic!("expected column creation")
        };
        assert_eq!(column.name, "Done");
        let Operation::UpsertExternalTask(upsert) = &changeset.operations[1] else {
            panic!("expected upsert")
        };
        assert!(
            matches!(&upsert.column, Some(ColumnReference::Temporary(key)) if key == &column.key)
        );

        let mut second = issue();
        second["id"] = "2".into();
        let changes = synchronize(&context("cloud"), &[], |_, _| {
            Ok(response(json!({"issues":[issue(),second],"isLast":true})))
        })
        .unwrap();
        let operations = changes.changeset.unwrap().operations;
        assert_eq!(operations.len(), 3);
        assert!(matches!((&operations[1], &operations[2]),
            (Operation::UpsertExternalTask(a), Operation::UpsertExternalTask(b))
            if matches!((&a.column, &b.column), (Some(ColumnReference::Temporary(a)), Some(ColumnReference::Temporary(b))) if a == b)));
    }
    #[test]
    fn missing_column_limit_fails_without_partial_output() {
        let issues: Vec<_> = (0..201)
            .map(|i| {
                let mut item = issue();
                item["id"] = i.to_string().into();
                item["fields"]["status"]["name"] = format!("Status {i}").into();
                item
            })
            .collect();
        assert_eq!(
            synchronize(&context("cloud"), &[], |_, _| Ok(response(
                json!({"issues":issues,"isLast":true})
            )))
            .unwrap_err()
            .code,
            "COLUMN_LIMIT_EXCEEDED"
        );
    }
    #[test]
    fn custom_description_template_preserves_inserted_text() {
        let mut ctx = context("cloud");
        let mut config: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap();
        config["description_template"] = "{key}: {summary}\n{description}\n{url}".into();
        ctx.config_json = config.to_string();
        let mut item = issue();
        item["fields"]["description"] = "Literal {url}".into();
        let changes = synchronize(&ctx, &columns(), |_, _| {
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(
            changes.tasks[0].description,
            "APP-1: Fix\nLiteral {url}\nhttps://jira.example.com/browse/APP-1"
        );
        let values = Placeholders {
            url: "url",
            description: "body",
            description_code: "code",
            key: "key",
            summary: "summary",
            comments: "notes",
        };
        assert_eq!(render_description("{unknown} {", &values), "{unknown} {");
        assert_eq!(render_description("", &values), "");
        assert_eq!(render_description("{comments}", &values), "notes");
    }
    #[test]
    fn comments_are_requested_only_when_the_template_uses_them() {
        for api in ["cloud", "data-center"] {
            let ctx = configured(
                api,
                json!({"description_template":"{description}\n\n{comments}"}),
            );
            let mut bodies: Vec<serde_json::Value> = vec![];
            let changes = synchronize(&ctx, &columns(), |_, body| {
                bodies.push(serde_json::from_slice(body).unwrap());
                let mut item = issue();
                item["fields"]["comment"] = if api == "cloud" {
                    json!({"comments":[
                        {"author":{"displayName":"Alice"},"created":"2026-10-10T12:00:00.000+0000",
                         "body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Looks {url} good"}]}]}},
                        {"body":{"type":"doc","content":[]}}
                    ]})
                } else {
                    json!({"comments":[
                        {"author":{"displayName":"Alice"},"created":"2026-10-10T12:00:00.000+0000","body":"Looks {url} good"},
                        {"author":{"displayName":" "},"body":""}
                    ]})
                };
                Ok(response(if api == "cloud" {
                    json!({"issues":[item],"isLast":true})
                } else {
                    json!({"issues":[item],"startAt":0,"total":1})
                }))
            })
            .unwrap();
            let mut fields = SEARCH_FIELDS.to_vec();
            fields.push(COMMENT_FIELD);
            assert_eq!(bodies[0]["fields"], json!(fields));
            assert_eq!(bodies[0]["maxResults"], COMMENT_PAGE_SIZE);
            assert_eq!(
                changes.tasks[0].description,
                "Details\n\n> Alice · 2026-10-10 12:00\n>\n> Looks {url} good\n\n> Unknown"
            );
        }
        // Self-hosted wiki markup becomes Markdown; {description_code} keeps the original.
        let ctx = configured(
            "data-center",
            json!({"description_template":"{description}\n\n{description_code}\n\n{comments}"}),
        );
        let changes = synchronize(&ctx, &columns(), |_, _| {
            let mut item = issue();
            item["fields"]["description"] = "h2. Steps\n# Open *menu*\n{code}a ``` b{code}".into();
            item["fields"]["comment"] = json!({"comments":[{"author":{"displayName":"Al"},"body":"* see [doc|https://d.io]"}]});
            Ok(response(json!({"issues":[item],"startAt":0,"total":1})))
        })
        .unwrap();
        assert_eq!(
            changes.tasks[0].description,
            "## Steps\n1. Open **menu**\n````\na ``` b\n````\n\n\
             ````\nh2. Steps\n# Open *menu*\n{code}a ``` b{code}\n````\n\n\
             > Al\n>\n> - see [doc](https://d.io)"
        );
        // Cloud ADF text is not reinterpreted as wiki markup.
        let ctx = configured(
            "cloud",
            json!({"description_template":"{description}|{description_code}"}),
        );
        let changes = synchronize(&ctx, &columns(), |_, _| {
            let mut item = issue();
            item["fields"]["description"] = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"h1. *x*"}]}]});
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(changes.tasks[0].description, "h1. *x*|```\nh1. *x*\n```");
        let ctx = configured(
            "cloud",
            json!({"description_template":"[{description_code}]"}),
        );
        let changes = synchronize(&ctx, &columns(), |_, _| {
            let mut item = issue();
            item["fields"]["description"] = serde_json::Value::Null;
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(changes.tasks[0].description, "[]");
        // Every body line is quoted, including blank lines inside the comment.
        let page: CommentPage = serde_json::from_value(json!({"comments":[
            {"author":{"displayName":"Bob"},"created":"yesterday","body":"Line 1  \n\nLine 2\n"}
        ]}))
        .unwrap();
        assert_eq!(
            comments(Some(page), true),
            "> Bob · yesterday\n>\n> Line 1\n>\n> Line 2"
        );
        // Missing or null comment fields render nothing.
        let ctx = configured("cloud", json!({"description_template":"[{comments}]"}));
        for value in [serde_json::Value::Null, json!({}), json!({"comments":null})] {
            let changes = synchronize(&ctx, &columns(), |_, _| {
                let mut item = issue();
                item["fields"]["comment"] = value.clone();
                Ok(response(json!({"issues":[item],"isLast":true})))
            })
            .unwrap();
            assert_eq!(changes.tasks[0].description, "[]");
        }
    }
    #[test]
    fn mapping_and_pagination_for_both_apis() {
        for api in ["cloud", "data-center"] {
            let mut urls = vec![];
            let mut bodies: Vec<serde_json::Value> = vec![];
            let mut ctx = context(api);
            if api == "data-center" {
                let mut config: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap();
                config["base_url"] = "https://jira.example.com/jira".into();
                ctx.config_json = config.to_string();
            }
            let changes = synchronize(&ctx, &columns(), |url, body| {
                urls.push(url.to_string());
                bodies.push(serde_json::from_slice(body).unwrap());
                let mut item = issue();
                if urls.len() == 2 {
                    item["fields"]["summary"] = "Latest".into();
                    item["fields"]["updated"] = "2026-10-10T12:01:00.000+0000".into();
                }
                Ok(response(if api == "cloud" {
                    json!({"issues":[item],"isLast":urls.len()==2,"nextPageToken":"next +"})
                } else {
                    json!({"issues":[item],"startAt":urls.len()-1,"total":2})
                }))
            })
            .unwrap();
            assert_eq!(urls.len(), 2);
            assert!(urls.iter().all(|url| url
                == if api == "cloud" {
                    "https://jira.example.com/rest/api/3/search/jql"
                } else {
                    "https://jira.example.com/jira/rest/api/2/search"
                }));
            assert_eq!(bodies[0]["maxResults"], PAGE_SIZE);
            assert_eq!(bodies[0]["fields"], json!(SEARCH_FIELDS));
            if api == "cloud" {
                assert_eq!(bodies[0]["jql"], "project = APP");
                assert!(bodies[0].get("nextPageToken").is_none());
                assert_eq!(bodies[1]["nextPageToken"], "next +");
            } else {
                assert_eq!(bodies[0]["jql"], "project = APP ORDER BY key ASC");
                assert_eq!(bodies[0]["startAt"], 0);
                assert_eq!(bodies[1]["startAt"], 1);
            }
            assert_eq!(changes.tasks.len(), 1);
            let task = &changes.tasks[0];
            assert_eq!(task.title, "APP-1 Latest");
            assert_eq!(task.column_id, 0);
            assert_eq!(task.labels, vec!["fix:1.2"]);
            assert_eq!(
                task.description,
                "https://jira.example.com/browse/APP-1\n\nDetails"
            );
        }
    }
    #[test]
    fn duplicate_columns_partial_failure_and_bad_pagination_fail_atomically() {
        let ctx = context("cloud");
        let duplicates = vec![
            Column {
                id: 0,
                name: "In Progress".into(),
            },
            Column {
                id: 1,
                name: "In Progress".into(),
            },
        ];
        assert_eq!(
            synchronize(&ctx, &duplicates, |_, _| Ok(response(
                json!({"issues":[issue()],"isLast":true})
            )))
            .unwrap_err()
            .code,
            "STATUS_COLUMN_DUPLICATED"
        );
        let mut calls = 0;
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| {
                calls += 1;
                if calls == 1 {
                    Ok(response(
                        json!({"issues":[issue()],"isLast":false,"nextPageToken":"next"}),
                    ))
                } else {
                    Err(error("HTTP_FAILED", None))
                }
            })
            .unwrap_err()
            .code,
            "HTTP_FAILED"
        );
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| Ok(response(
                json!({"issues":[],"isLast":false,"nextPageToken":"repeat"})
            )))
            .unwrap_err()
            .code,
            "PAGE_TOKEN_REPEATED"
        );
        assert_eq!(
            synchronize(&context("data-center"), &columns(), |_, _| Ok(response(
                json!({"issues":[],"startAt":0,"total":1})
            )))
            .unwrap_err()
            .code,
            "PAGE_EMPTY_BEFORE_TOTAL"
        );
    }
    #[test]
    fn limits_and_rate_limits() {
        let issues: Vec<_> = (0..2001)
            .map(|i| {
                let mut item = issue();
                item["id"] = i.to_string().into();
                item
            })
            .collect();
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| Ok(response(
                json!({"issues":issues,"isLast":true})
            )))
            .unwrap_err()
            .code,
            "TICKET_LIMIT_EXCEEDED"
        );
        let mut page = 0;
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| {
                page += 1;
                Ok(response(
                    json!({"issues":[],"isLast":false,"nextPageToken":page.to_string()}),
                ))
            })
            .unwrap_err()
            .code,
            "PAGE_LIMIT_EXCEEDED"
        );
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_, _| Ok(HttpResponse {
                status: 429,
                headers: vec![("Retry-After".into(), "120".into())],
                body: vec![]
            }))
            .unwrap_err()
            .retry_after_seconds,
            Some(120)
        );
    }
    #[test]
    fn adf_and_url_validation() {
        let mut text = String::new();
        description(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Docs","marks":[{"type":"link","attrs":{"href":"https://example.com"}}]}]}]}),
            &mut text,
        );
        assert_eq!(text, "Docs (https://example.com)\n");
        for base in [
            "http://jira.example.com",
            "https://token@jira.example.com",
            "https://jira.example.com?token=x",
            "https://jira.example.com/../evil",
            "https://localhost:8443",
        ] {
            assert_eq!(normalize_url(base), None, "{base}");
        }
        assert!(normalize_url("https://api.atlassian.com/ex/jira/cloud-id").is_some());
    }

    #[derive(Default)]
    struct MockBoard {
        active: Vec<TaskInfo>,
        archived: Vec<TaskInfo>,
        version: Option<String>,
        resource_limit: bool,
        calls: Vec<String>,
    }
    impl BoardTasks for MockBoard {
        fn list_active(&mut self, offset: u32, limit: u32) -> Result<TaskPage, HostError> {
            if self.resource_limit {
                return Err(HostError::ResourceLimit);
            }
            Ok(TaskPage {
                version: self.version.clone().unwrap_or_else(|| "snapshot".into()),
                tasks: self
                    .active
                    .iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .cloned()
                    .collect(),
                offset,
                limit,
            })
        }
        fn archived(&mut self, id: i64) -> Result<Option<TaskInfo>, HostError> {
            Ok(self.archived.iter().find(|t| t.task.id == id).cloned())
        }
        fn archive(&mut self, id: i64) -> Result<(), HostError> {
            self.calls.push(format!("archive {id}"));
            Ok(())
        }
        fn restore(&mut self, id: i64, column: &ColumnReference) -> Result<(), HostError> {
            self.calls.push(match column {
                ColumnReference::Id(c) => format!("restore {id} -> {c}"),
                ColumnReference::Temporary(k) => format!("restore {id} -> {k}"),
            });
            Ok(())
        }
    }
    fn card(id: i64, title: &str, description: &str) -> TaskInfo {
        TaskInfo {
            board_id: 1,
            column_id: Some(0),
            position: Some(0),
            archived: false,
            task: Task {
                id,
                title: title.into(),
                description: description.into(),
                pinned: false,
                color: String::new(),
                start_time: None,
                due_time: None,
                created_time: None,
                start_at_midnight: false,
                due_at_midnight: false,
                labels: vec![],
                items: vec![],
                recurrence: None,
            },
        }
    }
    fn run_with_board(
        ctx: &RunContext,
        columns: &[Column],
        issues: Vec<serde_json::Value>,
        board: &mut MockBoard,
    ) -> (Result<RunOutput, PluginError>, Vec<(String, u32)>) {
        let mut logs = vec![];
        let result = synchronize_pages(
            ctx,
            columns,
            |_, _| Ok(response(json!({"issues":issues.clone(),"isLast":true}))),
            |code, count| logs.push((code.to_string(), count)),
            |_, _| Ok(()),
            board,
        );
        (result, logs)
    }
    #[test]
    fn archives_cards_whose_tickets_left_the_query_and_restores_returning_ones() {
        let link = "https://jira.example.com/browse/";
        let mut ctx = configured("cloud", json!({"archive_missing":true}));
        ctx.cursor = Some("archived:20,21,22".into());
        let mut recurring = card(15, "APP-5 Weekly", &format!("{link}APP-5"));
        recurring.task.recurrence = Some(cardbe::plugin::types::Recurrence {
            frequency: cardbe::plugin::types::RecurrenceFrequency::Weekly,
            interval: 1,
        });
        let mut returning = card(20, "APP-1 Fix", &format!("{link}APP-1"));
        returning.archived = true;
        let mut still_gone = card(21, "APP-9 Old", &format!("{link}APP-9"));
        still_gone.archived = true;
        let mut board = MockBoard {
            active: vec![
                card(10, "APP-1 Fix", &format!("{link}APP-1\n\nDetails")),
                card(11, "APP-2 Closed", &format!("Text\n{link}APP-2")),
                card(12, "APP-3 Edited", &format!("{link}APP-30")),
                card(13, "Manual card", &format!("{link}APP-6")),
                card(
                    14,
                    "APP-4 Other site",
                    "https://other.example.com/browse/APP-4",
                ),
                recurring,
                card(16, "app-7 lowercase", &format!("{link}app-7")),
            ],
            archived: vec![returning, still_gone],
            ..Default::default()
        };
        let (result, logs) = run_with_board(&ctx, &columns(), vec![issue()], &mut board);
        let result = result.unwrap();
        assert_eq!(board.calls, vec!["restore 20 -> 0", "archive 11"]);
        assert_eq!(result.cursor.as_deref(), Some("archived:21,11"));
        assert!(logs.contains(&("TICKETS_ARCHIVED".into(), 1)));
        assert!(logs.contains(&("TICKETS_RESTORED".into(), 1)));

        // Restoring into a column created in the same run uses its temporary key.
        let mut board = MockBoard {
            archived: vec![card(20, "APP-1 Fix", &format!("{link}APP-1"))],
            ..Default::default()
        };
        board.archived[0].archived = true;
        let (result, _) = run_with_board(&ctx, &[], vec![issue()], &mut board);
        assert_eq!(board.calls, vec!["restore 20 -> status:0"]);
        // 21 and 22 are no longer archived, and 20 was restored: nothing stays tracked.
        assert_eq!(result.unwrap().cursor, None);
    }
    #[test]
    fn archiving_is_opt_in_and_skipped_for_empty_results() {
        let link = "https://jira.example.com/browse/";
        let stale = || MockBoard {
            active: vec![card(11, "APP-2 Closed", &format!("{link}APP-2"))],
            ..Default::default()
        };
        let mut ctx = context("cloud");
        ctx.cursor = Some("archived:5".into());
        let mut board = stale();
        let (result, _) = run_with_board(&ctx, &columns(), vec![issue()], &mut board);
        assert!(board.calls.is_empty());
        assert_eq!(result.unwrap().cursor.as_deref(), Some("archived:5"));

        let mut ctx = configured("cloud", json!({"archive_missing":true}));
        ctx.cursor = Some("archived:5".into());
        let mut board = stale();
        let (result, _) = run_with_board(&ctx, &columns(), vec![], &mut board);
        assert!(board.calls.is_empty());
        assert_eq!(result.unwrap().cursor.as_deref(), Some("archived:5"));

        let mut board = MockBoard {
            resource_limit: true,
            ..stale()
        };
        let (result, _) = run_with_board(&ctx, &columns(), vec![issue()], &mut board);
        assert!(result.is_ok() && board.calls.is_empty());

        let mut board = MockBoard {
            version: Some("other".into()),
            ..stale()
        };
        let (result, _) = run_with_board(&ctx, &columns(), vec![issue()], &mut board);
        assert_eq!(result.unwrap_err().code, "BOARD_SNAPSHOT_MISMATCH");
    }
    #[test]
    fn archiving_respects_the_operation_budget() {
        let link = "https://jira.example.com/browse/";
        let ctx = configured("cloud", json!({"archive_missing":true}));
        let mut board = MockBoard {
            active: (0..205)
                .map(|i| card(i, &format!("OLD-{i} x"), &format!("{link}OLD-{i}")))
                .collect(),
            ..Default::default()
        };
        // A new status column and its upsert use 2 of the 200 operations.
        let (result, logs) = run_with_board(&ctx, &[], vec![issue()], &mut board);
        assert_eq!(board.calls.len(), 198);
        assert!(logs.contains(&("ARCHIVE_DEFERRED".into(), 7)));
        let cursor = result.unwrap().cursor.unwrap();
        assert!(cursor.starts_with("archived:0,1,") && cursor.len() <= MAX_CURSOR);
        let ids: Vec<i64> = (0..2000).collect();
        let cursor = encode_cursor(&ids).unwrap();
        assert!(cursor.len() <= MAX_CURSOR && cursor.ends_with(",1999"));
        assert_eq!(encode_cursor(&[]), None);
    }
    #[test]
    fn assignee_creator_and_labels_become_tags() {
        let changes = synchronize(&context("cloud"), &columns(), |_, _| {
            let mut item = issue();
            item["fields"]["assignee"] = json!({"displayName":" Alice "});
            item["fields"]["creator"] = json!({"displayName":"Bob"});
            item["fields"]["labels"] = json!(["backend", "1.2", " "]);
            item["fields"]["versions"] = json!([{"name":"1.1"}, {"name":" "}]);
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(
            changes.tasks[0].labels,
            vec![
                "assignee:Alice",
                "creator:Bob",
                "1.2",
                "affects:1.1",
                "backend",
                "fix:1.2"
            ]
        );
        let changes = synchronize(&context("cloud"), &columns(), |_, _| {
            let mut item = issue();
            item["fields"]["assignee"] = serde_json::Value::Null;
            item["fields"]["creator"] = json!({});
            item["fields"]["labels"] = serde_json::Value::Null;
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(changes.tasks[0].labels, vec!["fix:1.2"]);
    }
}
