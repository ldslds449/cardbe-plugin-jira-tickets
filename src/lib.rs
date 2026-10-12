#![allow(clippy::too_many_arguments)] // Generated WIT ABI.
wit_bindgen::generate!({ path: "cardbe-plugin-api/wit", world: "plugin" });

use cardbe::plugin::types::{
    Changeset, ColumnReference, CreateColumn, ExternalTask, HttpResponse, Operation,
    UpsertExternalTask,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

struct Jira;
export!(Jira);

#[derive(Deserialize)]
struct Config {
    base_url: String,
    site_url: String,
    jql: String,
    api: String,
    #[serde(default = "default_description_template")]
    description_template: String,
}
fn default_description_template() -> String {
    "{url}\n\n{description}".into()
}
fn render_description(template: &str, url: &str, body: &str, key: &str, summary: &str) -> String {
    let mut out = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        out.push_str(match &remaining[..=end] {
            "{url}" => url,
            "{description}" => body,
            "{key}" => key,
            "{summary}" => summary,
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
    #[serde(rename = "fixVersions")]
    fix_versions: Vec<Named>,
    updated: String,
}
#[derive(Deserialize)]
struct Named {
    name: String,
}

impl Guest for Jira {
    fn run(context: RunContext) -> Result<RunOutput, PluginError> {
        let snapshot =
            cardbe::plugin::board::list_columns().map_err(|_| error("BOARD_QUERY_FAILED", None))?;
        if snapshot.version != context.board_version {
            return Err(error("BOARD_QUERY_FAILED", None));
        }
        let columns: Vec<_> = snapshot
            .columns
            .into_iter()
            .map(|c| Column {
                id: c.id,
                name: c.name,
            })
            .collect();
        let changes = synchronize(&context, &columns, |url| {
            cardbe::plugin::host::http_get(url).map_err(|_| error("HTTP_FAILED", None))
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
        cardbe::plugin::host::log("TICKETS_SYNCED", (changes.tasks.len() + upserts) as u32);
        cardbe::plugin::host::log("COLUMNS_CREATED", created as u32);
        Ok(changes)
    }
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
fn valid_base(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    !host.is_empty()
        && host.contains('.')
        && host.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && path.split('/').all(|part| {
            !matches!(part, "." | "..")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
        })
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
    mut get: impl FnMut(&str) -> Result<HttpResponse, PluginError>,
) -> Result<RunOutput, PluginError> {
    let config: Config =
        serde_json::from_str(&context.config_json).map_err(|_| error("INVALID_CONFIG", None))?;
    let base = config.base_url.trim_end_matches('/');
    let site = config.site_url.trim_end_matches('/');
    if !valid_base(base)
        || !valid_base(site)
        || config.jql.trim().is_empty()
        || !matches!(config.api.as_str(), "cloud" | "data-center")
    {
        return Err(error("INVALID_CONFIG", None));
    }
    let mut tasks: BTreeMap<String, ExternalTask> = BTreeMap::new();
    let mut start = 0;
    let mut token: Option<String> = None;
    let mut tokens = BTreeSet::new();
    for _ in 0..100 {
        let endpoint = if config.api == "cloud" {
            "3/search/jql"
        } else {
            "2/search"
        };
        let mut url = format!("{base}/rest/api/{endpoint}?jql={}&fields=summary,description,status,fixVersions,updated&maxResults=100", encode(&config.jql));
        if config.api == "cloud" {
            if let Some(token) = &token {
                url.push_str(&format!("&nextPageToken={}", encode(token)));
            }
        } else {
            url.push_str(&format!("&startAt={start}"));
        }
        let response = get(&url)?;
        match response.status {
            200 => {}
            400 => return Err(error("INVALID_JQL", None)),
            401 => return Err(error("AUTHENTICATION_FAILED", None)),
            403 => return Err(error("PERMISSION_DENIED", None)),
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
            _ => return Err(error("API_FAILED", Some(60))),
        }
        let page: Page =
            serde_json::from_slice(&response.body).map_err(|_| error("INVALID_RESPONSE", None))?;
        let count = page.issues.len();
        for issue in page.issues {
            if issue.id.is_empty()
                || issue.key.is_empty()
                || issue.fields.summary.trim().is_empty()
                || issue.fields.status.name.trim().is_empty()
                || issue.fields.updated.is_empty()
            {
                return Err(error("INVALID_RESPONSE", None));
            }
            let link = format!("{site}/browse/{}", encode(&issue.key));
            let mut body = String::new();
            if let Some(value) = issue.fields.description {
                description(&value, &mut body);
            }
            let task = ExternalTask {
                external_key: format!("jira:{site}:{}", issue.id),
                title: format!("{} {}", issue.key, issue.fields.summary),
                description: render_description(
                    &config.description_template,
                    &link,
                    body.trim(),
                    &issue.key,
                    &issue.fields.summary,
                ),
                url: link,
                state: issue.fields.status.name,
                labels: issue
                    .fields
                    .fix_versions
                    .into_iter()
                    .map(|v| v.name)
                    .filter(|v| !v.trim().is_empty())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                updated_at: issue.fields.updated,
                column_id: 0,
            };
            if tasks
                .get(&task.external_key)
                .is_none_or(|old| task.updated_at >= old.updated_at)
            {
                tasks.insert(task.external_key.clone(), task);
            }
            if tasks.len() > 2000 {
                return Err(error("CHANGESET_TOO_LARGE", None));
            }
        }
        if config.api == "cloud" {
            if page.is_last == Some(true) {
                return output(context, columns, tasks);
            }
            let next = page
                .next_page_token
                .filter(|v| !v.is_empty())
                .ok_or_else(|| error("INVALID_RESPONSE", None))?;
            if !tokens.insert(next.clone()) {
                return Err(error("INVALID_RESPONSE", None));
            }
            token = Some(next);
        } else {
            let total = page.total.ok_or_else(|| error("INVALID_RESPONSE", None))?;
            if page.start_at != Some(start) {
                return Err(error("INVALID_RESPONSE", None));
            }
            start += count;
            if start >= total {
                return output(context, columns, tasks);
            }
            if count == 0 {
                return Err(error("INVALID_RESPONSE", None));
            }
        }
    }
    Err(error("PAGE_LIMIT_EXCEEDED", None))
}

fn output(
    context: &RunContext,
    columns: &[Column],
    tasks: BTreeMap<String, ExternalTask>,
) -> Result<RunOutput, PluginError> {
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
                        return Err(error("CHANGESET_TOO_LARGE", None));
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
                _ => return Err(error("STATUS_COLUMN_NOT_FOUND", None)),
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
    Ok(RunOutput {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    fn creates_missing_columns_once_and_reuses_existing_columns() {
        let mut second = issue();
        second["id"] = "2".into();
        let mut third = issue();
        third["id"] = "3".into();
        third["fields"]["status"]["name"] = "Done".into();
        let changes = synchronize(&context("cloud"), &columns(), |_| {
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
        let changes = synchronize(&context("cloud"), &[], |_| {
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
            synchronize(&context("cloud"), &[], |_| Ok(response(
                json!({"issues":issues,"isLast":true})
            )))
            .unwrap_err()
            .code,
            "CHANGESET_TOO_LARGE"
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
        let changes = synchronize(&ctx, &columns(), |_| {
            Ok(response(json!({"issues":[item],"isLast":true})))
        })
        .unwrap();
        assert_eq!(
            changes.tasks[0].description,
            "APP-1: Fix\nLiteral {url}\nhttps://jira.example.com/browse/APP-1"
        );
        assert_eq!(
            render_description("{unknown} {", "", "", "", ""),
            "{unknown} {"
        );
        assert_eq!(render_description("", "url", "body", "key", "summary"), "");
    }
    #[test]
    fn mapping_and_pagination_for_both_apis() {
        for api in ["cloud", "data-center"] {
            let mut urls = vec![];
            let mut ctx = context(api);
            if api == "data-center" {
                let mut config: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap();
                config["base_url"] = "https://jira.example.com/jira".into();
                ctx.config_json = config.to_string();
            }
            let changes = synchronize(&ctx, &columns(), |url| {
                urls.push(url.to_string());
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
            assert!(urls.iter().all(|url| url.starts_with(if api == "cloud" {
                "https://jira.example.com/rest/api/3/search/jql?"
            } else {
                "https://jira.example.com/jira/rest/api/2/search?"
            })));
            assert!(urls[0].contains("jql=project%20%3D%20APP"));
            assert!(urls[1].contains(if api == "cloud" {
                "nextPageToken=next%20%2B"
            } else {
                "startAt=1"
            }));
            assert_eq!(changes.tasks.len(), 1);
            let task = &changes.tasks[0];
            assert_eq!(task.title, "APP-1 Latest");
            assert_eq!(task.column_id, 0);
            assert_eq!(task.labels, vec!["1.2"]);
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
            synchronize(&ctx, &duplicates, |_| Ok(response(
                json!({"issues":[issue()],"isLast":true})
            )))
            .unwrap_err()
            .code,
            "STATUS_COLUMN_NOT_FOUND"
        );
        let mut calls = 0;
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_| {
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
            synchronize(&context("cloud"), &columns(), |_| Ok(response(
                json!({"issues":[],"isLast":false,"nextPageToken":"repeat"})
            )))
            .unwrap_err()
            .code,
            "INVALID_RESPONSE"
        );
        assert_eq!(
            synchronize(&context("data-center"), &columns(), |_| Ok(response(
                json!({"issues":[],"startAt":0,"total":1})
            )))
            .unwrap_err()
            .code,
            "INVALID_RESPONSE"
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
            synchronize(&context("cloud"), &columns(), |_| Ok(response(
                json!({"issues":issues,"isLast":true})
            )))
            .unwrap_err()
            .code,
            "CHANGESET_TOO_LARGE"
        );
        let mut page = 0;
        assert_eq!(
            synchronize(&context("cloud"), &columns(), |_| {
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
            synchronize(&context("cloud"), &columns(), |_| Ok(HttpResponse {
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
            "https://jira.example.com:444",
        ] {
            assert!(!valid_base(base));
        }
        assert!(valid_base("https://api.atlassian.com/ex/jira/cloud-id"));
    }
}
