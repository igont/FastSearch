use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    CanonicalRecord, ContentHash, EmbeddingModelId, ErrorKind, FastSearchError, RecordKind,
    StableId,
};

pub(crate) const CHUNKER_VERSION: &str = "markdown-structure-v3";
const MAX_LIST_CHARS: usize = 1_200;
pub(crate) const PARENT_ID_METADATA: &str = "_fastsearch_parent_id";
const CHUNK_KIND_METADATA: &str = "_fastsearch_chunk_kind";
const SOURCE_START_METADATA: &str = "_fastsearch_source_start_line";
const SOURCE_END_METADATA: &str = "_fastsearch_source_end_line";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChunkKind {
    Paragraph,
    UnorderedList,
    OrderedList,
    TableRow,
    CodeBlock,
    Section,
}

impl ChunkKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::UnorderedList => "unordered_list",
            Self::OrderedList => "ordered_list",
            Self::TableRow => "table_row",
            Self::CodeBlock => "code_block",
            Self::Section => "section",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ChunkEnvelope {
    pub chunker_version: String,
    pub chunk_id: String,
    pub record_id: String,
    pub source_root_id: Option<String>,
    pub source_path: String,
    pub source_hash: String,
    pub heading_path: Vec<String>,
    pub kind: ChunkKind,
    pub source_line_start: Option<usize>,
    pub source_line_end: Option<usize>,
    pub raw_block: String,
    pub context: String,
    pub lexical_input: String,
    pub embedding_model: String,
    pub embedding_input: String,
    pub embedding_input_sha256: String,
    pub characters: usize,
}

pub(crate) struct ProjectedCorpus {
    pub records: Vec<CanonicalRecord>,
    pub chunks: Vec<ChunkEnvelope>,
}

pub(crate) fn project_records(
    records: &[CanonicalRecord],
    model: EmbeddingModelId,
) -> Result<ProjectedCorpus, FastSearchError> {
    let mut projected = Vec::new();
    let mut envelopes = Vec::new();
    for record in records {
        if record.kind() != RecordKind::MarkdownSection {
            projected.push(record.clone());
            continue;
        }
        let context = heading_context(record);
        let base_line = record
            .metadata()
            .get(SOURCE_START_METADATA)
            .and_then(|value| value.parse::<usize>().ok());
        for (ordinal, block) in split_blocks(record.searchable_content())
            .into_iter()
            .enumerate()
        {
            let block_text = block.text.trim().to_owned();
            if block_text.is_empty() {
                continue;
            }
            let projected_id =
                make_chunk_id(record.id().as_str(), ordinal, block.kind, &block_text);
            let visible_text = visible_block_text(block.kind, &block_text);
            let lexical_input = format!("{context}: {visible_text}");
            let embedding_input = embedding_input(model, &lexical_input);
            let source_start = base_line.map(|line| line + block.start_line);
            let source_end = base_line.map(|line| line + block.end_line);
            let mut metadata = record.metadata().clone();
            metadata.insert(
                PARENT_ID_METADATA.to_owned(),
                record.id().as_str().to_owned(),
            );
            metadata.insert(
                CHUNK_KIND_METADATA.to_owned(),
                block.kind.as_str().to_owned(),
            );
            if let Some(line) = source_start {
                metadata.insert(SOURCE_START_METADATA.to_owned(), line.to_string());
            }
            if let Some(line) = source_end {
                metadata.insert(SOURCE_END_METADATA.to_owned(), line.to_string());
            }
            let hash = content_hash(&projected_id, &lexical_input);
            let projection = CanonicalRecord::new(
                StableId::parse(projected_id.clone())?,
                RecordKind::MarkdownSection,
                record.locator().clone(),
                context.clone(),
                visible_text,
                metadata,
                record.relations().to_vec(),
                ContentHash::parse(hash)?,
            )?;
            envelopes.push(ChunkEnvelope {
                chunker_version: CHUNKER_VERSION.to_owned(),
                chunk_id: projected_id,
                record_id: record.id().as_str().to_owned(),
                source_root_id: record.metadata().get("_fastsearch_root_id").cloned(),
                source_path: record.locator().path().to_owned(),
                source_hash: record.content_hash().as_str().to_owned(),
                heading_path: heading_path(record),
                kind: block.kind,
                source_line_start: source_start,
                source_line_end: source_end,
                raw_block: block_text,
                context: context.clone(),
                lexical_input,
                embedding_model: model.slug().to_owned(),
                embedding_input_sha256: sha256(&embedding_input),
                characters: embedding_input.chars().count(),
                embedding_input,
            });
            projected.push(projection);
        }
    }
    Ok(ProjectedCorpus {
        records: projected,
        chunks: envelopes,
    })
}

pub(crate) fn lexical_input(record: &CanonicalRecord) -> String {
    if record.metadata().contains_key(PARENT_ID_METADATA) {
        format!("{}: {}", record.title(), record.searchable_content())
    } else {
        format!("{}\n{}", record.title(), record.searchable_content())
    }
}

pub(crate) fn embedding_input(model: EmbeddingModelId, lexical: &str) -> String {
    match model {
        EmbeddingModelId::MultilingualE5Small
        | EmbeddingModelId::MultilingualE5Base
        | EmbeddingModelId::MultilingualE5Large => format!("passage: {lexical}"),
        EmbeddingModelId::NomicEmbedTextV2Moe => format!("search_document: {lexical}"),
        _ => lexical.to_owned(),
    }
}

fn heading_path(record: &CanonicalRecord) -> Vec<String> {
    match record.locator().selector() {
        crate::domain::SourceSelector::MarkdownHeading { heading_path } => heading_path.clone(),
        _ => vec![record.title().to_owned()],
    }
}

fn heading_context(record: &CanonicalRecord) -> String {
    heading_path(record).join(" > ")
}

fn visible_markdown_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0_usize;
    while index < input.len() {
        if input[index..].starts_with("[[")
            && let Some(relative_end) = input[index + 2..].find("]]")
        {
            let end = index + 2 + relative_end;
            output.push_str(&visible_wiki_link(&input[index + 2..end]));
            index = end + 2;
            continue;
        }

        let (open, label_start) = if input[index..].starts_with("![") {
            (index + 1, index + 2)
        } else if input[index..].starts_with('[') {
            (index, index + 1)
        } else {
            let character = input[index..]
                .chars()
                .next()
                .expect("index remains on a UTF-8 boundary");
            output.push(character);
            index += character.len_utf8();
            continue;
        };

        let Some(label_end) = matching_delimiter(input, open, b'[', b']') else {
            output.push_str(&input[index..label_start]);
            index = label_start;
            continue;
        };
        let target_open = label_end + 1;
        if input.as_bytes().get(target_open) != Some(&b'(') {
            output.push_str(&input[index..=label_end]);
            index = target_open;
            continue;
        }
        let Some(target_end) = matching_delimiter(input, target_open, b'(', b')') else {
            output.push_str(&input[index..=label_end]);
            index = target_open;
            continue;
        };
        output.push_str(&visible_markdown_text(&input[label_start..label_end]));
        index = target_end + 1;
    }
    output
}

fn visible_block_text(kind: ChunkKind, input: &str) -> String {
    let visible = visible_markdown_text(input);
    match kind {
        ChunkKind::UnorderedList | ChunkKind::OrderedList => semantic_list_text(&visible),
        _ => visible,
    }
}

fn semantic_list_text(input: &str) -> String {
    let item_indents = input
        .lines()
        .filter(|line| unordered_item(line).is_some() || ordered_item(line).is_some())
        .map(leading_spaces)
        .collect::<Vec<_>>();
    let base_indent = item_indents.iter().copied().min().unwrap_or(0);
    let mut output = String::with_capacity(input.len());

    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let item = unordered_item(line).or_else(|| ordered_item(line));
        if let Some(item) = item {
            if !output.is_empty() {
                if leading_spaces(line) == base_indent {
                    if matches!(output.chars().last(), Some('.' | '!' | '?' | ';' | ':')) {
                        output.push(' ');
                    } else {
                        output.push_str("; ");
                    }
                } else if output.ends_with(':') {
                    output.push(' ');
                } else {
                    output.push_str(" — ");
                }
            }
            output.push_str(item.trim());
        } else {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(line.trim());
        }
    }

    output
}

fn matching_delimiter(input: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let mut depth = 0_usize;
    let mut escaped = false;
    for (offset, byte) in input.as_bytes()[open..].iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
        } else if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(open + offset);
            }
        }
    }
    None
}

fn visible_wiki_link(value: &str) -> String {
    if let Some((_, alias)) = value.rsplit_once('|') {
        return alias.trim().to_owned();
    }
    let target = value.trim().trim_matches(['\'', '"']);
    let without_anchor = target.split_once('#').map_or(target, |(path, _)| path);
    let file = without_anchor
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(without_anchor)
        .trim_end_matches(".md");
    if file.is_empty() {
        target.trim_start_matches('#').to_owned()
    } else {
        file.to_owned()
    }
}

#[derive(Debug)]
struct Block {
    kind: ChunkKind,
    text: String,
    start_line: usize,
    end_line: usize,
}

fn split_blocks(content: &str) -> Vec<Block> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim().is_empty() || is_thematic_break(lines[index]) {
            index += 1;
            continue;
        }
        if lines[index].trim_start().starts_with("```") {
            let start = index;
            index += 1;
            while index < lines.len() {
                let closing = lines[index].trim_start().starts_with("```");
                index += 1;
                if closing {
                    break;
                }
            }
            blocks.push(block(ChunkKind::CodeBlock, &lines, start, index));
            continue;
        }
        if let Some(table) = table_at(&lines, index) {
            blocks.extend(table.0);
            index = table.1;
            continue;
        }
        if unordered_item(lines[index]).is_some() {
            let (lists, next) = unordered_list(&lines, index);
            blocks.extend(lists);
            index = next;
            continue;
        }
        if ordered_item(lines[index]).is_some() {
            let (lists, next) = ordered_list(&lines, index);
            blocks.extend(lists);
            index = next;
            continue;
        }
        let start = index;
        index += 1;
        while index < lines.len()
            && !lines[index].trim().is_empty()
            && !is_thematic_break(lines[index])
            && unordered_item(lines[index]).is_none()
            && ordered_item(lines[index]).is_none()
            && table_at(&lines, index).is_none()
            && !lines[index].trim_start().starts_with("```")
        {
            index += 1;
        }
        blocks.push(block(ChunkKind::Paragraph, &lines, start, index));
    }
    if blocks.is_empty() && !content.trim().is_empty() {
        blocks.push(Block {
            kind: ChunkKind::Section,
            text: content.trim().to_owned(),
            start_line: 0,
            end_line: content.lines().count().saturating_sub(1),
        });
    }
    blocks
}

fn unordered_list(lines: &[&str], start: usize) -> (Vec<Block>, usize) {
    let mut starts = Vec::new();
    let mut index = start;
    let base_indent = leading_spaces(lines[start]);
    while index < lines.len() && !lines[index].trim().is_empty() {
        if unordered_item(lines[index]).is_some() && leading_spaces(lines[index]) == base_indent {
            starts.push(index);
        } else if !is_indented(lines[index]) {
            break;
        }
        index += 1;
    }
    let mut blocks = Vec::new();
    for (item_start, item_end) in bounded_item_windows(lines, &starts, index, MAX_LIST_CHARS) {
        blocks.push(block(ChunkKind::UnorderedList, lines, item_start, item_end));
    }
    (blocks, index)
}

fn ordered_list(lines: &[&str], start: usize) -> (Vec<Block>, usize) {
    let mut starts = Vec::new();
    let mut index = start;
    let base_indent = leading_spaces(lines[start]);
    while index < lines.len() && !lines[index].trim().is_empty() {
        if ordered_item(lines[index]).is_some() && leading_spaces(lines[index]) == base_indent {
            starts.push(index);
        } else if !is_indented(lines[index]) {
            break;
        }
        index += 1;
    }
    let blocks = bounded_item_windows(lines, &starts, index, MAX_LIST_CHARS)
        .into_iter()
        .map(|(item_start, item_end)| block(ChunkKind::OrderedList, lines, item_start, item_end))
        .collect();
    (blocks, index)
}

fn bounded_item_windows(
    lines: &[&str],
    starts: &[usize],
    end: usize,
    max_chars: usize,
) -> Vec<(usize, usize)> {
    let mut windows = Vec::new();
    let mut window_start = starts[0];
    let mut window_chars = 0;

    for (position, item_start) in starts.iter().copied().enumerate() {
        let item_end = starts.get(position + 1).copied().unwrap_or(end);
        let item_chars = lines[item_start..item_end]
            .iter()
            .map(|line| line.len() + 1)
            .sum::<usize>();
        if window_chars > 0 && window_chars + item_chars > max_chars {
            windows.push((window_start, item_start));
            window_start = item_start;
            window_chars = 0;
        }
        window_chars += item_chars;
    }

    windows.push((window_start, end));
    windows
}

fn table_at(lines: &[&str], start: usize) -> Option<(Vec<Block>, usize)> {
    if start + 1 >= lines.len()
        || !lines[start].contains('|')
        || !is_table_separator(lines[start + 1])
    {
        return None;
    }
    let headers = table_cells(lines[start]);
    if headers.is_empty() {
        return None;
    }
    let mut index = start + 2;
    let mut blocks = Vec::new();
    while index < lines.len() && lines[index].contains('|') && !lines[index].trim().is_empty() {
        let values = table_cells(lines[index]);
        let normalized = headers
            .iter()
            .enumerate()
            .filter_map(|(cell, header)| values.get(cell).map(|value| format!("{header}: {value}")))
            .collect::<Vec<_>>()
            .join("; ");
        if !normalized.is_empty() {
            blocks.push(Block {
                kind: ChunkKind::TableRow,
                text: normalized,
                start_line: index,
                end_line: index,
            });
        }
        index += 1;
    }
    Some((blocks, index))
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .map(ToOwned::to_owned)
        .collect()
}

fn is_table_separator(line: &str) -> bool {
    let cells = table_cells(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':').trim();
            cell.len() >= 3 && cell.bytes().all(|byte| byte == b'-')
        })
}

fn block(kind: ChunkKind, lines: &[&str], start: usize, end: usize) -> Block {
    Block {
        kind,
        text: lines[start..end].join("\n").trim().to_owned(),
        start_line: start,
        end_line: end.saturating_sub(1),
    }
}

fn unordered_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
}

fn ordered_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    trimmed.get(digits..)?.strip_prefix(". ")
}

fn is_indented(line: &str) -> bool {
    leading_spaces(line) > 0
}

fn leading_spaces(line: &str) -> usize {
    line.bytes()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count()
}

fn is_thematic_break(line: &str) -> bool {
    matches!(line.trim(), "---" | "***" | "___")
}

fn make_chunk_id(parent: &str, ordinal: usize, kind: ChunkKind, text: &str) -> String {
    let digest = sha256(&format!("{parent}\0{}\0{text}", kind.as_str()));
    format!("chunk-v1:{parent}:{}:{}", ordinal + 1, &digest[..16])
}

fn content_hash(id: &str, lexical: &str) -> String {
    format!("sha256:v1:{}", sha256(&format!("{id}\0{lexical}")))
}

fn sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(crate) fn parent_id(record: &CanonicalRecord) -> Result<Option<StableId>, FastSearchError> {
    record
        .metadata()
        .get(PARENT_ID_METADATA)
        .map(|value| StableId::parse(value.clone()))
        .transpose()
        .map_err(|error| FastSearchError::new(ErrorKind::ProjectionFailure, error.message()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::{SourceLocator, SourceSelector};

    fn record(content: &str) -> CanonicalRecord {
        let locator =
            SourceLocator::markdown("guide.md", ["Руководство", "Правила проектирования"]).unwrap();
        let mut metadata = BTreeMap::new();
        metadata.insert(SOURCE_START_METADATA.to_owned(), "10".to_owned());
        CanonicalRecord::new(
            StableId::parse("section-1").unwrap(),
            RecordKind::MarkdownSection,
            locator,
            "Правила проектирования",
            content,
            metadata,
            Vec::new(),
            ContentHash::parse("hash-1").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn paragraphs_receive_full_heading_context() {
        let corpus = project_records(
            &[record("Нужно делать так-то\n\nНельзя делать так-то")],
            EmbeddingModelId::MultilingualE5Small,
        )
        .unwrap();

        assert_eq!(corpus.records.len(), 2);
        assert_eq!(
            corpus.chunks[0].lexical_input,
            "Руководство > Правила проектирования: Нужно делать так-то"
        );
        assert_eq!(corpus.chunks[0].source_line_start, Some(10));
        assert!(corpus.chunks[0].embedding_input.starts_with("passage: "));
    }

    #[test]
    fn list_is_one_semantic_chunk_and_table_rows_remain_independent() {
        let content = "- Первый пункт\n  - Деталь\n- Второй пункт\n\n| Правило | Статус |\n| --- | --- |\n| Один | Обязательно |";
        let corpus =
            project_records(&[record(content)], EmbeddingModelId::Qwen3Embedding06B).unwrap();

        assert_eq!(corpus.chunks.len(), 2);
        assert_eq!(corpus.chunks[0].kind, ChunkKind::UnorderedList);
        assert_eq!(
            corpus.chunks[0].raw_block,
            "- Первый пункт\n  - Деталь\n- Второй пункт"
        );
        assert_eq!(
            corpus.chunks[0].lexical_input,
            "Руководство > Правила проектирования: Первый пункт — Деталь; Второй пункт"
        );
        assert_eq!(corpus.chunks[1].kind, ChunkKind::TableRow);
        assert_eq!(
            corpus.chunks[1].raw_block,
            "Правило: Один; Статус: Обязательно"
        );
    }

    #[test]
    fn long_unordered_lists_split_only_between_complete_items() {
        let first = format!("- {}", "а".repeat(700));
        let second = format!("- {}\n  - вложенная деталь", "б".repeat(700));
        let content = format!("{first}\n{second}");

        let corpus =
            project_records(&[record(&content)], EmbeddingModelId::Qwen3Embedding06B).unwrap();

        assert_eq!(corpus.chunks.len(), 2);
        assert_eq!(corpus.chunks[0].kind, ChunkKind::UnorderedList);
        assert_eq!(corpus.chunks[1].kind, ChunkKind::UnorderedList);
        assert!(
            corpus.chunks[1]
                .lexical_input
                .ends_with(" — вложенная деталь")
        );
    }

    #[test]
    fn complete_list_sentences_do_not_receive_an_extra_semicolon() {
        assert_eq!(
            semantic_list_text("- Первое правило.\n- Второе правило."),
            "Первое правило. Второе правило."
        );
        assert_eq!(
            semantic_list_text("- Родитель:\n  - Деталь"),
            "Родитель: Деталь"
        );
    }

    #[test]
    fn projection_keeps_the_canonical_locator() {
        let corpus =
            project_records(&[record("Текст")], EmbeddingModelId::Qwen3Embedding06B).unwrap();
        assert!(matches!(
            corpus.records[0].locator().selector(),
            SourceSelector::MarkdownHeading { .. }
        ));
        assert_eq!(
            parent_id(&corpus.records[0]).unwrap().unwrap().as_str(),
            "section-1"
        );
    }

    #[test]
    fn index_inputs_keep_only_visible_markdown_and_obsidian_link_labels() {
        let content = "[FastGraph как проверяемый граф знаний](<../Парадигмы/01 FastGraph как проверяемый граф знаний.md>) и [[../TDR/TDR-FG-1.md|TDR-FG-1]], затем [[Архитектура/Контракт.md]].";

        let corpus =
            project_records(&[record(content)], EmbeddingModelId::MultilingualE5Small).unwrap();

        assert_eq!(corpus.chunks[0].raw_block, content);
        assert_eq!(
            corpus.chunks[0].lexical_input,
            "Руководство > Правила проектирования: FastGraph как проверяемый граф знаний и TDR-FG-1, затем Контракт."
        );
        assert_eq!(
            corpus.chunks[0].embedding_input,
            "passage: Руководство > Правила проектирования: FastGraph как проверяемый граф знаний и TDR-FG-1, затем Контракт."
        );
    }
}
