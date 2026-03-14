use super::definition::find_definition_line;
use ginc::ast::DocComment;

/// Strip doc comment (--- and everything after) from a line
fn strip_doc_comment(line: &str) -> &str {
    if let Some(pos) = line.find("---") {
        &line[..pos]
    } else {
        line
    }
}

fn extract_definition_block(source: &str, start_line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if start_line >= lines.len() {
        return String::new();
    }

    let def_line = strip_doc_comment(lines[start_line]).trim_end();
    let base_indent = lines[start_line].len() - lines[start_line].trim_start().len();

    let mut block = String::from(def_line.trim_start());

    for line in &lines[start_line + 1..] {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            block.push('\n');
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent <= base_indent {
            break;
        }
        block.push('\n');
        // Strip doc comments from nested lines too
        let relative = strip_doc_comment(&line[base_indent..]).trim_end();
        block.push_str(relative);
    }

    block
}

/// Build hover content for a variant within a union
pub fn build_variant_hover(
    module: &str,
    variant: &str,
    parent_tag: &str,
    doc: Option<&DocComment>,
) -> String {
    let mut result = format!(
        "*{module}.{parent_tag}*\n\n\
        ```gin\n\
        {variant}\n\
        ```"
    );

    if let Some(doc) = doc {
        result.push_str("\n\n---\n\n");
        result.push_str(&doc.0);
    }

    result
}

pub fn build_hover(
    source: &str,
    module: &str,
    word: &str,
    is_tag: bool,
    doc: &Option<DocComment>,
) -> String {
    let mut md = String::new();
    md.push_str(&format!("*{module}*\n\n"));

    if let Some(def_line) = find_definition_line(source, word, is_tag) {
        let block = extract_definition_block(source, def_line);
        md.push_str(&format!("```gin\n{block}\n```\n"));
    } else {
        md.push_str(&format!("```gin\n{word}\n```\n"));
    }

    if let Some(doc) = doc {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }

    md
}
