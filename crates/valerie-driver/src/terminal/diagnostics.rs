//! Generic compiler and verifier diagnostic model.
//!
//! Provides structured span diagnostics and pretty-printing with source code
//! excerpts, line numbers, and carets matching rustc / Cargo.

use super::config::colors_enabled;
use super::style::Style;
use std::fs;
use std::path::Path;

/// Severity of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
    Help,
}

impl DiagnosticLevel {
    pub fn name(&self) -> &'static str {
        match self {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Note => "note",
            DiagnosticLevel::Help => "help",
        }
    }

    pub fn color(&self) -> &'static str {
        match self {
            DiagnosticLevel::Error => Style::RED_BOLD,
            DiagnosticLevel::Warning => Style::YELLOW_BOLD,
            DiagnosticLevel::Note => Style::CYAN_BOLD,
            DiagnosticLevel::Help => Style::GREEN_BOLD,
        }
    }
}

/// Source code location span for an error or warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: String,
    pub line_start: usize,
    pub col_start: usize,
    pub line_end: usize,
    pub col_end: usize,
}

impl SourceSpan {
    pub fn point(file: impl Into<String>, line: usize, col: usize) -> Self {
        Self {
            file: file.into(),
            line_start: line,
            col_start: col,
            line_end: line,
            col_end: col + 1,
        }
    }

    pub fn range(
        file: impl Into<String>,
        line_start: usize,
        col_start: usize,
        line_end: usize,
        col_end: usize,
    ) -> Self {
        Self {
            file: file.into(),
            line_start,
            col_start,
            line_end,
            col_end,
        }
    }
}

/// Structured diagnostic representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub code: Option<String>,
    pub message: String,
    pub span: Option<SourceSpan>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            code: None,
            message: message.into(),
            span: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Renders the diagnostic to a formatted string.
    pub fn render(&self) -> String {
        let use_color = colors_enabled();
        let level_color = if use_color { self.level.color() } else { "" };
        let blue_bold = if use_color { Style::BLUE_BOLD } else { "" };
        let reset = if use_color { Style::RESET } else { "" };
        let bold = if use_color { Style::BOLD } else { "" };

        let code_suffix = match &self.code {
            Some(c) => format!("[{}]", c),
            None => "".to_string(),
        };

        let mut out = String::new();
        out.push_str(&format!(
            "{}{}{}{}{}: {}\n",
            bold,
            level_color,
            self.level.name(),
            code_suffix,
            reset,
            self.message
        ));

        if let Some(span) = &self.span {
            out.push_str(&format!(
                "  {}-->{} {}:{}:{}\n",
                blue_bold, reset, span.file, span.line_start, span.col_start
            ));

            if let Ok(content) = fs::read_to_string(Path::new(&span.file)) {
                let lines: Vec<&str> = content.lines().collect();
                if span.line_start > 0 && span.line_start <= lines.len() {
                    let src_line = lines[span.line_start - 1];
                    let line_num_str = format!("{}", span.line_start);

                    out.push_str(&format!("   {}|{}\n", blue_bold, reset));
                    out.push_str(&format!(
                        " {}{}{}|{} {}\n",
                        line_num_str, blue_bold, reset, reset, src_line
                    ));

                    let pointer_pad = if span.col_start > 1 {
                        " ".repeat(span.col_start - 1)
                    } else {
                        "".to_string()
                    };
                    let pointer_len = if span.col_end > span.col_start {
                        span.col_end - span.col_start
                    } else {
                        1
                    };
                    let carets = "^".repeat(pointer_len);

                    out.push_str(&format!(
                        "   {}|{} {}{}{}{}\n",
                        blue_bold, reset, pointer_pad, level_color, carets, reset
                    ));
                }
            }
        }

        out
    }
}

/// Pluggable diagnostic parser trait for different compiler backends.
pub trait DiagnosticParser: Send + Sync {
    fn try_parse(&self, line: &str) -> Option<Diagnostic>;
}

/// Parser for F* verifier output (`file(l,c-l,c): (Error n) msg`).
pub struct FStarDiagnosticParser;

impl DiagnosticParser for FStarDiagnosticParser {
    fn try_parse(&self, line: &str) -> Option<Diagnostic> {
        if let Some((loc, rest)) = line.split_once("): ") {
            let (file, coords) = loc.split_once('(')?;

            let mut code = None;
            let mut msg = rest.to_string();

            if rest.starts_with("(Error ") || rest.starts_with("(Warning ") {
                if let Some((c, m)) = rest.split_once(')') {
                    code = Some(c.trim_start_matches('(').to_string());
                    msg = m.trim().to_string();
                }
            }

            let (ls, cs, le, ce) = parse_coords(coords);
            return Some(
                Diagnostic::error(msg)
                    .with_code(code.unwrap_or_else(|| "F*".to_string()))
                    .with_span(SourceSpan::range(file.trim(), ls, cs, le, ce)),
            );
        }

        // Handle modern F* format: `* Error 168 at /path/to/file.fst(5,0-5,0):`
        let trimmed = line.trim();
        let stripped = trimmed.strip_prefix("* ").unwrap_or(trimmed);
        if stripped.starts_with("Error ") || stripped.starts_with("Warning ") {
            if let Some((err_header, loc_part)) = stripped.split_once(" at ") {
                let code = err_header.trim();
                let loc_clean = loc_part.trim().trim_end_matches(':');
                if let Some((file, coords)) = loc_clean.split_once('(') {
                    let coords_clean = coords.trim_end_matches(')');
                    let (ls, cs, le, ce) = parse_coords(coords_clean);
                    return Some(
                        Diagnostic::error("Verification error")
                            .with_code(code)
                            .with_span(SourceSpan::range(
                                file.trim(),
                                ls,
                                cs,
                                le,
                                ce,
                            )),
                    );
                }
            }
        }

        None
    }
}

/// Parser for Vale compiler messages (`vale-compiler: parse error in file at line L, col C: msg`).
pub struct ValeDiagnosticParser;

impl DiagnosticParser for ValeDiagnosticParser {
    fn try_parse(&self, line: &str) -> Option<Diagnostic> {
        let (_, file_and_rest) = line.split_once("parse error in ")?;
        let (file, loc_and_msg) = file_and_rest.split_once(" at line ")?;
        let (line_str, col_and_msg) = loc_and_msg.split_once(", col ")?;

        let line_num = line_str.trim().parse::<usize>().ok()?;
        let (col_num, msg) = if let Some((c, m)) = col_and_msg.split_once(": ") {
            (c.trim().parse::<usize>().unwrap_or(1), m.trim().to_string())
        } else {
            (1, col_and_msg.trim().to_string())
        };

        Some(
            Diagnostic::error(msg)
                .with_code("Syntax")
                .with_span(SourceSpan::point(file.trim(), line_num, col_num)),
        )
    }
}

fn parse_coords(coords: &str) -> (usize, usize, usize, usize) {
    if let Some((start, end)) = coords.split_once('-') {
        let (ls, cs) = parse_pair(start);
        let (le, ce) = parse_pair(end);
        (ls, cs, if le == 0 { ls } else { le }, ce)
    } else {
        let (ls, cs) = parse_pair(coords);
        (ls, cs, ls, cs + 1)
    }
}

fn parse_pair(pair: &str) -> (usize, usize) {
    if let Some((l, c)) = pair.split_once(',') {
        let l_num = l.trim().parse::<usize>().unwrap_or(1);
        let c_num = c.trim().parse::<usize>().unwrap_or(1);
        (l_num, c_num)
    } else {
        let l_num = pair.trim().parse::<usize>().unwrap_or(1);
        (l_num, 1)
    }
}

/// Registry of diagnostic parsers providing a unified diagnostic formatting engine.
pub struct DiagnosticEngine {
    parsers: Vec<Box<dyn DiagnosticParser>>,
}

impl Default for DiagnosticEngine {
    fn default() -> Self {
        Self {
            parsers: vec![
                Box::new(FStarDiagnosticParser),
                Box::new(ValeDiagnosticParser),
            ],
        }
    }
}

impl DiagnosticEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, parser: impl DiagnosticParser + 'static) {
        self.parsers.push(Box::new(parser));
    }

    pub fn parse_all(&self, raw: &str) -> Vec<Diagnostic> {
        let mut results = Vec::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            for parser in &self.parsers {
                if let Some(diag) = parser.try_parse(trimmed) {
                    results.push(diag);
                    break;
                }
            }
        }
        results
    }

    pub fn format(&self, raw: &str) -> String {
        let diags = self.parse_all(raw);
        if diags.is_empty() {
            return raw.to_string();
        }

        let mut output = String::new();
        for d in diags {
            output.push_str(&d.render());
            output.push('\n');
        }
        output
    }
}

pub fn format_tool_diagnostics(raw: &str) -> String {
    DiagnosticEngine::new().format(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fstar_diagnostic_rendering() {
        let raw = "tests/fixtures/ChaCha.fst(42,12-42,30): (Error 19) assertion failed at point";
        let engine = DiagnosticEngine::new();
        let diags = engine.parse_all(raw);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].code.as_deref(), Some("Error 19"));
        assert_eq!(diags[0].message, "assertion failed at point");

        let rendered = diags[0].render();
        assert!(rendered.contains("error[Error 19]: assertion failed at point"));
        assert!(rendered.contains("--> tests/fixtures/ChaCha.fst:42:12"));
    }

    #[test]
    fn test_vale_diagnostic_rendering() {
        let raw = "vale-compiler: parse error in tests/fixtures/failures/syntax_error.vaf at line 8, col 35: Stdlib.Parsing.Parse_error";
        let engine = DiagnosticEngine::new();
        let diags = engine.parse_all(raw);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].code.as_deref(), Some("Syntax"));
        assert_eq!(diags[0].message, "Stdlib.Parsing.Parse_error");
    }
}
