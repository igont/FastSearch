use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;

use fastsearch::domain::{
    CanonicalRecord, ContentHash, RecordKind, SourceLocator, SourceSelector, StableId,
};

fn value_after_tab(row: &str, index: usize) -> &str {
    row.split('\t').nth(index).expect("synthetic TSV shape")
}

fn main() -> Result<(), String> {
    let fixture_root = env::args().nth(1).ok_or("fixture root is required")?;
    let fixture_root = Path::new(&fixture_root);
    let markdown_path = fixture_root.join("one-section.md");
    let tsv_path = fixture_root.join("registry.tsv");
    let markdown = fs::read_to_string(&markdown_path).map_err(|error| error.to_string())?;
    let tsv = fs::read_to_string(&tsv_path).map_err(|error| error.to_string())?;

    let markdown_title = markdown
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("# "))
        .ok_or("one Markdown section with an H1 is required")?;
    let markdown_content = markdown.lines().skip(2).collect::<Vec<_>>().join("\n");
    let tsv_row = tsv.lines().nth(1).ok_or("one TSV data row is required")?;

    let use_shared_values = env::var_os("D1_FORCE_SHARED_VALUES").is_some();
    let markdown_id = StableId::parse(if use_shared_values {
        "synthetic:shared"
    } else {
        "markdown:synthetic/one-section.md#FastSearch"
    })
    .map_err(|error| error.to_string())?;
    let tsv_id = StableId::parse(if use_shared_values {
        "synthetic:shared"
    } else {
        "registry:synthetic/registry.tsv#row=2"
    })
    .map_err(|error| error.to_string())?;
    let markdown_hash = ContentHash::parse(if use_shared_values {
        "sha256:shared"
    } else {
        "sha256:293d6a27157b9ee1161ce4539dcec42b76241ec3500c5b3b87fdd2bb6c7c5259"
    })
    .map_err(|error| error.to_string())?;
    let tsv_hash = ContentHash::parse(if use_shared_values {
        "sha256:shared"
    } else {
        "sha256:34c09da5ceef057e552c5498cbec6222c5868f43c705e4f3334315f771f0a3f4"
    })
    .map_err(|error| error.to_string())?;

    let markdown_record = CanonicalRecord::new(
        markdown_id,
        RecordKind::MarkdownSection,
        SourceLocator::markdown("synthetic/one-section.md", [markdown_title])
            .map_err(|error| error.to_string())?,
        markdown_title,
        markdown_content,
        BTreeMap::from([(String::from("format"), String::from("markdown"))]),
        Vec::new(),
        markdown_hash,
    )
    .map_err(|error| error.to_string())?;
    let tsv_record = CanonicalRecord::new(
        tsv_id,
        RecordKind::RegistryRow,
        SourceLocator::registry_row(
            "synthetic/registry.tsv",
            NonZeroUsize::new(2).ok_or("synthetic TSV row number must be non-zero")?,
        )
        .map_err(|error| error.to_string())?,
        value_after_tab(tsv_row, 0),
        tsv_row,
        BTreeMap::from([
            (String::from("format"), String::from("tsv")),
            (
                String::from("owner"),
                String::from(value_after_tab(tsv_row, 1)),
            ),
            (
                String::from("status"),
                String::from(value_after_tab(tsv_row, 2)),
            ),
        ]),
        Vec::new(),
        tsv_hash,
    )
    .map_err(|error| error.to_string())?;

    assert_ne!(
        markdown_record.id().as_str(),
        tsv_record.id().as_str(),
        "stable IDs must differ"
    );
    assert_ne!(
        markdown_record.content_hash().as_str(),
        tsv_record.content_hash().as_str(),
        "content hashes must differ"
    );
    assert_eq!(markdown_record.locator().path(), "synthetic/one-section.md");
    assert_eq!(tsv_record.locator().path(), "synthetic/registry.tsv");
    assert_eq!(markdown_record.kind(), RecordKind::MarkdownSection);
    assert_eq!(tsv_record.kind(), RecordKind::RegistryRow);
    assert!(matches!(
        markdown_record.locator().selector(),
        SourceSelector::MarkdownHeading { heading_path } if heading_path == &["FastSearch"]
    ));
    assert!(matches!(
        tsv_record.locator().selector(),
        SourceSelector::RegistryRow { row } if row.get() == 2
    ));
    assert_eq!(
        markdown_record.searchable_content(),
        "Каноническая запись должна хранить заголовок и текст секции."
    );
    assert_eq!(
        tsv_record.searchable_content(),
        "FS-001\tsearch-team\tactive"
    );
    assert_eq!(markdown_record.title(), "FastSearch");
    assert_eq!(tsv_record.title(), "FS-001");
    assert_eq!(
        markdown_record.metadata().get("format"),
        Some(&String::from("markdown"))
    );
    assert_eq!(
        tsv_record.metadata().get("owner"),
        Some(&String::from("search-team"))
    );
    assert_eq!(
        tsv_record.metadata().get("status"),
        Some(&String::from("active"))
    );
    assert!(markdown_record.relations().is_empty());
    assert!(tsv_record.relations().is_empty());

    println!("D1 PASS: public CanonicalRecord preserves both synthetic records");
    Ok(())
}
