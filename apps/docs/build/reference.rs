//! Structured section content: API entries, callouts, term lists and type maps.
//! Emits typed static records like the rest of the build: prose goes through
//! `prose`, signatures through the build-time highlighter, and nothing is HTML.
use crate::{highlight::Highlighter, prose};
use serde_json::Value;
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Fields {
    /// `api`, `callouts`, `terms`, `map` and `search` field initializers.
    pub source: String,
    /// Member anchors, which share the page's id namespace with sections.
    pub anchors: Vec<String>,
}

const KINDS: &[&str] = &[
    "struct",
    "enum",
    "function",
    "trait",
    "type alias",
    "generated type",
];

pub fn section(section: &Value, ids: &BTreeSet<&str>, highlighter: &Highlighter) -> Result<Fields> {
    let id = section["id"].as_str().unwrap_or("");
    let mut search = Vec::new();
    let mut anchors = Vec::new();
    let api = match &section["api"] {
        Value::Null => "&[]".to_owned(),
        api => format!(
            "&[{}]",
            entry(api, id, highlighter, &mut search, &mut anchors)?
        ),
    };
    let mut callouts = String::from("&[");
    for (index, callout) in list(section, "callouts")?.iter().enumerate() {
        let kind = callout["kind"].as_str().unwrap_or("note");
        let label = match kind {
            "tip" => "Good to know",
            "warning" => "Watch out",
            "note" => "Note",
            _ => return Err(format!("{id}: callout kind must be tip, warning or note").into()),
        };
        let body = required(callout, "body", id)?;
        search.push(body.to_owned());
        callouts.push_str(&format!(
            "CalloutData {{ id: {index}, kind: {kind:?}, label: {label:?}, body: {} }},",
            prose::compile(body)?
        ));
    }
    callouts.push(']');
    let terms = terms(list(section, "terms")?, id, &mut search)?;
    let mut map = String::from("&[");
    for (index, group) in list(section, "map")?.iter().enumerate() {
        let side = side(group, id)?.0;
        map.push_str(&format!(
            "MapGroupData {{ id: {index}, title: {:?}, side: {side:?}, cards: &[",
            required(group, "title", id)?
        ));
        for card in list(group, "cards")? {
            let href = required(card, "href", id)?;
            let target = href.strip_prefix('#');
            if target.is_some_and(|target| !ids.contains(target))
                || target.is_none() && !href.starts_with("/docs/")
            {
                return Err(format!("{id}: map card links to an unknown section: {href}").into());
            }
            let (name, text) = (required(card, "name", id)?, required(card, "text", id)?);
            search.extend([name.to_owned(), text.to_owned()]);
            map.push_str(&format!(
                "MapCardData {{ href: {href:?}, name: {name:?}, kind: {:?}, text: {text:?} }},",
                card["kind"].as_str().unwrap_or("")
            ));
        }
        map.push_str("] },");
    }
    map.push(']');
    Ok(Fields {
        source: format!(
            "api: {api}, callouts: {callouts}, terms: {terms}, map: {map}, search: {:?}",
            search.join("\n")
        ),
        anchors,
    })
}

fn entry(
    api: &Value,
    id: &str,
    highlighter: &Highlighter,
    search: &mut Vec<String>,
    anchors: &mut Vec<String>,
) -> Result<String> {
    let kind = required(api, "kind", id)?;
    if !KINDS.contains(&kind) {
        return Err(format!("{id}: api kind must be one of {KINDS:?}").into());
    }
    let (side, side_label) = side(api, id)?;
    let from = api["from"].as_str().unwrap_or("");
    search.push(from.to_owned());
    let mut source = format!(
        "ApiData {{ kind: {kind:?}, side: {side:?}, side_label: {side_label:?}, from: {}, params: {}, groups: &[",
        prose::compile(from)?,
        terms(list(api, "params")?, id, search)?
    );
    for (index, group) in list(api, "groups")?.iter().enumerate() {
        source.push_str(&format!(
            "MemberGroupData {{ id: {index}, title: {:?}, members: &[",
            group["title"].as_str().unwrap_or("")
        ));
        for member in list(group, "members")? {
            let name = required(member, "name", id)?;
            let anchor = anchor(id, name)?;
            if anchors.contains(&anchor) {
                return Err(format!("duplicate member anchor: {anchor}").into());
            }
            let text = required(member, "text", id)?;
            let details = member["details"].as_str().unwrap_or("");
            let signature = member["signature"].as_str().unwrap_or("");
            search.extend([name.to_owned(), text.to_owned(), details.to_owned()]);
            let details_label = match (signature.is_empty(), details.is_empty()) {
                (false, false) => "Signature and details",
                (false, true) => "Signature",
                _ => "Details",
            };
            source.push_str(&format!(
                "MemberData {{ anchor: {anchor:?}, name: {name:?}, returns: {:?}, text: {}, signature: {}, details: {}, details_label: {details_label:?} }},",
                member["returns"].as_str().unwrap_or(""),
                prose::compile(text)?,
                highlighter.tokens(signature, "Rust")?,
                prose::compile(details)?,
            ));
            anchors.push(anchor);
        }
        source.push_str("] },");
    }
    source.push_str(&format!(
        "], declaration: {} }}",
        highlighter.tokens(api["declaration"].as_str().unwrap_or(""), "Rust")?
    ));
    Ok(source)
}

fn terms(terms: &[Value], id: &str, search: &mut Vec<String>) -> Result<String> {
    let mut source = String::from("&[");
    for (index, term) in terms.iter().enumerate() {
        let (name, text) = (required(term, "term", id)?, required(term, "text", id)?);
        search.extend([name.to_owned(), text.to_owned()]);
        source.push_str(&format!(
            "TermData {{ id: {index}, term: {}, text: {} }},",
            prose::inline(name)?,
            prose::compile(text)?
        ));
    }
    source.push(']');
    Ok(source)
}

/// `{section}-{member}`: `.on_progress(callback)` in `job` is `job-on-progress`,
/// and `ctx.share(value)` in `shared` is `shared-share`.
fn anchor(section: &str, name: &str) -> Result<String> {
    let path = name.split(['(', '<', '{', ':', ' ']).next().unwrap_or("");
    let ident = path.rsplit('.').find(|part| !part.is_empty()).unwrap_or("");
    if ident.is_empty() || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{section}: member name needs an identifier: {name}").into());
    }
    Ok(format!(
        "{section}-{}",
        ident.to_lowercase().replace('_', "-")
    ))
}

fn side(value: &Value, id: &str) -> Result<(&'static str, &'static str)> {
    match value["side"].as_str() {
        Some("page") => Ok(("page", "Page code")),
        Some("worker") => Ok(("worker", "Worker code")),
        Some("both") => Ok(("both", "Page and worker code")),
        _ => Err(format!("{id}: side must be page, worker or both").into()),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    match &value[key] {
        Value::Null => Ok(&[]),
        Value::Array(items) => Ok(items),
        _ => Err(format!("{key} must be an array").into()),
    }
}

fn required<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{id}: {key} is required").into())
}
