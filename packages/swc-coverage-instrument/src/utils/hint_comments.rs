use once_cell::sync::Lazy;
use regex::Regex as Regexp;
use swc_common::{
    comments::{Comment, Comments},
    Span,
};
use swc_ecmascript::ast::*;

/// pattern for istanbul to ignore the whole file
/// This is not fully identical to original file comments
/// https://github.com/istanbuljs/istanbuljs/blob/6f45283feo31faaa066375528f6b68e3a9927b2d5/packages/istanbul-lib-instrument/src/visitor.js#L10=
/// as regex package doesn't support lookaround
static COMMENT_FILE_REGEX: Lazy<Regexp> =
    Lazy::new(|| Regexp::new(r"^\s*istanbul\s+ignore\s+(file)(\W|$)").unwrap());

/// pattern for istanbul to ignore a section
pub static COMMENT_RE: Lazy<Regexp> =
    Lazy::new(|| Regexp::new(r"^\s*istanbul\s+ignore\s+(if|else|next)(\W|$)").unwrap());

pub fn should_ignore_file<C: Clone + Comments>(comments: &C, program: &Program) -> bool {
    let pos = match &program {
        Program::Module(module) => module.span,
        Program::Script(script) => script.span,
        _ => unreachable!("program should be module or script"),
    };

    let validate_comments = |comments: &Option<Vec<Comment>>| {
        if let Some(comments) = comments {
            comments
                .iter()
                .any(|comment| COMMENT_FILE_REGEX.is_match(&comment.text))
        } else {
            false
        }
    };

    vec![
        comments.get_leading(pos.lo),
        comments.get_leading(pos.hi),
        comments.get_trailing(pos.lo),
        comments.get_trailing(pos.hi),
    ]
    .iter()
    .any(|c| validate_comments(c))
}

pub fn lookup_hint_comments<C: Clone + Comments>(
    comments: &C,
    span: Option<&Span>,
) -> Option<String> {
    if let Some(span) = span {
        let h = comments.get_leading(span.hi);
        let l = comments.get_leading(span.lo);

        if let Some(h) = h {
            let h_value = h.iter().find_map(|c| {
                COMMENT_RE
                    .captures(&c.text)
                    .map(|captures| captures.get(1).map(|c| c.as_str().trim().to_string()))
                    .flatten()
            });

            if let Some(h_value) = h_value {
                return Some(h_value);
            }
        }

        if let Some(l) = l {
            let l_value = l.iter().find_map(|c| {
                COMMENT_RE
                    .captures(&c.text)
                    .map(|captures| captures.get(1).map(|c| c.as_str().trim().to_string()))
                    .flatten()
            });

            return l_value;
        }
    }

    return None;
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum IgnoreScope {
    Next,
    If,
    Else,
}

pub fn should_ignore<C: Clone + Comments>(
    comments: &C,
    span: Option<&Span>,
) -> Option<IgnoreScope> {
    let comments = lookup_hint_comments(comments, span);

    if let Some(comments) = comments.as_deref() {
        match comments {
            "next" => Some(IgnoreScope::Next),
            "if" => Some(IgnoreScope::If),
            "else" => Some(IgnoreScope::Else),
            _ => None,
        }
    } else {
        None
    }
}
