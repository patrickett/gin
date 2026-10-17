use crate::RuntimeStorage;
use ast::{ConstValue, NormalExpr, ty::Ty};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedValue {
    pub storage: RuntimeStorage,
    pub gin_type: Option<Ty>,
}

impl EvaluatedValue {
    pub fn from_constant(value: ConstValue) -> Self {
        Self::new(value, None)
    }

    pub fn new(value: ConstValue, gin_type: Option<Ty>) -> Self {
        let storage = match value {
            value @ ConstValue::Int(_) => match value
                .as_const_size_int()
                .and_then(|value| i64::try_from(value).ok())
            {
                Some(value) => RuntimeStorage::Int64(value),
                None => RuntimeStorage::Constant(value),
            },
            ConstValue::Text(value) => RuntimeStorage::Bytes(value.into_bytes()),
            value => RuntimeStorage::Constant(value),
        };
        Self { storage, gin_type }
    }

    pub fn display(&self) -> String {
        self.inspection(InspectionLimits::default()).display
    }

    pub fn inspection(&self, limits: InspectionLimits) -> ValueInspection {
        let limits = limits.bounded();
        let mut budget = TextBudget::new(limits.text_bytes_max / 2);
        let (type_name, type_truncated) = type_summary(self.gin_type.as_ref(), &mut budget);
        let root = match &self.storage {
            RuntimeStorage::Int64(value) => Input::Integer(*value),
            RuntimeStorage::Bytes(bytes) => text_input(bytes, budget.limit - budget.used),
            RuntimeStorage::Constant(value) => Input::Constant(value),
        };
        let unit = matches!(self.gin_type, Some(Ty::Unit));
        let mut nodes = inspect_nodes(root, unit, limits, &mut budget);
        let type_name = type_name.unwrap_or_default();
        let display_limit = limits.text_bytes_max - budget.used;
        let (display, display_truncated) = display_nodes(&nodes, display_limit);
        let truncated =
            type_truncated || display_truncated || nodes.iter().any(|node| node.truncated);
        if display_truncated {
            nodes[0].truncated = true;
        }
        ValueInspection {
            type_name,
            type_truncated,
            display,
            nodes,
            truncated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectionLimits {
    pub depth_max: u32,
    pub nodes_max: u32,
    pub text_bytes_max: u32,
}

impl Default for InspectionLimits {
    fn default() -> Self {
        Self {
            depth_max: 8,
            nodes_max: 256,
            text_bytes_max: 8192,
        }
    }
}

impl InspectionLimits {
    fn bounded(self) -> Self {
        Self {
            depth_max: self.depth_max.min(32),
            nodes_max: self.nodes_max.clamp(1, 4096),
            text_bytes_max: self.text_bytes_max.min(65_536),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueInspection {
    pub type_name: String,
    pub type_truncated: bool,
    pub display: String,
    pub nodes: Vec<ValueNode>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueNode {
    pub kind: ValueKind,
    pub value: Option<String>,
    pub children: Vec<ValueChild>,
    pub total_children: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueChild {
    pub name: Option<String>,
    pub node: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Unit,
    Proposition,
    Integer,
    Float,
    Text,
    Bytes,
    Tag,
    ResultAlternative,
    Record,
    ResultRow,
    FixedArray,
    Sequence,
    SequenceRepeat,
    Domain,
    Expression,
}

#[derive(Clone, Copy)]
enum Input<'a> {
    Constant(&'a ConstValue),
    Expression(&'a NormalExpr),
    Integer(i64),
    Text(&'a str, bool),
    Bytes(&'a [u8]),
}

fn text_input(bytes: &[u8], bytes_max: u32) -> Input<'_> {
    let prefix = &bytes[..bytes.len().min(bytes_max as usize)];
    match std::str::from_utf8(prefix) {
        Ok(text) => Input::Text(text, prefix.len() < bytes.len()),
        Err(error) if error.error_len().is_none() && prefix.len() < bytes.len() => {
            let text = std::str::from_utf8(&prefix[..error.valid_up_to()])
                .expect("UTF-8 error identifies a valid prefix");
            Input::Text(text, true)
        }
        Err(_) => Input::Bytes(bytes),
    }
}

struct TextBudget {
    limit: u32,
    used: u32,
}

impl TextBudget {
    fn new(limit: u32) -> Self {
        Self { limit, used: 0 }
    }

    fn text(&mut self, text: &str) -> (String, bool) {
        let available = (self.limit - self.used) as usize;
        let mut end = text.len().min(available);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.used += end as u32;
        (text[..end].to_owned(), end < text.len())
    }

    fn scalar(&mut self, text: String) -> (Option<String>, bool) {
        if text.len() <= (self.limit - self.used) as usize {
            self.used += text.len() as u32;
            (Some(text), false)
        } else {
            (None, true)
        }
    }
}

fn inspect_nodes(
    root: Input<'_>,
    unit: bool,
    limits: InspectionLimits,
    budget: &mut TextBudget,
) -> Vec<ValueNode> {
    let mut inputs = vec![(root, 0)];
    let mut nodes = Vec::new();
    for index in 0..limits.nodes_max as usize {
        let Some(&(input, depth)) = inputs.get(index) else {
            break;
        };
        let mut node = input_node(input, budget);
        if index == 0 && unit {
            node.kind = ValueKind::Unit;
        }
        let total = child_count(input);
        node.total_children = total as u64;
        let remaining = limits.nodes_max as usize - inputs.len();
        let count = if depth < limits.depth_max {
            total.min(remaining)
        } else {
            0
        };
        for child_index in 0..count {
            let (name, input) = child_at(input, child_index);
            let name = name.map(|name| {
                let (name, truncated) = budget.text(name);
                node.truncated |= truncated;
                name
            });
            node.children.push(ValueChild {
                name,
                node: inputs.len() as u32,
            });
            inputs.push((input, depth + 1));
        }
        node.truncated |= count < total;
        nodes.push(node);
    }
    assert!(!nodes.is_empty());
    assert!(nodes.len() <= limits.nodes_max as usize);
    nodes
}

fn input_node(input: Input<'_>, budget: &mut TextBudget) -> ValueNode {
    let mut node = ValueNode {
        kind: ValueKind::Expression,
        value: None,
        children: Vec::new(),
        total_children: 0,
        truncated: false,
    };
    match input {
        Input::Integer(value) => {
            scalar_node(&mut node, ValueKind::Integer, value.to_string(), budget)
        }
        Input::Text(value, truncated) => {
            text_node(&mut node, ValueKind::Text, value, budget);
            node.truncated |= truncated;
        }
        Input::Bytes(value) => {
            scalar_node(
                &mut node,
                ValueKind::Bytes,
                format!("{} bytes", value.len()),
                budget,
            );
            node.truncated = !value.is_empty();
        }
        Input::Constant(value) => constant_node(&mut node, value, budget),
        Input::Expression(value) => expression_node(&mut node, value, budget),
    }
    node
}

fn scalar_node(node: &mut ValueNode, kind: ValueKind, value: String, budget: &mut TextBudget) {
    node.kind = kind;
    (node.value, node.truncated) = budget.scalar(value);
}

fn text_node(node: &mut ValueNode, kind: ValueKind, value: &str, budget: &mut TextBudget) {
    node.kind = kind;
    let (text, truncated) = budget.text(value);
    node.value = Some(text);
    node.truncated = truncated;
}

fn constant_node(node: &mut ValueNode, value: &ConstValue, budget: &mut TextBudget) {
    match value {
        ConstValue::Proposition(value) => {
            scalar_node(node, ValueKind::Proposition, value.to_string(), budget)
        }
        ConstValue::Int(value) => scalar_node(node, ValueKind::Integer, value.to_string(), budget),
        ConstValue::Float(ast::HashFloat(value)) => {
            scalar_node(node, ValueKind::Float, value.to_string(), budget)
        }
        ConstValue::Text(value) => text_node(node, ValueKind::Text, value, budget),
        ConstValue::Tag {
            name, qual_path, ..
        } => {
            tag_node(node, name.as_str(), qual_path.as_deref(), budget);
        }
        ConstValue::ResultAlternative { label, .. } => {
            text_node(node, ValueKind::ResultAlternative, label.as_str(), budget);
            node.truncated = true;
        }
        ConstValue::Record { .. } => node.kind = ValueKind::Record,
        ConstValue::ResultRow(_) => node.kind = ValueKind::ResultRow,
        ConstValue::FixedArray(_) => node.kind = ValueKind::FixedArray,
        ConstValue::Sequence(_) => node.kind = ValueKind::Sequence,
        ConstValue::SequenceRepeat { .. } => node.kind = ValueKind::SequenceRepeat,
        ConstValue::Domain(domain) => {
            let summary = match domain.storage_hull() {
                Some(hull) => format!("integer domain in {} to {}", hull.min(), hull.max()),
                None => "integer domain".to_owned(),
            };
            scalar_node(node, ValueKind::Domain, summary, budget);
            node.truncated = true;
        }
    }
}

fn tag_node(node: &mut ValueNode, name: &str, path: Option<&str>, budget: &mut TextBudget) {
    node.kind = ValueKind::Tag;
    let mut qualified = String::new();
    if let Some(path) = path {
        let (prefix, truncated) = budget.text(path);
        qualified.push_str(&prefix);
        node.truncated = truncated;
        if !node.truncated {
            let (separator, truncated) = budget.text(".");
            qualified.push_str(&separator);
            node.truncated = truncated;
        }
    }
    if !node.truncated {
        let (name, truncated) = budget.text(name);
        qualified.push_str(&name);
        node.truncated = truncated;
    }
    node.value = Some(qualified);
}

fn expression_node(node: &mut ValueNode, value: &NormalExpr, budget: &mut TextBudget) {
    match value {
        NormalExpr::Value(value) => constant_node(node, value, budget),
        NormalExpr::Var(name) => text_node(node, ValueKind::Expression, name.as_str(), budget),
        NormalExpr::Inferred(_) => {
            text_node(node, ValueKind::Expression, "?", budget);
            node.truncated = true;
        }
        NormalExpr::Add(..) => text_node(node, ValueKind::Expression, "+", budget),
        NormalExpr::Sub(..) => text_node(node, ValueKind::Expression, "-", budget),
        NormalExpr::Mul(..) => text_node(node, ValueKind::Expression, "*", budget),
        NormalExpr::TargetQuery { kind, .. } => {
            scalar_node(node, ValueKind::Expression, format!("#{kind}"), budget);
            node.truncated = true;
        }
    }
}

fn child_count(input: Input<'_>) -> usize {
    let input = unwrap_expression(input);
    match input {
        Input::Constant(ConstValue::Tag { args, .. }) => args.len(),
        Input::Constant(ConstValue::Record { fields }) => fields.len(),
        Input::Constant(
            ConstValue::ResultRow(items)
            | ConstValue::FixedArray(items)
            | ConstValue::Sequence(items),
        ) => items.len(),
        Input::Constant(ConstValue::SequenceRepeat { .. }) => 2,
        Input::Expression(NormalExpr::Add(..) | NormalExpr::Sub(..) | NormalExpr::Mul(..)) => 2,
        _ => 0,
    }
}

fn child_at(input: Input<'_>, index: usize) -> (Option<&str>, Input<'_>) {
    let input = unwrap_expression(input);
    match input {
        Input::Constant(ConstValue::Tag { args, .. }) => (None, Input::Constant(&args[index])),
        Input::Constant(ConstValue::Record { fields }) => {
            let (name, value) = &fields[index];
            (Some(name.as_str()), Input::Constant(value))
        }
        Input::Constant(
            ConstValue::ResultRow(items)
            | ConstValue::FixedArray(items)
            | ConstValue::Sequence(items),
        ) => (None, Input::Constant(&items[index])),
        Input::Constant(ConstValue::SequenceRepeat { seed, count }) => match index {
            0 => (Some("seed"), Input::Constant(seed)),
            _ => (Some("count"), Input::Expression(count)),
        },
        Input::Expression(
            NormalExpr::Add(left, right)
            | NormalExpr::Sub(left, right)
            | NormalExpr::Mul(left, right),
        ) => match index {
            0 => (None, Input::Expression(left)),
            _ => (None, Input::Expression(right)),
        },
        _ => unreachable!("child index comes from child_count"),
    }
}

fn unwrap_expression(input: Input<'_>) -> Input<'_> {
    match input {
        Input::Expression(NormalExpr::Value(value)) => Input::Constant(value),
        input => input,
    }
}

fn type_summary(ty: Option<&Ty>, budget: &mut TextBudget) -> (Option<String>, bool) {
    let Some(ty) = ty else { return (None, false) };
    let (summary, incomplete) = match ty {
        Ty::Named { name, instance } => (name.as_str(), !instance.arguments.is_empty()),
        Ty::Opaque(name) => (name.as_str(), false),
        Ty::Record { name, .. } | Ty::Union { name, .. } => (name.as_str(), true),
        Ty::AnonymousInteger { validity } => {
            let summary = match validity.domain().storage_hull() {
                Some(hull) => format!("integer in {} to {}", hull.min(), hull.max()),
                None => "integer".to_owned(),
            };
            let (summary, truncated) = budget.scalar(summary);
            let exact_hull = matches!(
                validity.domain().predicate(),
                ast::integer::CanonicalPredicate::Hull(_)
            );
            return (summary, truncated || !exact_hull);
        }
        Ty::Unit => ("unit", false),
        Ty::Float { value } => ("float", value.is_some()),
        Ty::Proposition(_) => ("proposition", true),
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer) => ("integer literal", false),
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Text) => ("text literal", false),
        Ty::Literal(_) => ("literal", true),
        Ty::ResultFamily { .. } => ("result family", true),
        Ty::UnresolvedSequence { .. } | Ty::Sequence(_) => ("sequence", true),
        Ty::Array { .. } => ("array", true),
        Ty::Ptr { .. } => ("pointer", true),
        Ty::OwnedStorage { .. } => ("owned storage", true),
        Ty::Address { .. } => ("address", true),
        Ty::Ref { mutable: true, .. } => ("mutable reference", true),
        Ty::Ref { .. } => ("reference", true),
    };
    let (summary, truncated) = budget.text(summary);
    (Some(summary), incomplete || truncated)
}

enum DisplayPart<'a> {
    Node(u32),
    Text(&'a str),
    Quoted(&'a str),
}

fn display_nodes(nodes: &[ValueNode], bytes_max: u32) -> (String, bool) {
    let mut output = String::new();
    let mut parts = vec![DisplayPart::Node(0)];
    let mut truncated = false;
    for _ in 0..nodes.len().saturating_mul(8).saturating_add(1) {
        let Some(part) = parts.pop() else { break };
        match part {
            DisplayPart::Node(index) => {
                push_node_display(&mut parts, &nodes[index as usize], index)
            }
            DisplayPart::Text(text) => {
                if !append_text(&mut output, text, bytes_max as usize) {
                    truncated = true;
                    break;
                }
            }
            DisplayPart::Quoted(text) => {
                if !append_quoted(&mut output, text, bytes_max as usize) {
                    truncated = true;
                    break;
                }
            }
        }
    }
    truncated |= !parts.is_empty();
    if truncated && bytes_max >= 3 {
        let mut end = output.len().min(bytes_max as usize - 3);
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push('…');
    }
    (output, truncated)
}

fn push_node_display<'a>(parts: &mut Vec<DisplayPart<'a>>, node: &'a ValueNode, index: u32) {
    let value = node.value.as_deref().unwrap_or("…");
    let delimiters = match node.kind {
        ValueKind::Unit => {
            parts.push(DisplayPart::Text("completed (no value)"));
            return;
        }
        ValueKind::Text if index == 0 => {
            parts.push(DisplayPart::Text(value));
            None
        }
        ValueKind::Text => {
            parts.push(DisplayPart::Quoted(value));
            None
        }
        ValueKind::Proposition => {
            parts.push(DisplayPart::Text(match node.value.as_deref() {
                Some("true") => "holds",
                Some("false") => "does not hold",
                _ => "…",
            }));
            None
        }
        ValueKind::Record | ValueKind::ResultRow => Some(("{", "}")),
        ValueKind::FixedArray | ValueKind::Sequence => Some(("[", "]")),
        ValueKind::SequenceRepeat => Some(("repeat(", ")")),
        ValueKind::Tag | ValueKind::Expression if node.total_children > 0 => Some(("(", ")")),
        _ => {
            parts.push(DisplayPart::Text(value));
            None
        }
    };
    if let Some((open, close)) = delimiters {
        parts.push(DisplayPart::Text(close));
        if node.truncated {
            parts.push(DisplayPart::Text("…"));
        }
        for (index, child) in node.children.iter().enumerate().rev() {
            if index + 1 < node.children.len() || node.truncated {
                parts.push(DisplayPart::Text(", "));
            }
            parts.push(DisplayPart::Node(child.node));
            if let Some(name) = &child.name {
                parts.push(DisplayPart::Text(": "));
                parts.push(DisplayPart::Text(name));
            }
        }
        parts.push(DisplayPart::Text(open));
        if matches!(node.kind, ValueKind::Tag | ValueKind::Expression) {
            parts.push(DisplayPart::Text(value));
        }
    } else if node.truncated {
        let last = parts.pop();
        parts.push(DisplayPart::Text("…"));
        if let Some(last) = last {
            parts.push(last);
        }
    }
}

fn append_text(output: &mut String, text: &str, bytes_max: usize) -> bool {
    let available = bytes_max - output.len();
    let mut end = available.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&text[..end]);
    end == text.len()
}

fn append_quoted(output: &mut String, text: &str, bytes_max: usize) -> bool {
    if !append_text(output, "\"", bytes_max) {
        return false;
    }
    for character in text.chars().take(bytes_max) {
        for escaped in character.escape_debug() {
            if !append_text(output, escaped.encode_utf8(&mut [0; 4]), bytes_max) {
                return false;
            }
        }
    }
    append_text(output, "\"", bytes_max)
}

#[cfg(test)]
#[path = "tests/value_tests.rs"]
mod tests;
