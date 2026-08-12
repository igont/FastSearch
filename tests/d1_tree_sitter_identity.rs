use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_VISITED_NODES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralSymbol {
    language: &'static str,
    kind: &'static str,
    name: String,
    start_byte: usize,
}

fn language_for(extension: &str) -> Option<(&'static str, Language)> {
    match extension {
        "rs" => Some(("rust", tree_sitter_rust::LANGUAGE.into())),
        "py" => Some(("python", tree_sitter_python::LANGUAGE.into())),
        _ => None,
    }
}

fn structural_symbols(
    extension: &str,
    source: &str,
) -> Result<Vec<StructuralSymbol>, &'static str> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("source exceeds D1 spike byte limit");
    }
    let (language_name, language) = language_for(extension).ok_or("unsupported language")?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|_| "grammar load failed")?;
    let tree = parser
        .parse(source, None)
        .ok_or("parser produced no tree")?;
    if tree.root_node().has_error() {
        return Err("parse error publishes no structural symbols");
    }

    let mut visited = 0usize;
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > MAX_VISITED_NODES {
            return Err("parse tree exceeds D1 spike node limit");
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    let query = Query::new(&language, declaration_query(language_name))
        .map_err(|_| "declaration query is incompatible with grammar")?;
    let mut query_cursor = QueryCursor::new();
    let mut symbols = Vec::new();
    let mut captures = query_cursor.captures(&query, tree.root_node(), source.as_bytes());
    while let Some((query_match, capture_index)) = captures.next() {
        let declaration = query_match.captures[*capture_index].node;
        let kind = declaration_kind(language_name, declaration.kind())
            .ok_or("declaration query captured an unsupported node")?;
        let name = declaration
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            .ok_or("declaration has no UTF-8 name")?;
        symbols.push(StructuralSymbol {
            language: language_name,
            kind,
            name: name.to_owned(),
            start_byte: declaration.start_byte(),
        });
    }
    symbols.sort_by_key(|symbol| symbol.start_byte);
    Ok(symbols)
}

fn declaration_query(language: &str) -> &'static str {
    match language {
        "rust" => "(function_item) @declaration\n(struct_item) @declaration",
        "python" => "(function_definition) @declaration\n(class_definition) @declaration",
        _ => unreachable!("language_for is the only caller"),
    }
}

fn declaration_kind(language: &str, node_kind: &str) -> Option<&'static str> {
    match (language, node_kind) {
        ("rust", "function_item") => Some("function"),
        ("rust", "struct_item") => Some("struct"),
        ("python", "function_definition") => Some("function"),
        ("python", "class_definition") => Some("class"),
        _ => None,
    }
}

fn structural_identity(root: &str, relative_locator: &str, symbol: &StructuralSymbol) -> String {
    format!(
        "structural-v1:{root}:{relative_locator}:{}:{}:{}:{}",
        symbol.language, symbol.kind, symbol.name, symbol.start_byte
    )
}

#[test]
fn duplicate_names_have_distinct_structural_identities() {
    let source = "fn repeated() {}\nmod nested { fn repeated() {} }\n";
    let symbols = structural_symbols("rs", source).expect("valid Rust fixture parses");
    let repeated = symbols
        .iter()
        .filter(|symbol| symbol.name == "repeated")
        .collect::<Vec<_>>();
    assert_eq!(repeated.len(), 2);
    assert_ne!(
        structural_identity("code-fastsearch", "src/navigator.rs", repeated[0]),
        structural_identity("code-fastsearch", "src/navigator.rs", repeated[1]),
        "duplicate declarations must not collapse into one identity"
    );
}

#[test]
fn rust_and_python_declarations_are_deterministic_and_structural_only() {
    let rust = structural_symbols("rs", "pub struct Navigator;\npub fn rebuild() {}\n")
        .expect("Rust declarations parse");
    let python = structural_symbols(
        "py",
        "class Navigator:\n    pass\ndef rebuild():\n    return 1\n",
    )
    .expect("Python declarations parse");
    assert_eq!(
        rust,
        structural_symbols("rs", "pub struct Navigator;\npub fn rebuild() {}\n").unwrap()
    );
    assert_eq!(
        rust.iter()
            .map(|symbol| (&symbol.kind, symbol.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(&"struct", "Navigator"), (&"function", "rebuild")]
    );
    assert_eq!(
        python
            .iter()
            .map(|symbol| (&symbol.kind, symbol.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(&"class", "Navigator"), (&"function", "rebuild")]
    );
    let unicode = structural_identity("code-fastsearch", "src/Навигатор.rs", &rust[0]);
    let renamed = structural_identity("code-fastsearch", "src/navigator.rs", &rust[0]);
    assert_ne!(
        unicode, renamed,
        "rename changes the canonical relative locator"
    );
    assert!(
        !unicode.contains(":\\"),
        "identity never serializes an absolute path"
    );
}

#[test]
fn syntax_errors_limits_and_unsupported_languages_publish_no_partial_facts() {
    assert_eq!(
        structural_symbols("rs", "fn broken("),
        Err("parse error publishes no structural symbols")
    );
    assert_eq!(
        structural_symbols("txt", "fn ignored() {}"),
        Err("unsupported language")
    );
    assert_eq!(
        structural_symbols("py", &"x".repeat(MAX_SOURCE_BYTES + 1)),
        Err("source exceeds D1 spike byte limit")
    );
    assert_eq!(
        structural_symbols("rs", &"fn item() {}\n".repeat(200)),
        Err("parse tree exceeds D1 spike node limit")
    );
}
