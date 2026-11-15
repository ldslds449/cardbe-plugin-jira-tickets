//! Converts common Jira wiki markup (Server/Data Center text fields) to Markdown.
//!
//! Supported: `h1.`–`h6.` headings, `*`/`#`/`-` lists (nested), `bq.` and `{quote}`,
//! `{code}`/`{noformat}` blocks, `||header||` and `|cell|` tables, `----`, `*bold*`,
//! `{{monospace}}`, `[text|url]`, `[url]`, `[~user]`, and `{color}` tags (removed).
//! Anything else is kept as written.

/// Bounds how far inline markup searches for a closing token, keeping conversion linear.
const INLINE_WINDOW: usize = 4096;

pub fn to_markdown(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut quote = false;
    let mut rest = text;
    let mut after_code = false;
    loop {
        let block = find_block(rest);
        let (segment, next) = match &block {
            Some(found) => (&rest[..found.start], Some(found)),
            None => (rest, None),
        };
        let segment = if after_code {
            segment
                .strip_prefix("\r\n")
                .or_else(|| segment.strip_prefix('\n'))
                .unwrap_or(segment)
        } else {
            segment
        };
        if !(after_code && next.is_some() && segment.trim().is_empty()) {
            for line in segment.lines() {
                convert_line(line, &mut quote, &mut out);
            }
        }
        let Some(found) = next else { break };
        let after = &rest[found.open_end..];
        let close = format!("{{{}}}", found.name);
        let (body, remaining) = match after.find(&close) {
            Some(i) => (&after[..i], &after[i + close.len()..]),
            None => (after, ""),
        };
        let body: Vec<_> = body.trim_matches(['\r', '\n']).lines().collect();
        out.push(code_block(&body.join("\n"), &found.language));
        rest = remaining;
        after_code = true;
    }
    out.join("\n")
}

/// Wraps `text` in a fenced code block whose fence is longer than any backtick run inside.
pub fn code_block(text: &str, language: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for b in text.bytes() {
        run = if b == b'`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}{language}\n{text}\n{fence}")
}

struct Block {
    start: usize,
    open_end: usize,
    name: &'static str,
    language: String,
}

fn find_block(text: &str) -> Option<Block> {
    let mut best: Option<Block> = None;
    for name in ["code", "noformat"] {
        let open = format!("{{{name}");
        let mut from = 0;
        while let Some(i) = text[from..].find(&open) {
            let start = from + i;
            let after = &text[start + open.len()..];
            let Some(end) = after.find('}') else { break };
            let params = &after[..end];
            if params.is_empty() || params.starts_with(':') {
                if best.as_ref().is_none_or(|b| start < b.start) {
                    best = Some(Block {
                        start,
                        open_end: start + open.len() + end + 1,
                        name,
                        language: if name == "code" {
                            language(params.trim_start_matches(':'))
                        } else {
                            String::new()
                        },
                    });
                }
                break;
            }
            from = start + open.len();
        }
    }
    best
}

/// `{code:java}` or `{code:language=java|title=x}`; returns a safe fence info string.
fn language(params: &str) -> String {
    let value = params
        .split('|')
        .find_map(|p| match p.split_once('=') {
            Some((key, value)) if key.trim() == "language" => Some(value),
            Some(_) => None,
            None => Some(p),
        })
        .unwrap_or("")
        .trim();
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'#' | b'_' | b'.'))
    {
        value.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn convert_line(line: &str, quote: &mut bool, out: &mut Vec<String>) {
    // `{quote}` toggles a quoted region and may appear mid-line.
    let pieces: Vec<_> = line.split("{quote}").collect();
    let has_marker = pieces.len() > 1;
    for (i, piece) in pieces.into_iter().enumerate() {
        if i > 0 {
            *quote = !*quote;
            if !*quote {
                // A blank line ends the quote; otherwise Markdown continues it lazily.
                out.push(String::new());
            }
        }
        // Text around a marker is emitted only when present; the marker itself adds no line.
        if has_marker && piece.trim().is_empty() {
            continue;
        }
        let converted = convert_text_line(piece);
        if converted.is_empty() {
            out.push(if *quote { ">".into() } else { String::new() });
        }
        for text in converted.lines() {
            out.push(match (*quote, text.is_empty()) {
                (true, true) => ">".into(),
                (true, false) => format!("> {text}"),
                (false, _) => text.into(),
            });
        }
    }
}

fn convert_text_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed == "----" {
        return "---".into();
    }
    let start = line.trim_start();
    if let Some((level, text)) = heading(start) {
        return format!("{} {}", "#".repeat(level), inline(text.trim()));
    }
    if let Some(text) = start.strip_prefix("bq. ") {
        return format!("> {}", inline(text.trim()));
    }
    if let Some((depth, numbered, text)) = list_item(start) {
        return format!(
            "{}{} {}",
            "    ".repeat(depth - 1),
            if numbered { "1." } else { "-" },
            inline(text.trim())
        );
    }
    if let Some(cells) = trimmed.strip_prefix("||") {
        let cells: Vec<_> = cells
            .trim_end_matches('|')
            .split("||")
            .map(|c| inline(c.trim()))
            .collect();
        return format!(
            "| {} |\n|{}|",
            cells.join(" | "),
            vec![" --- "; cells.len()].join("|")
        );
    }
    if trimmed.len() > 1 && trimmed.starts_with('|') {
        // Links are converted first so their `|` separators do not split cells.
        let row = inline(trimmed);
        let cells: Vec<_> = row
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        return format!("| {} |", cells.join(" | "));
    }
    inline(line.trim_end())
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let rest = line.strip_prefix('h')?;
    let level = rest.bytes().next().filter(|b| (b'1'..=b'6').contains(b))? - b'0';
    let text = rest[1..].strip_prefix(". ")?;
    Some((level as usize, text))
}

fn list_item(line: &str) -> Option<(usize, bool, &str)> {
    let marker = line
        .bytes()
        .take_while(|b| matches!(b, b'*' | b'#'))
        .count();
    if marker > 0 {
        let text = line[marker..].strip_prefix(' ')?;
        return Some((marker, line.as_bytes()[marker - 1] == b'#', text));
    }
    line.strip_prefix("- ").map(|text| (1, false, text))
}

/// Returns the longest prefix of at most `max` bytes ending on a character boundary.
fn window(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let near = window(rest, INLINE_WINDOW);
        if let Some(code) = near.strip_prefix("{{") {
            if let Some(end) = code.find("}}").filter(|&end| end > 0) {
                out.push_str(&code_span(&code[..end]));
                i += end + 4;
                continue;
            }
        }
        if near.starts_with("{color") {
            if let Some(end) = near.find('}') {
                let tag = &near[..=end];
                if tag == "{color}" || tag.starts_with("{color:") {
                    i += end + 1;
                    continue;
                }
            }
        }
        if near.starts_with('[') {
            if let Some(end) = near.find(']') {
                if let Some(link) = link(&near[1..end]) {
                    out.push_str(&link);
                    i += end + 1;
                    continue;
                }
            }
        }
        if near.starts_with('*')
            && text[..i]
                .chars()
                .last()
                .is_none_or(|c| !c.is_alphanumeric())
        {
            if let Some(end) = near[1..].find('*') {
                let content = &near[1..1 + end];
                let after = &rest[2 + end..];
                if !content.is_empty()
                    && !content.starts_with(' ')
                    && !content.ends_with(' ')
                    && after.chars().next().is_none_or(|c| !c.is_alphanumeric())
                {
                    out.push_str("**");
                    out.push_str(&inline(content));
                    out.push_str("**");
                    i += end + 2;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().expect("non-empty rest");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn code_span(code: &str) -> String {
    if code.contains('`') {
        format!("`` {code} ``")
    } else {
        format!("`{code}`")
    }
}

fn is_url(value: &str) -> bool {
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| value.starts_with(scheme))
        && !value.contains(char::is_whitespace)
}

fn link(inner: &str) -> Option<String> {
    if let Some(user) = inner.strip_prefix('~').filter(|u| !u.is_empty()) {
        return Some(format!("@{user}"));
    }
    match inner.split_once('|') {
        Some((label, url)) if is_url(url.trim()) => {
            let label = label.trim();
            let url = url.trim();
            Some(if label.is_empty() {
                format!("<{url}>")
            } else {
                format!("[{label}]({url})")
            })
        }
        None if is_url(inner.trim()) => Some(format!("<{}>", inner.trim())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_block_markup() {
        assert_eq!(
            to_markdown("h1. Title\nh3. Sub\n* a\n** b\n# one\n## two\n- dash\n----\nbq. said"),
            "# Title\n### Sub\n- a\n    - b\n1. one\n    1. two\n- dash\n---\n> said"
        );
        assert_eq!(
            to_markdown("||A||B||\n|1|[x|https://e.com/a]|"),
            "| A | B |\n| --- | --- |\n| 1 | [x](https://e.com/a) |"
        );
        assert_eq!(
            to_markdown("before\n{quote}\nline 1\n\nline 2\n{quote}\nafter"),
            "before\n> line 1\n>\n> line 2\n\nafter"
        );
    }

    #[test]
    fn converts_code_blocks_without_touching_their_content() {
        assert_eq!(
            to_markdown(
                "Run:\n{code:language=Rust|title=x}\nlet *a* = [b|https://x.y];\n{code}\nDone"
            ),
            "Run:\n```rust\nlet *a* = [b|https://x.y];\n```\nDone"
        );
        assert_eq!(
            to_markdown("{noformat}\n```\nh1. raw\n{noformat}"),
            "````\n```\nh1. raw\n````"
        );
        assert_eq!(to_markdown("{code}unclosed"), "```\nunclosed\n```");
        assert_eq!(to_markdown("{codec} stays"), "{codec} stays");
    }

    #[test]
    fn converts_inline_markup_conservatively() {
        assert_eq!(
            inline("*bold* and {{mono}} [Docs|https://d.io] [https://x.io] [~jdoe] {color:red}red{color}"),
            "**bold** and `mono` [Docs](https://d.io) <https://x.io> @jdoe red"
        );
        assert_eq!(
            inline("2*3*4 a* b* [APP-1] [x|APP-2] {{}}"),
            "2*3*4 a* b* [APP-1] [x|APP-2] {{}}"
        );
        assert_eq!(inline("{{a`b}}"), "`` a`b ``");
        assert_eq!(to_markdown("plain\n\ntext"), "plain\n\ntext");
        assert_eq!(code_block("x", ""), "```\nx\n```");
    }

    #[test]
    fn long_unclosed_markup_stays_literal() {
        let text = "[".repeat(50_000) + &"*".repeat(50_000) + "界";
        assert_eq!(to_markdown(&text), text);
    }
}
