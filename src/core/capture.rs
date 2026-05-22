use serde_json::json;

use super::model::*;
use super::store::*;
use super::RumbProject;

const CAPTURE_TITLE_MAX_CHARS: usize = 120;

impl RumbProject {
    /// Drop a half-formed thought into the inbox as a draft `note` — no "what kind
    /// of thing is this?" freeze at intake. The full text is preserved in `body`;
    /// the title is a clean one-line summary. Recorded as an undoable changeset so
    /// it can be undone or groomed out of the inbox later.
    pub fn capture(&self, input: Capture) -> Result<Item, RumbError> {
        if input.text.trim().is_empty() {
            return Err(RumbError::EmptyTitle);
        }

        self.mutate(|m| {
            let inbox = inbox_id(m.conn())?.ok_or(RumbError::MissingInbox)?;
            let now = timestamp();
            let item = Item {
                id: next_item_id(m.conn())?,
                parent_id: Some(inbox),
                kind: "note".to_owned(),
                title: normalize_capture_title(&input.text),
                status: Status::Draft,
                tier: Tier::Standard,
                source_ref: None,
                body: Some(input.text.clone()),
                created_at: now,
                updated_at: now,
            };
            m.insert_item(&item)?;
            m.event(
                "item.capture",
                "item",
                &item.id,
                json!({
                    "kind": &item.kind,
                    "status": item.status.to_string(),
                    "tier": item.tier.to_string(),
                    "title": &item.title,
                })
                .to_string(),
                now,
            );
            m.mark_undoable();
            Ok(item)
        })
    }
}

/// Collapse a raw capture into a single clean title line: tabs/newlines and runs
/// of whitespace become single spaces, the result is trimmed and truncated on a
/// char boundary (`chars().take` — byte slicing would panic mid-codepoint).
fn normalize_capture_title(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(CAPTURE_TITLE_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize_capture_title;

    #[test]
    fn collapses_whitespace_to_single_line() {
        assert_eq!(
            normalize_capture_title("  hello\n\tworld   again  "),
            "hello world again"
        );
    }

    #[test]
    fn truncates_on_char_boundary() {
        let long = "é".repeat(200);
        let title = normalize_capture_title(&long);
        assert_eq!(title.chars().count(), 120);
    }
}
