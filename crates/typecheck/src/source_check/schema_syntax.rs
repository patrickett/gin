//! Layout authority depends on retained fields faithfully covering written source.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn file_syntax(&mut self) -> Result<()> {
        self.charge(self.item_count())?;
        let files = self.files;
        for (index, file) in files.iter().enumerate() {
            let mut cursor = 0;
            let mut previous_directive = false;
            for item in &file.items {
                self.span(item.span_id)?;
                let range = self.range_of(item.span_id);
                let directive = matches!(
                    item.value,
                    Item::Entry | Item::Load | Item::Store | Item::Wrap | Item::Unrestricted | Item::Instruction { .. }
                );
                // `#a, #b` on one line is the same two directives as on two lines.
                let comma = previous_directive
                    && directive
                    && self
                        .gap_text(range.file, cursor, range.start)?
                        .trim_matches([' ', '\t'])
                        == ",";
                if !comma {
                    self.syntax_trivia(range.file, cursor, range.start)?;
                }
                previous_directive = directive;
                cursor = range.end;
            }
            self.syntax_trivia_to_eof(index as u32, cursor)?;
        }
        Ok(())
    }

    pub(super) fn declaration_syntax(
        &mut self,
        parent: SpanId,
        declaration: &'a TypeDeclaration,
    ) -> Result<()> {
        let range = self.range_of(parent);
        let name = self.range_of(declaration.name.span_id);
        let body = self.range_of(declaration.body.span_id);
        if range.start != name.start || range.end != body.end {
            return Err(SourceCheckError::Malformed);
        }
        let mut cursor = name.end;
        if let Some(parameters) = &declaration.parameters {
            let parameters = self.range_of(parameters.span_id);
            self.syntax_trivia(parameters.file, cursor, parameters.start)?;
            cursor = parameters.end;
        }
        self.syntax_trivia(body.file, cursor, body.start)
    }

    pub(super) fn field_syntax(&mut self, parent: SpanId, field: &'a Binding) -> Result<()> {
        self.validate_field(field)?;
        let range = self.range_of(parent);
        let mut cursor = range.start;
        self.charge(field.attributes.len())?;
        for attribute in &field.attributes {
            self.child(parent, attribute.span_id)?;
            let part = self.range_of(attribute.span_id);
            self.syntax_trivia(part.file, cursor, part.start)?;
            // Fields take `#private` and `#register`; `#infix` marks methods.
            let (hash, keyword, end) = match attribute.value {
                FieldAttribute::Private {
                    hash,
                    private_keyword,
                } => (hash, private_keyword, private_keyword),
                FieldAttribute::Register {
                    hash,
                    keyword,
                    close,
                    ..
                } => (hash, keyword, close),
                FieldAttribute::Infix { .. } | FieldAttribute::Inline { .. } => {
                    return Err(SourceCheckError::Unsupported);
                }
            };
            let hash = self.range_of(hash);
            let keyword = self.range_of(keyword);
            if part.start != hash.start || part.end != self.range_of(end).end {
                return Err(SourceCheckError::Malformed);
            }
            self.syntax_trivia(keyword.file, hash.end, keyword.start)?;
            cursor = part.end;
        }
        self.child(parent, field.name.span_id)?;
        let name = self.range_of(field.name.span_id);
        self.syntax_trivia(name.file, cursor, name.start)?;
        cursor = name.end;
        if let Some(annotation) = &field.annotation {
            self.child(parent, annotation.span_id)?;
            self.schema_annotation_syntax(annotation)?;
            let annotation = self.range_of(annotation.span_id);
            self.syntax_trivia(annotation.file, cursor, annotation.start)?;
            cursor = annotation.end;
        }
        // `Type or Other`: each further variant of a result, with the `or` before it.
        for alternative in &field.alternatives {
            self.child(parent, alternative.span_id)?;
            let part = self.range_of(alternative.span_id);
            self.syntax_trivia(part.file, cursor, part.start)?;
            cursor = part.end;
        }
        self.syntax_trivia(range.file, cursor, range.end)
    }

    /// Has members are separated by a comma or a newline, never hidden syntax.
    /// Comments are lexer trivia; their commas and attribute-looking text are inert.
    pub(super) fn field_gap(
        &mut self,
        file: u32,
        start: usize,
        end: usize,
        between: bool,
    ) -> Result<()> {
        let text = self.gap_text(file, start, end)?;
        self.charge(text.len())?;
        let bytes = text.as_bytes();
        let mut index = 0;
        let mut comment = false;
        let mut comma = false;
        let mut newline = false;
        while index < bytes.len() {
            if comment {
                if bytes[index] == b'\n' {
                    comment = false;
                    newline = true;
                }
                index += 1;
            } else {
                match bytes[index] {
                    b' ' | b'\t' => index += 1,
                    b'\n' => {
                        newline = true;
                        index += 1;
                    }
                    b'-' if bytes.get(index + 1) == Some(&b'-') => {
                        comment = true;
                        index += 2;
                    }
                    b',' if between && !comma => {
                        comma = true;
                        index += 1;
                    }
                    _ => return Err(SourceCheckError::Malformed),
                }
            }
        }
        if comment || (between && !comma && !newline) {
            return Err(SourceCheckError::Malformed);
        }
        Ok(())
    }
}
