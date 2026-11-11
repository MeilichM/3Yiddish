use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub aliases: Vec<String>,
    pub context: String,
    pub scope: String,
    pub source: String,
    pub review_status: String,
}
pub fn normalize(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .replace('װ', "וו")
        .replace('ײ', "יי")
        .replace('ױ', "וי")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn load(path: &str) -> Result<Vec<Entry>> {
    let entries: Vec<Entry> =
        serde_json::from_slice(&std::fs::read(path).context("Cannot read knowledge file")?)?;
    let mut ids = std::collections::HashSet::new();
    for e in &entries {
        if e.id.is_empty()
            || !ids.insert(&e.id)
            || e.source.is_empty()
            || e.scope.is_empty()
            || e.context.is_empty()
            || e.aliases.is_empty()
            || e.aliases.iter().any(|a| normalize(a).is_empty())
        {
            bail!(
                "Knowledge entries require unique IDs, nonempty aliases, context, source and scope"
            );
        }
        if !matches!(e.review_status.as_str(), "draft" | "reviewed") {
            bail!("Unknown knowledge review status");
        }
    }
    Ok(entries)
}
pub fn search(entries: &[Entry], query: &str) -> Vec<Entry> {
    let query = format!(" {} ", normalize(query));
    let mut hits: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let score = entry
                .aliases
                .iter()
                .map(|a| normalize(a))
                .filter(|a| query.contains(&format!(" {a} ")))
                .map(|a| a.len())
                .max()?;
            Some((score, entry))
        })
        .collect();
    hits.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then(a.id.cmp(&b.id)));
    hits.into_iter().take(5).map(|(_, e)| e.clone()).collect()
}
