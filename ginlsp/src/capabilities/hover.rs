use super::definition::find_definition_line;
use ginc::ast::DocComment;

fn extract_definition_block(source: &str, start_line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if start_line >= lines.len() {
        return String::new();
    }

    let def_line = lines[start_line];
    let base_indent = def_line.len() - def_line.trim_start().len();

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
        let relative = &line[base_indent..];
        block.push_str(relative);
    }

    block
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

    if let Some(ref dc) = doc {
        md.push_str(&format!("\n---\n\n{}\n", dc.0));
    }

    md
}
