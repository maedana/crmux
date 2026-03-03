pub fn cwd_to_project_dir(cwd: &str) -> String {
    cwd.trim_end_matches('/').replace(['/', '.'], "-")
}

pub fn parse_summary_from_index(json: &str, session_id: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
    let sessions = parsed.as_array()?;
    for entry in sessions {
        if entry.get("sessionId")?.as_str()? == session_id {
            return entry.get("summary")?.as_str().map(ToString::to_string);
        }
    }
    None
}

pub fn read_plan_title(plans_dir: &std::path::Path, slug: &str) -> Option<String> {
    let path = plans_dir.join(format!("{slug}.md"));
    let content = std::fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    Some(first_line.strip_prefix("# ").unwrap_or(first_line).to_string())
}

pub fn extract_slug_from_jsonl(reader: impl std::io::BufRead) -> Option<String> {
    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(&line).ok()?;
        if let Some(slug) = parsed.get("slug").and_then(|s| s.as_str()) {
            return Some(slug.to_string());
        }
    }
    None
}

pub fn extract_last_prompt_from_jsonl(file: &std::fs::File) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    const TAIL_SIZE: u64 = 64 * 1024;
    let mut file = file;
    let file_len = file.metadata().ok()?.len();
    let start = file_len.saturating_sub(TAIL_SIZE);
    file.seek(SeekFrom::Start(start)).ok()?;

    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;

    for line in buf.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_user = parsed
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t == "user");
        if !is_user {
            continue;
        }
        let text = (|| {
            let content = parsed.get("message")?.get("content")?;
            if let Some(s) = content.as_str() {
                Some(s.to_string())
            } else if let Some(arr) = content.as_array() {
                Some(
                    arr.iter()
                        .find_map(|block| block.get("text")?.as_str())?
                        .to_string(),
                )
            } else {
                None
            }
        })();
        if let Some(text) = text {
            if text.len() > 80 {
                let truncated: String = text.chars().take(80).collect();
                return Some(format!("{truncated}…"));
            }
            return Some(text);
        }
    }
    None
}

pub fn resolve_auto_title(cwd: &str, session_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let project_dir = cwd_to_project_dir(cwd);
    let base = format!("{home}/.claude/projects/{project_dir}");

    // 1. Try sessions-index.json summary
    let index_path = format!("{base}/sessions-index.json");
    if let Ok(json) = std::fs::read_to_string(&index_path)
        && let Some(summary) = parse_summary_from_index(&json, session_id)
    {
        return Some(summary);
    }

    let jsonl_path = format!("{base}/{session_id}.jsonl");

    // 2. Try plan title from slug
    if let Ok(file) = std::fs::File::open(&jsonl_path) {
        let reader = std::io::BufReader::new(file);
        if let Some(slug) = extract_slug_from_jsonl(reader) {
            let plans_dir = std::path::PathBuf::from(format!("{home}/.claude/plans"));
            if let Some(title) = read_plan_title(&plans_dir, &slug) {
                return Some(title);
            }
        }
    }

    // 3. Fallback: last user message from session JSONL
    if let Ok(file) = std::fs::File::open(&jsonl_path) {
        if let Some(prompt) = extract_last_prompt_from_jsonl(&file) {
            return Some(prompt);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_cwd_to_project_dir_basic() {
        assert_eq!(
            super::cwd_to_project_dir("/home/user/proj"),
            "-home-user-proj"
        );
    }

    #[test]
    fn test_cwd_to_project_dir_with_dot() {
        assert_eq!(
            super::cwd_to_project_dir("/home/user/.config/proj"),
            "-home-user--config-proj"
        );
    }

    #[test]
    fn test_cwd_to_project_dir_trailing_slash() {
        assert_eq!(
            super::cwd_to_project_dir("/home/user/proj/"),
            "-home-user-proj"
        );
    }

    #[test]
    fn test_parse_summary_from_index_found() {
        let json = r#"[
            {"sessionId": "abc-123", "summary": "Fix login bug"},
            {"sessionId": "def-456", "summary": "Add tests"}
        ]"#;
        assert_eq!(
            super::parse_summary_from_index(json, "abc-123"),
            Some("Fix login bug".to_string())
        );
    }

    #[test]
    fn test_parse_summary_from_index_not_found() {
        let json = r#"[{"sessionId": "abc-123", "summary": "Fix bug"}]"#;
        assert_eq!(super::parse_summary_from_index(json, "unknown"), None);
    }

    #[test]
    fn test_parse_summary_from_index_invalid_json() {
        assert_eq!(super::parse_summary_from_index("not json", "abc"), None);
    }

    #[test]
    fn test_read_plan_title_with_heading() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("my-slug.md");
        std::fs::write(&plan_path, "# My Cool Plan\nsome details\n").unwrap();
        assert_eq!(
            super::read_plan_title(dir.path(), "my-slug"),
            Some("My Cool Plan".to_string())
        );
    }

    #[test]
    fn test_read_plan_title_no_heading_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("my-slug.md");
        std::fs::write(&plan_path, "No heading here\n").unwrap();
        assert_eq!(
            super::read_plan_title(dir.path(), "my-slug"),
            Some("No heading here".to_string())
        );
    }

    #[test]
    fn test_read_plan_title_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(super::read_plan_title(dir.path(), "nonexistent"), None);
    }

    #[test]
    fn test_extract_slug_from_jsonl_found() {
        let jsonl = r#"{"type":"user","message":{"content":"hello"},"slug":"my-cool-plan"}
{"type":"user","message":{"content":"world"}}"#;
        let reader = std::io::BufReader::new(jsonl.as_bytes());
        assert_eq!(
            super::extract_slug_from_jsonl(reader),
            Some("my-cool-plan".to_string())
        );
    }

    #[test]
    fn test_extract_slug_from_jsonl_no_slug() {
        let jsonl = r#"{"type":"user","message":{"content":"hello"}}
{"type":"user","message":{"content":"world"}}"#;
        let reader = std::io::BufReader::new(jsonl.as_bytes());
        assert_eq!(super::extract_slug_from_jsonl(reader), None);
    }

    #[test]
    fn test_extract_slug_from_jsonl_empty() {
        let reader = std::io::BufReader::new("".as_bytes());
        assert_eq!(super::extract_slug_from_jsonl(reader), None);
    }

    fn write_tempfile(content: &str) -> (tempfile::TempDir, std::fs::File) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, content).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        (dir, file)
    }

    #[test]
    fn test_extract_last_prompt_short() {
        let (_dir, file) = write_tempfile(
            r#"{"type":"user","message":{"content":"hello world"}}"#,
        );
        assert_eq!(
            super::extract_last_prompt_from_jsonl(&file),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_extract_last_prompt_truncated() {
        let long_text = "a".repeat(100);
        let jsonl = format!(r#"{{"type":"user","message":{{"content":"{long_text}"}}}}"#);
        let (_dir, file) = write_tempfile(&jsonl);
        let result = super::extract_last_prompt_from_jsonl(&file).unwrap();
        assert_eq!(result.chars().count(), 81); // 80 + "…"
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_extract_last_prompt_skips_non_user() {
        let jsonl = "{\"type\":\"system\",\"message\":{\"content\":\"sys\"}}\n\
                     {\"type\":\"user\",\"message\":{\"content\":\"user msg\"}}";
        let (_dir, file) = write_tempfile(jsonl);
        assert_eq!(
            super::extract_last_prompt_from_jsonl(&file),
            Some("user msg".to_string())
        );
    }

    #[test]
    fn test_extract_last_prompt_array_content() {
        let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"block text"}]}}"#;
        let (_dir, file) = write_tempfile(jsonl);
        assert_eq!(
            super::extract_last_prompt_from_jsonl(&file),
            Some("block text".to_string())
        );
    }

    #[test]
    fn test_extract_last_prompt_empty() {
        let (_dir, file) = write_tempfile("");
        assert_eq!(super::extract_last_prompt_from_jsonl(&file), None);
    }

    #[test]
    fn test_extract_last_prompt_picks_last_user() {
        let jsonl = "{\"type\":\"user\",\"message\":{\"content\":\"first msg\"}}\n\
                     {\"type\":\"assistant\",\"message\":{\"content\":\"reply\"}}\n\
                     {\"type\":\"user\",\"message\":{\"content\":\"last msg\"}}";
        let (_dir, file) = write_tempfile(jsonl);
        assert_eq!(
            super::extract_last_prompt_from_jsonl(&file),
            Some("last msg".to_string())
        );
    }
}
