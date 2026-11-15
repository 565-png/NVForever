//! Bounded summaries of actual headless trace events, not inferred script success.
//! Inspired by h9x000 / Claude Fable 5.2, ffc6b6fb62484b1089f08dddf328681b61aab165.

use fnv_game::trace::{Event, EventKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const MAX_ENTRIES: usize = 4096;
const MAX_WORD_BYTES: usize = 8 * 1024 * 1024;

pub fn summarize(events: &[Event], detail: bool) -> Result<Value, String> {
    let mut rows: BTreeMap<(&str, &str, String), usize> = BTreeMap::new();
    let mut retained = 0;
    for e in events {
        let (kind, unit, words) = match &e.kind {
            EventKind::CallFailed { name, unit, error, .. } => ("call_failed", unit, format!("{name}: {error}")),
            EventKind::ScriptError { unit, error } => ("script_error", unit, error.clone()),
            EventKind::Skipped { unit, what } => ("skipped", unit, what.clone()),
            EventKind::Unhandled { name, unit, .. } => ("unhandled", unit, name.clone()),
            EventKind::ArgUnread { name, unit, param, wanted, got, .. } => ("argument_unread", unit, format!("{name} parameter {param}: {got}, wanted {wanted}")),
            EventKind::ScriptPrint { unit, text } => ("printed", unit, text.clone()),
            _ => continue,
        };
        let key = (kind, unit.as_str(), words);
        if !rows.contains_key(&key) {
            retained += key.1.len() + key.2.len();
            if rows.len() == MAX_ENTRIES || retained > MAX_WORD_BYTES {
                return Err("runner diagnostic retention limit exceeded; no partial summary".into());
            }
        }
        *rows.entry(key).or_default() += 1;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let entries: Vec<Value> = rows.into_iter().enumerate().map(|(id, ((kind, unit, words), count))| {
        *counts.entry(kind).or_default() += count;
        let mut row = json!({"id": id, "kind": kind, "trace_events": count});
        if detail {
            row["unit"] = json!(unit);
            row["words"] = json!(words);
        }
        row
    }).collect();
    Ok(json!({"schema": "fnv.ext.runner_diagnostics/1", "detail": detail, "counts": counts, "entries": entries,
        "scope": "Recorded trace events only; some runtime diagnostics are deduplicated before tracing. No rendered/native/mod acceptance."}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_words_and_units_require_detail_and_counts_are_events() {
        let event = Event { frame: 0, kind: EventKind::CallFailed { opcode: "x".into(), name: "F".into(), unit: "private/unit".into(), error: "private words".into() } };
        let events = [event.clone(), event];
        let safe = summarize(&events, false).unwrap();
        assert_eq!(safe["counts"]["call_failed"], 2);
        assert!(!safe.to_string().contains("private"));
        let raw = summarize(&events, true).unwrap();
        assert_eq!(raw["entries"][0]["unit"], "private/unit");
        assert_eq!(raw["entries"][0]["words"], "F: private words");
    }

    #[test]
    fn retention_boundary_refuses_instead_of_hiding_new_failures() {
        let events: Vec<Event> = (0..=MAX_ENTRIES).map(|i| Event { frame: 0, kind: EventKind::ScriptPrint { unit: "authored".into(), text: i.to_string() } }).collect();
        assert!(summarize(&events[..MAX_ENTRIES], false).is_ok());
        assert!(summarize(&events, false).unwrap_err().contains("retention limit"));
    }

    #[test]
    fn retained_word_bytes_are_capped_even_for_one_entry() {
        let mut event = Event { frame: 0, kind: EventKind::ScriptPrint { unit: String::new(), text: "x".repeat(MAX_WORD_BYTES) } };
        assert!(summarize(std::slice::from_ref(&event), false).is_ok());
        if let EventKind::ScriptPrint { text, .. } = &mut event.kind { text.push('x'); }
        assert!(summarize(&[event], false).unwrap_err().contains("retention limit"));
    }
}
