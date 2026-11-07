use crate::{
    ArrayExpression, CollectionPunctuation as Punctuation, LoopExpression, LoopKeyword,
    NumericSelectExpression, RepeatExpression, SelectExpression, SequenceDelimiters,
    SequenceExpression,
};

use super::*;

impl<'syntax> Lookup<'syntax> {
    pub(super) fn sequence_children(
        &mut self,
        index: usize,
        sequence: &'syntax SequenceExpression,
    ) {
        if let SequenceDelimiters::Parenthesized { open, .. } = sequence.delimiters {
            self.punctuation(index, Punctuation::Open, open);
        }
        for element in &sequence.elements {
            self.expression(index, element);
        }
        for separator in &sequence.separators {
            self.punctuation(index, Punctuation::Comma, *separator);
        }
        if let SequenceDelimiters::Parenthesized { close, .. } = sequence.delimiters {
            self.punctuation(index, Punctuation::Close, close);
        }
    }

    pub(super) fn numeric_select_children(
        &mut self,
        index: usize,
        select: &'syntax NumericSelectExpression,
    ) {
        self.expression(index, &select.base);
        self.punctuation(index, Punctuation::Dot, select.dot);
        self.expression(index, &select.index);
    }

    pub(super) fn array_children(&mut self, index: usize, array: &'syntax ArrayExpression) {
        self.punctuation(index, Punctuation::Open, array.open);
        for (element, label) in array.elements.iter().zip(&array.labels) {
            if let Some(label) = label {
                self.name(index, label);
            }
            self.expression(index, element);
        }
        for separator in &array.separators {
            self.punctuation(index, Punctuation::Comma, *separator);
        }
        if let Some(length) = &array.length {
            self.punctuation(index, Punctuation::Semicolon, length.separator);
            self.expression(index, &length.value);
        }
        self.punctuation(index, Punctuation::Close, array.close);
    }

    pub(super) fn repeat_children(&mut self, index: usize, repeat: &'syntax RepeatExpression) {
        self.punctuation(index, Punctuation::Open, repeat.open);
        self.expression(index, &repeat.value);
        self.punctuation(index, Punctuation::Semicolon, repeat.separator);
        self.expression(index, &repeat.length);
        self.punctuation(index, Punctuation::Close, repeat.close);
    }

    pub(super) fn select_children(&mut self, index: usize, select: &'syntax SelectExpression) {
        self.expression(index, &select.base);
        self.punctuation(index, Punctuation::Dot, select.dot);
        self.punctuation(index, Punctuation::Open, select.open);
        self.expression(index, &select.index);
        if let Some(length) = &select.length {
            self.punctuation(index, Punctuation::Semicolon, length.separator);
            self.expression(index, &length.value);
        }
        self.punctuation(index, Punctuation::Close, select.close);
    }

    pub(super) fn loop_children(&mut self, index: usize, loop_expression: &'syntax LoopExpression) {
        match loop_expression {
            LoopExpression::For {
                for_keyword,
                binder,
                in_keyword,
                iterable,
                invariant,
                do_keyword,
                body,
            } => {
                self.add(index, Kind::LoopKeyword(LoopKeyword::For), *for_keyword);
                self.add(index, Kind::LoopBinder(binder), binder.span_id);
                self.add(index, Kind::LoopKeyword(LoopKeyword::In), *in_keyword);
                self.expression(index, iterable);
                if let Some(invariant) = invariant {
                    self.add(index, Kind::LoopKeyword(LoopKeyword::Where), invariant.where_keyword);
                    self.expression(index, &invariant.condition);
                }
                self.add(index, Kind::LoopKeyword(LoopKeyword::Do), *do_keyword);
                self.expression(index, body);
            }
            LoopExpression::While {
                while_keyword,
                condition,
                invariant,
                do_keyword,
                body,
            } => {
                self.add(index, Kind::LoopKeyword(LoopKeyword::While), *while_keyword);
                self.expression(index, condition);
                if let Some(invariant) = invariant {
                    self.add(index, Kind::LoopKeyword(LoopKeyword::Where), invariant.where_keyword);
                    self.expression(index, &invariant.condition);
                }
                self.add(index, Kind::LoopKeyword(LoopKeyword::Do), *do_keyword);
                self.expression(index, body);
            }
        }
    }

    fn punctuation(&mut self, index: usize, punctuation: Punctuation, span: SpanId) {
        self.add(index, Kind::CollectionPunctuation(punctuation), span);
    }
}
