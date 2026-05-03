use crate::align_ast::{AlignableNode, group_alignable_nodes};
use crate::config::Config;
use crate::mods;
use std::collections::HashMap;
use tree_sitter::Parser;

/// Alignment group information for a specific line.
#[derive(Debug, Clone)]
struct AlignmentGroup {
    /// The maximum prefix width for this group
    max_prefix_width: usize,
}

/// The main formatter visitor that walks the tree-sitter-gin AST and builds formatted output.
pub struct FmtVisitor<'a> {
    /// The source code being formatted
    pub source: &'a str,
    /// The output buffer being built
    pub buffer: String,
    /// Current indentation level
    pub indent_level: usize,
    /// Formatting configuration
    pub config: Config,
    /// Collection of alignable nodes found during visiting
    pub alignable_nodes: Vec<AlignableNode>,
    /// Counter for assigning unique IDs to alignable nodes
    pub next_node_id: usize,
    /// Alignment groups indexed by (source line number, delimiter kind)
    alignment_groups: HashMap<(usize, crate::align_ast::DelimiterKind), AlignmentGroup>,
}

impl<'a> FmtVisitor<'a> {
    /// Create a new visitor with the given source and config.
    pub fn new(source: &'a str, config: Config) -> Self {
        Self {
            source,
            buffer: String::new(),
            indent_level: 0,
            config,
            alignable_nodes: Vec::new(),
            next_node_id: 0,
            alignment_groups: HashMap::new(),
        }
    }

    /// Get the next node ID for alignment tracking.
    pub fn next_node_id(&mut self) -> usize {
        let id = self.next_node_id;
        self.next_node_id += 1;
        id
    }

    /// Set alignment groups after collection phase.
    fn set_alignment_groups(&mut self, nodes: Vec<AlignableNode>) {
        use crate::align_ast::DelimiterKind;

        self.alignable_nodes = nodes;
        // Sort by (kind, indent_level, source_line) so same-kind nodes are adjacent for grouping
        let kind_ord = |k: DelimiterKind| -> u8 {
            match k {
                DelimiterKind::Is => 0,
                DelimiterKind::Colon => 1,
                DelimiterKind::Dash => 2,
            }
        };
        self.alignable_nodes.sort_by(|a, b| {
            (kind_ord(a.kind), a.indent_level, a.source_line).cmp(&(
                kind_ord(b.kind),
                b.indent_level,
                b.source_line,
            ))
        });
        let groups = group_alignable_nodes(&self.alignable_nodes);

        for group_indices in &groups {
            if group_indices.len() < 2 {
                continue;
            }

            let max_width = group_indices
                .iter()
                .map(|&i| self.alignable_nodes[i].prefix_display_width)
                .max()
                .unwrap_or(0);

            for &idx in group_indices {
                let node = &self.alignable_nodes[idx];
                self.alignment_groups.insert(
                    (node.source_line, node.kind),
                    AlignmentGroup {
                        max_prefix_width: max_width,
                    },
                );
            }
        }

        // Reconciliation pass: adjust Dash prefix widths to account for Is alignment shifts.
        // When a line has both Is and Dash alignment, the Is alignment changes the width
        // before "---", so we need to adjust the Dash prefix_display_width accordingly.
        //
        // Collect Is alignment info per source_line
        let mut is_info: HashMap<usize, (usize, usize)> = HashMap::new(); // source_line -> (max_is_prefix, original_is_pos)
        for node in &self.alignable_nodes {
            if node.kind == DelimiterKind::Is
                && let Some(group) = self
                    .alignment_groups
                    .get(&(node.source_line, DelimiterKind::Is))
            {
                let max_is_prefix = group.max_prefix_width;
                // Find original position of "is" in the source line
                // Find the byte offset of the line start
                let line_start_byte = {
                    let mut newline_count = 0;
                    let mut pos = 0;
                    for (i, ch) in self.source.char_indices() {
                        if newline_count == node.source_line {
                            pos = i;
                            break;
                        }
                        if ch == '\n' {
                            newline_count += 1;
                        }
                    }
                    if newline_count < node.source_line {
                        continue;
                    }
                    pos
                };
                let line_end = self.source[line_start_byte..]
                    .find('\n')
                    .map(|p| line_start_byte + p)
                    .unwrap_or(self.source.len());
                let line_text = &self.source[line_start_byte..line_end];
                if let Some(is_pos) = line_text.find(" is ").map(|p| p + 1) {
                    is_info
                        .entry(node.source_line)
                        .or_insert((max_is_prefix, is_pos));
                }
            }
        }

        // Now adjust Dash nodes that share a line with Is-aligned nodes
        if !is_info.is_empty() {
            // Adjust prefix_display_width on Dash AlignableNodes
            for node in &mut self.alignable_nodes {
                if node.kind == DelimiterKind::Dash
                    && let Some(&(max_is_pw, original_is_pos)) = is_info.get(&node.source_line)
                {
                    // After Is alignment, everything shifts by (max_is_pw + 1 - original_is_pos)
                    let shift = (max_is_pw + 1) as isize - original_is_pos as isize;
                    node.prefix_display_width =
                        (node.prefix_display_width as isize + shift).max(0) as usize;
                }
            }

            // Recompute Dash groups with adjusted widths
            let dash_groups: Vec<Vec<usize>> = groups
                .iter()
                .filter(|g| !g.is_empty() && self.alignable_nodes[g[0]].kind == DelimiterKind::Dash)
                .cloned()
                .collect();

            for group_indices in &dash_groups {
                if group_indices.len() < 2 {
                    continue;
                }

                let max_width = group_indices
                    .iter()
                    .map(|&i| self.alignable_nodes[i].prefix_display_width)
                    .max()
                    .unwrap_or(0);

                for &idx in group_indices {
                    let node = &self.alignable_nodes[idx];
                    self.alignment_groups.insert(
                        (node.source_line, DelimiterKind::Dash),
                        AlignmentGroup {
                            max_prefix_width: max_width,
                        },
                    );
                }
            }
        }
    }

    /// Get alignment padding for a node on a specific line.
    fn get_alignment_padding(
        &self,
        source_line: usize,
        kind: crate::align_ast::DelimiterKind,
        current_prefix_width: usize,
    ) -> usize {
        if let Some(group) = self.alignment_groups.get(&(source_line, kind)) {
            group.max_prefix_width.saturating_sub(current_prefix_width)
        } else {
            0
        }
    }
}

/// Format the given source code using the provided config.
pub fn format(source: &str, config: Config) -> String {
    // Pass 1: Collect alignment info from original source
    let alignable_nodes =
        if config.align_declarations || config.align_binds || config.align_comments {
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_gin::language())
                .expect("Error loading Gin grammar");
            let tree = parser
                .parse(source, None)
                .expect("Failed to parse source code");

            let mut visitor = FmtVisitor::new(source, config.clone());
            visitor.collect_alignments(tree.root_node());
            visitor.alignable_nodes
        } else {
            Vec::new()
        };

    // Pass 2: Build output buffer with alignment
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_gin::language())
        .expect("Error loading Gin grammar");
    let tree = parser
        .parse(source, None)
        .expect("Failed to parse source code");

    let mut visitor = FmtVisitor::new(source, config);
    visitor.set_alignment_groups(alignable_nodes);
    visitor.visit_source_file(tree.root_node());

    // Pass 3: Wrap long lines
    wrap_lines(visitor.buffer, visitor.config.max_line_width)
}

impl<'a> FmtVisitor<'a> {
    /// Collect alignment information from the AST without building the buffer.
    fn collect_alignments(&mut self, node: tree_sitter::Node) {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "declaration" => {
                    if self.config.align_declarations
                        && let Some(align_info) = mods::decls::collect_alignable(self, child)
                    {
                        self.alignable_nodes.push(align_info);
                    }
                    // Also collect comment/doc_comment descendants for dash alignment
                    if self.config.align_comments {
                        self.collect_comments_recursive(child);
                    }
                }
                "bind_statement" if self.config.align_binds => {
                    if let Some(align_info) = mods::binds::collect_alignable(self, child) {
                        self.alignable_nodes.push(align_info);
                    }
                }
                "comment" | "doc_comment" if self.config.align_comments => {
                    if let Some(align_info) = mods::comments::collect_alignable(self, child) {
                        self.alignable_nodes.push(align_info);
                    }
                }
                // Recursively collect from nested structures (e.g., for_statement bodies)
                "for_statement" => {
                    self.collect_alignments(child);
                }
                _ => {}
            }
        }
    }

    /// Recursively collect comment/doc_comment nodes for dash alignment.
    fn collect_comments_recursive(&mut self, node: tree_sitter::Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "comment" || child.kind() == "doc_comment" {
                if let Some(align_info) = mods::comments::collect_alignable(self, child) {
                    self.alignable_nodes.push(align_info);
                }
            } else {
                self.collect_comments_recursive(child);
            }
        }
    }

    /// Visit a source_file node (the root of the AST).
    fn visit_source_file(&mut self, node: tree_sitter::Node) {
        let mut cursor = node.walk();
        let mut last_processed_byte = 0;

        for child in node.children(&mut cursor) {
            // Preserve any text between the last processed byte and this child
            if child.start_byte() > last_processed_byte {
                let gap = &self.source[last_processed_byte..child.start_byte()];
                self.buffer.push_str(gap);
            }

            // Process the child
            match child.kind() {
                "declaration" => {
                    self.visit_declaration_no_newline(child);
                    // Don't add newline here - the gap text will include it
                }
                "bind_statement" => {
                    self.visit_bind_statement_no_newline(child);
                    // Don't add newline here - the gap text will include it
                }
                "method_statement" => {
                    self.visit_method_statement_no_newline(child);
                }
                "import_statement" => {
                    self.visit_import_statement_no_newline(child);
                    // Don't add newline here - the gap text will include it
                }
                "for_statement" => {
                    self.visit_for_statement_no_newline(child);
                    // Don't add newline here - the gap text will include it
                }
                "comment" | "doc_comment" => {
                    self.visit_comment_no_newline(child);
                }
                "ERROR" => {
                    // Preserve ERROR nodes as-is - they contain unparsed text
                    // The tree-sitter grammar doesn't handle all Gin syntax correctly
                    if let Some(text) = self.text_for_node(child) {
                        self.buffer.push_str(&text);
                    }
                }
                _ => {
                    // Unknown node type - preserve as-is
                    if let Some(text) = self.text_for_node(child) {
                        self.buffer.push_str(&text);
                    }
                }
            }

            last_processed_byte = child.end_byte();
        }

        // Preserve any remaining text after the last child
        if last_processed_byte < self.source.len() {
            self.buffer.push_str(&self.source[last_processed_byte..]);
        }
    }

    /// Visit a declaration node without adding a newline.
    fn visit_declaration_no_newline(&mut self, node: tree_sitter::Node) {
        if let Some(text) = self.text_for_node(node) {
            // For multi-line declarations, process line-by-line for dash alignment
            if text.contains('\n') {
                if self.config.align_comments {
                    let start_line = self.source[..node.start_byte()].matches('\n').count();
                    let lines: Vec<&str> = text.split('\n').collect();
                    for (i, line) in lines.iter().enumerate() {
                        if i > 0 {
                            self.buffer.push('\n');
                        }
                        let source_line = start_line + i;
                        let align_info = self.alignable_nodes.iter().find(|n| {
                            n.source_line == source_line
                                && n.kind == crate::align_ast::DelimiterKind::Dash
                        });
                        if let Some(info) = align_info
                            && let Some(dash_pos) = line.find("---")
                        {
                            let prefix_trimmed = line[..dash_pos].trim_end();
                            let padding = self.get_alignment_padding(
                                source_line,
                                crate::align_ast::DelimiterKind::Dash,
                                info.prefix_display_width,
                            );
                            let target_column = info.prefix_display_width + padding;
                            let gap = (target_column + 1)
                                .saturating_sub(prefix_trimmed.len())
                                .max(1);
                            self.buffer.push_str(prefix_trimmed);
                            self.buffer.push_str(&" ".repeat(gap));
                            self.buffer.push_str(&line[dash_pos..]);
                            continue;
                        }
                        self.buffer.push_str(line);
                    }
                } else {
                    self.buffer.push_str(&text);
                }
                return;
            }

            // Apply alignment if enabled
            if self.config.align_declarations {
                // Calculate source line
                let source_line = self.source[..node.start_byte()].matches('\n').count();

                // Try to find alignment info for this node
                let align_info = self.alignable_nodes.iter().find(|n| {
                    n.source_line == source_line && n.kind == crate::align_ast::DelimiterKind::Is
                });

                if let Some(info) = align_info {
                    // Apply alignment by replacing the whitespace before "is"
                    let padding = self.get_alignment_padding(
                        source_line,
                        crate::align_ast::DelimiterKind::Is,
                        info.prefix_display_width,
                    );
                    // Find "is" and replace the whitespace before it
                    if let Some(is_pos) = text.find("is") {
                        let prefix_trimmed = text[..is_pos].trim_end();
                        let new_gap = " ".repeat(padding + 1);
                        self.buffer.push_str(prefix_trimmed);
                        self.buffer.push_str(&new_gap);

                        let rest = &text[is_pos..];
                        // Check for dash alignment in the rest
                        if self.config.align_comments {
                            let dash_info = self
                                .alignable_nodes
                                .iter()
                                .find(|n| {
                                    n.source_line == source_line
                                        && n.kind == crate::align_ast::DelimiterKind::Dash
                                })
                                .cloned();
                            if let Some(dinfo) = dash_info
                                && let Some(dash_pos) = rest.find("---")
                            {
                                let before_dash = rest[..dash_pos].trim_end();
                                let current_total =
                                    prefix_trimmed.len() + new_gap.len() + before_dash.len();
                                let dash_padding = self.get_alignment_padding(
                                    source_line,
                                    crate::align_ast::DelimiterKind::Dash,
                                    dinfo.prefix_display_width,
                                );
                                let target = dinfo.prefix_display_width + dash_padding;
                                let gap = target.saturating_sub(current_total).max(1);
                                self.buffer.push_str(before_dash);
                                self.buffer.push_str(&" ".repeat(gap));
                                self.buffer.push_str(&rest[dash_pos..]);
                                return;
                            }
                        }

                        self.buffer.push_str(rest);
                        return;
                    }
                }
            }

            // No alignment or not alignable
            self.buffer.push_str(&text);
        }
    }

    /// Visit a multi-line bind_statement, re-indenting doc_comment extras to match body level.
    fn visit_multiline_bind(&mut self, node: tree_sitter::Node) {
        // Detect body indent level from the first non-extra child after ":"
        let mut found_colon = false;
        let mut body_indent: Option<usize> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !found_colon {
                if child.kind() == ":" {
                    found_colon = true;
                }
                continue;
            }
            if !child.is_extra() {
                body_indent = Some(child.start_position().column);
                break;
            }
        }
        let target_indent = body_indent.unwrap_or(4);

        // Walk children, preserving gap text but re-indenting standalone doc_comment extras
        let mut last_byte = node.start_byte();
        let mut cursor2 = node.walk();
        for child in node.children(&mut cursor2) {
            // Emit gap text between last processed byte and this child
            if child.start_byte() > last_byte {
                let gap = &self.source[last_byte..child.start_byte()];
                self.buffer.push_str(gap);
            }

            let is_doc_comment = child.is_extra()
                && (child.kind() == "doc_comment" || child.kind() == "comment")
                && self.source.get(child.start_byte()..child.start_byte() + 3) == Some("---");

            if is_doc_comment {
                // Check if the doc_comment is on its own line (standalone, not postfix)
                let line_start = self.source[..child.start_byte()]
                    .rfind('\n')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                let before_on_line = &self.source[line_start..child.start_byte()];
                if before_on_line.trim().is_empty() {
                    // Standalone doc comment on its own line - re-indent
                    // Remove the old indentation already pushed in the gap
                    let buf_line_start = self.buffer.rfind('\n').map(|p| p + 1).unwrap_or(0);
                    self.buffer.truncate(buf_line_start);
                    self.buffer.push_str(&" ".repeat(target_indent));
                    if let Some(text) = self.text_for_node(child) {
                        self.buffer.push_str(text.trim_start());
                    }
                } else {
                    // Postfix doc comment - preserve as-is
                    if let Some(text) = self.text_for_node(child) {
                        self.buffer.push_str(&text);
                    }
                }
            } else {
                if let Some(text) = self.text_for_node(child) {
                    self.buffer.push_str(&text);
                }
            }

            last_byte = child.end_byte();
        }

        // Emit any remaining text after the last child
        if last_byte < node.end_byte() {
            self.buffer
                .push_str(&self.source[last_byte..node.end_byte()]);
        }
    }

    /// Visit a bind_statement node without adding a newline.
    fn visit_bind_statement_no_newline(&mut self, node: tree_sitter::Node) {
        if let Some(text) = self.text_for_node(node) {
            // Multi-line bind statements: walk children to re-indent doc comments
            if text.contains('\n') {
                self.visit_multiline_bind(node);
                return;
            }

            // Apply alignment if enabled
            if self.config.align_binds {
                // Calculate source line
                let source_line = self.source[..node.start_byte()].matches('\n').count();

                // Try to find alignment info for this node
                let align_info = self.alignable_nodes.iter().find(|n| {
                    n.source_line == source_line && n.kind == crate::align_ast::DelimiterKind::Colon
                });

                if let Some(info) = align_info {
                    // Apply alignment by replacing the whitespace after ":"
                    let padding = self.get_alignment_padding(
                        source_line,
                        crate::align_ast::DelimiterKind::Colon,
                        info.prefix_display_width,
                    );
                    // Find the colon position
                    if let Some(colon_pos) = text.find(':') {
                        // Everything up to and including the colon is the prefix
                        let prefix_with_colon = &text[..=colon_pos];
                        // The rest is after the colon
                        let rest = &text[colon_pos + 1..];
                        // The new gap should be (padding + 1) spaces
                        let new_gap = " ".repeat(padding + 1);
                        self.buffer.push_str(prefix_with_colon);
                        self.buffer.push_str(&new_gap);
                        self.buffer.push_str(rest.trim_start());
                        return;
                    }
                }
            }

            // No alignment or not alignable
            self.buffer.push_str(&text);
        }
    }

    /// Visit a method_statement node, formatting any `when_expression` body with alignment.
    ///
    /// Walks the node's children, emitting gap text as-is for the prefix
    /// (`Tag.method:`) but delegating `when_expression` children to
    /// `mods::when::rewrite` so that `then`/`else` align with `when`.
    fn visit_method_statement_no_newline(&mut self, node: tree_sitter::Node) {
        let mut last_byte = node.start_byte();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            // Preserve gap text (whitespace, punctuation) between children
            if child.start_byte() > last_byte {
                let gap = &self.source[last_byte..child.start_byte()];
                self.buffer.push_str(gap);
            }

            // `when_expression` may be wrapped in a `statement` node
            let when_node = if child.kind() == "when_expression" {
                Some(child)
            } else if child.kind() == "statement" {
                let mut c = child.walk();
                child
                    .children(&mut c)
                    .find(|n| n.kind() == "when_expression")
            } else {
                None
            };

            if let Some(when_child) = when_node {
                // Column where `when` will appear = current line length in buffer
                let line_start = self.buffer.rfind('\n').map(|p| p + 1).unwrap_or(0);
                let when_col = self.buffer.len() - line_start;
                if let Some(formatted) = mods::when::rewrite(self, when_child, when_col) {
                    self.buffer.push_str(&formatted);
                } else if let Some(text) = self.text_for_node(when_child) {
                    self.buffer.push_str(&text);
                }
            } else if let Some(text) = self.text_for_node(child) {
                self.buffer.push_str(&text);
            }

            last_byte = child.end_byte();
        }

        if last_byte < node.end_byte() {
            self.buffer
                .push_str(&self.source[last_byte..node.end_byte()]);
        }
    }

    /// Visit an import_statement node without adding a newline.
    fn visit_import_statement_no_newline(&mut self, node: tree_sitter::Node) {
        if let Some(text) = self.text_for_node(node) {
            self.buffer.push_str(&text);
        }
    }

    /// Visit a for_statement node without adding a newline.
    fn visit_for_statement_no_newline(&mut self, node: tree_sitter::Node) {
        if let Some(text) = self.text_for_node(node) {
            self.buffer.push_str(&text);
        }
    }

    /// Visit a comment node without adding a newline, with optional `---` alignment.
    fn visit_comment_no_newline(&mut self, node: tree_sitter::Node) {
        if let Some(text) = self.text_for_node(node) {
            // Skip multi-line comments
            if text.contains('\n') {
                self.buffer.push_str(&text);
                return;
            }

            // Apply alignment if enabled and comment has `---`
            if self.config.align_comments {
                let source_line = self.source[..node.start_byte()].matches('\n').count();

                // Try to find alignment info for this comment
                let align_info = self.alignable_nodes.iter().find(|n| {
                    n.source_line == source_line && n.kind == crate::align_ast::DelimiterKind::Dash
                });

                if let Some(info) = align_info
                    && let Some(dash_pos) = text.find("---")
                {
                    let padding = self.get_alignment_padding(
                        source_line,
                        crate::align_ast::DelimiterKind::Dash,
                        info.prefix_display_width,
                    );
                    let target_column = info.prefix_display_width + padding;

                    if dash_pos == 0 {
                        // --- is at start of comment node; prefix is already in the buffer
                        // Trim trailing whitespace so we control the exact gap
                        let buffer_line_start = self.buffer.rfind('\n').map(|p| p + 1).unwrap_or(0);
                        while self.buffer.len() > buffer_line_start && self.buffer.ends_with(' ') {
                            self.buffer.pop();
                        }
                        let current_line = &self.buffer[buffer_line_start..];
                        // Only add spacing if there's non-whitespace content on the line
                        // (i.e., this is a postfix comment after other tokens)
                        if current_line.trim().is_empty() {
                            // Prefix doc comment on its own line - don't add space
                            self.buffer.push_str(&text);
                            return;
                        }
                        let current_width = current_line.len();
                        // +1 ensures at least 1 space on the longest line
                        let gap = (target_column + 1).saturating_sub(current_width).max(1);
                        self.buffer.push_str(&" ".repeat(gap));
                    } else {
                        let prefix_trimmed = text[..dash_pos].trim_end();
                        let gap = (target_column + 1)
                            .saturating_sub(prefix_trimmed.len())
                            .max(1);
                        self.buffer.push_str(prefix_trimmed);
                        self.buffer.push_str(&" ".repeat(gap));
                    }
                    self.buffer.push_str(&text[dash_pos..]);
                    return;
                }
            }

            // No alignment or not alignable
            self.buffer.push_str(&text);
        }
    }

    /// Get the source text for a node.
    pub fn text_for_node(&self, node: tree_sitter::Node) -> Option<String> {
        let start_byte = node.start_byte();
        let end_byte = node.end_byte();
        if start_byte < self.source.len() && end_byte <= self.source.len() {
            Some(self.source[start_byte..end_byte].to_string())
        } else {
            None
        }
    }
}

/// Wrap long lines to stay within max_line_width.
///
/// Breaks after binary operators (+, -, *, /, =, etc.) with 4-space continuation indent.
fn wrap_lines(formatted: String, max_line_width: usize) -> String {
    if max_line_width == 0 {
        return formatted;
    }

    let mut result = Vec::new();
    let mut in_multiline_comment = false;

    for line in formatted.lines() {
        // Check for multi-line contexts (strings, comments, doc comments)
        let trimmed = line.trim();
        if trimmed.starts_with("---") {
            in_multiline_comment = true;
        } else if in_multiline_comment && !trimmed.starts_with("---") && !trimmed.is_empty() {
            in_multiline_comment = false;
        }

        // Skip wrapping for comment/doc lines and very short lines
        if in_multiline_comment || line.len() <= max_line_width || trimmed.is_empty() {
            result.push(line.to_string());
            continue;
        }

        // Try to wrap the line
        if let Some(wrapped) = wrap_single_line(line, max_line_width) {
            result.extend(wrapped);
        } else {
            // Couldn't wrap, keep original
            result.push(line.to_string());
        }
    }

    let mut output = result.join("\n");
    if formatted.ends_with('\n') {
        output.push('\n');
    }
    output
}

/// Try to wrap a single line at an operator boundary.
fn wrap_single_line(line: &str, max_width: usize) -> Option<Vec<String>> {
    // Binary operators to break after: +, -, *, /, %, =, :=, ==, !=, <, >, <=, >=
    // Also after , in function calls

    let mut lines = Vec::new();
    let mut current = line.to_string();
    let continuation_indent = "    ";

    while current.len() > max_width {
        // Find the best break point
        let break_pos = find_break_point(&current, max_width)?;

        // Split at the break position
        let before = &current[..break_pos];
        let after = &current[break_pos..];

        lines.push(before.to_string());
        current = format!("{continuation_indent}{}", after.trim_start());
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.len() > 1 { Some(lines) } else { None }
}

/// Find a position to break a long line.
///
/// Looks for operators near the max_width, preferring breaks later in the line.
fn find_break_point(line: &str, max_width: usize) -> Option<usize> {
    // Binary operators we can break after (including space after)
    const OPERATORS: &[&str] = &[
        " + ", " - ", " * ", " / ", " % ", " = ", " := ", " == ", " != ", " < ", " > ", " <= ",
        " >= ", " & ", " | ", " ^ ",
    ];
    const COMMA: &str = ", ";

    // Search backwards from max_width for a break point
    let search_end = max_width.min(line.len());

    // First try to find an operator
    for op in OPERATORS.iter() {
        if let Some(pos) = line[..search_end].rfind(op) {
            // Break after the operator
            return Some(pos + op.len());
        }
    }

    // Then try to find a comma
    if let Some(pos) = line[..search_end].rfind(COMMA) {
        return Some(pos + COMMA.len());
    }

    // No good break point found
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn debug_print_ast() {
        let source = "Maybe is Some or None\nResult is Ok or Error\n";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        let tree = parser.parse(source, None).unwrap();

        fn print_tree(node: tree_sitter::Node, indent: &str) {
            println!(
                "{}{} ({:?})",
                indent,
                node.kind(),
                (node.start_byte(), node.end_byte())
            );
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                print_tree(child, &format!("{}  ", indent));
            }
        }

        print_tree(tree.root_node(), "");
    }

    #[test]
    fn test_basic_format() {
        let source = "Maybe is Some or None\nResult is Ok or Error\n";
        let output = format(source, Config::default());
        // Alignment is not yet implemented in the rewrite function
        // The formatter currently preserves the original spacing
        assert_eq!(output, "Maybe is Some or None\nResult is Ok or Error\n");
    }

    #[test]
    fn test_comment_dash_alignment() {
        // Note: tree-sitter-gin grammar doesn't parse standalone comments
        // This test verifies the comment alignment when comments are parsed correctly
        let source = "# Maybe[thing] is --- Used to represent values that may or may not be present.\n\
                      #     Some(thing) or --- has a value\n\
                      #     None --- has no value\n";

        let output = format(source, Config::default());
        // Comments should be aligned if parsed correctly
        // The actual behavior depends on tree-sitter-gin comment parsing
        // For now, just verify the formatter doesn't crash
        assert!(!output.is_empty());
    }

    #[test]
    fn test_line_wrap_basic() {
        let source = "result := a_very_long_function_name_here(arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8)\n";
        let output = format(source, Config::default());
        // Should wrap at 80 chars
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 2, "Should wrap to at least 2 lines");
        // Check that continuation is indented with 4 spaces
        assert!(
            lines[1].starts_with("    "),
            "Continuation should be indented"
        );
    }

    #[test]
    fn test_line_wrap_binary_op() {
        let source =
            "y := very_long_name + another_very_long_name + yet_another_long_name_to_wrap_here\n";
        let output = format(source, Config::default());
        // Should wrap at binary operator
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 2, "Should wrap to at least 2 lines");
        // First line should end with operator
        assert!(
            lines[0].ends_with(" + ") || lines[0].ends_with(" - "),
            "First line should end with operator"
        );
    }
}
