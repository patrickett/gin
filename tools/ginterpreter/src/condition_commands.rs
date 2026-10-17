use crate::{InterpreterCondition, InterpreterFrame, InterpreterSession, Submission, TaskId};

impl InterpreterSession {
    pub(crate) fn condition_command(&mut self, name: &str, arguments: Option<&str>) -> Submission {
        let usage = match name {
            "frames" => "usage: #frames [task-id]",
            "frame" => "usage: #frame <index> [task-id]",
            "retry" => "usage: #retry [task-id]",
            "abort" => "usage: #abort [task-id]",
            _ => unreachable!("condition command"),
        };
        let Some((index, task)) = parse_selection(name, arguments) else {
            return answer(usage);
        };
        match name {
            "retry" => {
                let submission = match task {
                    Some(id) => self.retry_task(id),
                    None => self.retry(),
                };
                submission.unwrap_or_else(|| answer("no pending condition"))
            }
            "abort" => {
                let discarded = match task {
                    Some(id) => self.abort_task(id),
                    None => self.abort(),
                };
                answer(if discarded {
                    "pending condition discarded"
                } else {
                    "no pending condition"
                })
            }
            _ => {
                let condition = match task {
                    Some(id) => self.task_condition(id),
                    None => self.condition(),
                };
                answer(&frames_report(condition, index))
            }
        }
    }
}

fn parse_selection(name: &str, arguments: Option<&str>) -> Option<(Option<usize>, Option<TaskId>)> {
    let mut arguments = arguments.unwrap_or_default().split_whitespace();
    let index = if name == "frame" {
        Some(arguments.next()?.parse().ok()?)
    } else {
        None
    };
    let task = arguments
        .next()
        .map(|id| id.parse::<u64>().map(TaskId))
        .transpose()
        .ok()?;
    if arguments.next().is_some() {
        return None;
    }
    Some((index, task))
}

fn answer(text: &str) -> Submission {
    Submission::Answered {
        text: text.to_string(),
    }
}

fn frames_report(condition: Option<&InterpreterCondition>, index: Option<usize>) -> String {
    let Some(condition) = condition else {
        return "no pending condition".to_string();
    };
    if let Some(index) = index {
        let Some(frame) = condition.frame(index) else {
            return "unknown frame index".to_string();
        };
        let mut report = frame_report(frame, index);
        for local in &frame.locals {
            let value = match &local.value {
                Ok(ast::ConstValue::Int(value)) => value.to_string(),
                Ok(ast::ConstValue::Text(value)) => format!("{value:?}"),
                Ok(value) => format!("{value:?}"),
                Err(diagnostic) => format!("<{}>", diagnostic.message),
            };
            report.push_str(&format!(
                "\n  {}{} = {value}",
                if local.is_reference { "ref " } else { "" },
                local.name
            ));
        }
        report
    } else if condition.frames().is_empty() {
        "condition has no evaluator frames".to_string()
    } else {
        condition
            .frames()
            .iter()
            .enumerate()
            .map(|(index, frame)| frame_report(frame, index))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn frame_report(frame: &InterpreterFrame, index: usize) -> String {
    let (line, column) = diagnostic::source_position(&frame.source, frame.span.start());
    format!(
        "{index}: {} at {}:{line}:{column}",
        frame.function, frame.display_path
    )
}
