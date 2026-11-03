const DEFAULT_HOTWORDS: &str = "Golang=高粱|高亮|勾烂|狗浪|国王|够浪|GO\nGMP=鸡一|鸡安屁|鸡艾姆屁|积木P|节目P|G M P\ngoroutine=固若听|狗肉听\nchannel\nscheduler\ncontext\nMySQL\nPostgreSQL\nRedis\nKafka\nRabbitMQ\nDocker\nKubernetes\ngRPC\nHTTP\nHTTPS\nTCP\nJVM\nJava\nSpring Boot\nPython\nRust\nReact\nTypeScript\nNode JS\n微服务\n高并发\n数据库\n索引\n事务\nMVCC\n分布式\nCAP\nRaft\nPaxos\n";

#[derive(Clone, Debug)]
pub(crate) struct HotwordEntry {
    pub(crate) canonical: String,
    pub(crate) aliases: Vec<String>,
}

pub(crate) fn parse_hotwords(extra_hotwords: &str) -> Vec<HotwordEntry> {
    let mut entries = Vec::<HotwordEntry>::new();
    for line in format!("{DEFAULT_HOTWORDS}\n{extra_hotwords}").lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (canonical, aliases) = line
            .split_once('=')
            .map(|(canonical, aliases)| {
                (
                    canonical.trim(),
                    aliases
                        .split('|')
                        .map(str::trim)
                        .filter(|alias| !alias.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or((line, Vec::new()));
        if canonical.is_empty() {
            continue;
        }
        if let Some(existing) = entries
            .iter_mut()
            .find(|entry| entry.canonical.eq_ignore_ascii_case(canonical))
        {
            for alias in aliases {
                if !existing.aliases.iter().any(|item| item == &alias) {
                    existing.aliases.push(alias);
                }
            }
        } else {
            entries.push(HotwordEntry {
                canonical: canonical.to_string(),
                aliases,
            });
        }
    }
    entries
}

pub(crate) fn hotword_terms(extra_hotwords: &str) -> Vec<String> {
    parse_hotwords(extra_hotwords)
        .into_iter()
        .map(|entry| entry.canonical)
        .collect()
}

pub(crate) fn replace_ascii_word(text: &str, alias: &str, replacement: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    let needle = alias.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = lowered[cursor..].find(&needle) {
        let start = cursor + relative;
        let end = start + needle.len();
        let before_is_word = start > 0 && text.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after_is_word = end < text.len() && text.as_bytes()[end].is_ascii_alphanumeric();
        if before_is_word || after_is_word {
            output.push_str(&text[cursor..end]);
        } else {
            output.push_str(&text[cursor..start]);
            output.push_str(replacement);
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

pub(crate) fn normalize_hotword_aliases(text: &str, entries: &[HotwordEntry]) -> (String, usize) {
    let mut normalized = text.to_string();
    for entry in entries {
        let mut aliases = entry.aliases.clone();
        aliases.sort_by_key(|alias| std::cmp::Reverse(alias.chars().count()));
        for alias in aliases {
            normalized = if alias.is_ascii() {
                replace_ascii_word(&normalized, &alias, &entry.canonical)
            } else {
                normalized.replace(&alias, &entry.canonical)
            };
        }
    }
    let lowered = normalized.to_ascii_lowercase();
    let hits = entries
        .iter()
        .filter(|entry| lowered.contains(&entry.canonical.to_ascii_lowercase()))
        .count();
    (normalized, hits)
}

pub(crate) fn normalize_transcript(text: &str, extra_hotwords: &str) -> String {
    normalize_hotword_aliases(text, &parse_hotwords(extra_hotwords)).0
}

#[cfg(test)]
mod tests {
    use super::{normalize_hotword_aliases, parse_hotwords, replace_ascii_word};

    #[test]
    fn normalizes_common_technical_term_misrecognitions() {
        let entries = parse_hotwords("");
        let (text, hits) = normalize_hotword_aliases("请介绍一下高粱的鸡一模型", &entries);
        assert_eq!(text, "请介绍一下Golang的GMP模型");
        assert_eq!(hits, 2);
    }

    #[test]
    fn replaces_ascii_aliases_only_at_word_boundaries() {
        assert_eq!(replace_ascii_word("GO GMP", "GO", "Golang"), "Golang GMP");
        assert_eq!(replace_ascii_word("Golang", "GO", "Golang"), "Golang");
    }

    #[test]
    fn merges_duplicate_hotword_definitions() {
        let entries = parse_hotwords("Golang=够蓝\nGolang");
        let golang = entries
            .iter()
            .find(|entry| entry.canonical == "Golang")
            .expect("Golang hotword");
        assert!(golang.aliases.iter().any(|alias| alias == "够蓝"));
    }
}
