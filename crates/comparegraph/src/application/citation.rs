//! What a cited line must be before `claim` writes it (field-run asks 8, 11 and 14): not blank, a comment or a docstring
//! unless the caller allows it; matching an `--expect` pattern when one is given; and the one-line resolution of an
//! `--anchor 'path::regex'`. One classifier serves all three, so an anchor and an `--at` are judged the same way.
//!
//! Patterns are Rust `regex` syntax matched per line (linear time, no recursion; look-around and back-references are
//! refused when the pattern is parsed).
// ---------------------------------------------------------------- pattern

/// A compiled pattern; `is_match` tests one line. The `regex` crate matches in time linear in the line, without recursion,
/// and refuses look-around and back-references at parse time.
#[derive(Debug, Clone)]
pub struct Pattern {
    re: regex::Regex,
    pub source: String,
}

/// The largest compiled program, in bytes (a counted quantifier such as `(a{1000}){1000}` is refused past it).
const SIZE_LIMIT: usize = 1 << 20;

impl Pattern {
    pub fn parse(src: &str) -> Result<Pattern, String> {
        if src.is_empty() {
            return Err("an empty pattern".into());
        }
        let re = regex::RegexBuilder::new(src)
            .size_limit(SIZE_LIMIT)
            .dfa_size_limit(SIZE_LIMIT * 8)
            .build()
            .map_err(|e| e.to_string().split_whitespace().collect::<Vec<_>>().join(" "))?;
        Ok(Pattern { re, source: src.to_string() })
    }
    /// Does the pattern match anywhere in `line`?
    pub fn is_match(&self, line: &str) -> Result<bool, String> {
        Ok(self.re.is_match(line))
    }
}

// ---------------------------------------------------------------- line classes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Comment,
    Docstring,
    /// a line of a prose file (Markdown, plain text, reStructuredText)
    Prose,
    /// an import, use, include or package line: real code, weak evidence
    Import,
    Code,
}
impl LineKind {
    pub fn is_prose(self) -> bool {
        matches!(
            self,
            LineKind::Blank | LineKind::Comment | LineKind::Docstring | LineKind::Prose
        )
    }
    pub fn word(self) -> &'static str {
        match self {
            LineKind::Blank => "blank",
            LineKind::Comment => "comment",
            LineKind::Docstring => "docstring",
            LineKind::Prose => "prose",
            LineKind::Import => "import",
            LineKind::Code => "code",
        }
    }
}

#[derive(Clone, Copy)]
struct Syntax {
    hash: bool,
    slash: bool,
    block: bool,
    dash: bool,
    markup: bool,
    semicolon: bool,
    triple: bool,
    /// every non-blank line is prose
    prose: bool,
}

fn syntax_of(path: &str) -> Syntax {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let none = Syntax {
        hash: false,
        slash: false,
        block: false,
        dash: false,
        markup: false,
        semicolon: false,
        triple: false,
        prose: false,
    };
    let lower = name.to_ascii_lowercase();
    if lower == "dockerfile"
        || lower == "containerfile"
        || lower == "makefile"
        || lower.starts_with("dockerfile.")
    {
        return Syntax { hash: true, ..none };
    }
    match ext.as_str() {
        "py" | "pyi" | "pyx" => Syntax {
            hash: true,
            triple: true,
            ..none
        },
        "rb" | "sh" | "bash" | "zsh" | "fish" | "yaml" | "yml" | "toml" | "pl" | "pm" | "r"
        | "ex" | "exs" | "nix" | "mk" | "cmake" | "jl" | "cr" | "coffee" | "gd" | "graphql"
        | "gql" | "ps1" | "psm1" | "tcl" | "awk" | "conf" | "cfg" | "properties" | "gitignore"
        | "dockerignore" => Syntax { hash: true, ..none },
        "ini" => Syntax {
            hash: true,
            semicolon: true,
            ..none
        },
        "tf" | "hcl" | "php" => Syntax {
            hash: true,
            slash: true,
            block: true,
            ..none
        },
        "rs" | "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "go" | "java"
        | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "cs" | "kt" | "kts"
        | "swift" | "scala" | "sc" | "dart" | "m" | "mm" | "groovy" | "gradle" | "zig"
        | "proto" | "sol" | "v" | "fs" | "fsx" | "d" | "jsonc" | "json5" => Syntax {
            slash: true,
            block: true,
            ..none
        },
        "css" => Syntax {
            block: true,
            ..none
        },
        "scss" | "less" | "sass" => Syntax {
            slash: true,
            block: true,
            ..none
        },
        "sql" | "lua" | "hs" | "elm" | "ada" | "adb" | "ads" => Syntax { dash: true, ..none },
        "html" | "htm" | "xml" | "vue" | "svelte" | "xhtml" | "svg" => Syntax {
            markup: true,
            ..none
        },
        "clj" | "cljs" | "el" | "lisp" | "scm" | "asm" | "s" => Syntax {
            semicolon: true,
            ..none
        },
        "md" | "markdown" | "txt" | "text" | "rst" => Syntax { prose: true, ..none },
        _ => none,
    }
}

fn is_import(t: &str, sx: Syntax) -> bool {
    let starts = |p: &str| t.starts_with(p);
    if (sx.triple || sx.hash)
        && (starts("import ")
            || (starts("from ") && t.contains(" import "))
            || starts("require ")
            || starts("require_relative "))
    {
        return true;
    }
    if sx.slash {
        if starts("import ") || starts("import{") {
            return true;
        }
        if starts("use ")
            || starts("pub use ")
            || starts("extern crate ")
            || starts("package ")
            || starts("using ")
            || starts("#include")
            || starts("#import")
            || starts("@import")
        {
            return true;
        }
        if (starts("const ") || starts("let ") || starts("var "))
            && t.contains("require(")
            && t.ends_with(';')
        {
            return true;
        }
        if starts("export ") && t.contains(" from ") && !t.contains('(') {
            return true;
        }
    }
    false
}

/// A Python `def`/`class` header line (`def f(x):`, `async def f():`, `class C(B):`): its next statement may be a
/// docstring. A signature split over several lines is not recognised.
fn is_def_header(t: &str) -> bool {
    (t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("class ")) && t.ends_with(':')
}

/// A line that is only one single-quoted Python string literal (`'doc'`, `"doc"`, `r"doc"`).
fn is_one_string(t: &str) -> bool {
    let rest = t.trim_start_matches(['r', 'R', 'u', 'U', 'b', 'B', 'f', 'F']);
    if t.len() - rest.len() > 2 {
        return false;
    }
    let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
        return false;
    };
    rest.len() >= 2 && rest.ends_with(q) && !rest[1..rest.len() - 1].contains(q)
}

/// The opener of a Python string that starts the stripped line (`"""`, `r'''`, …), if any.
fn triple_opener(t: &str) -> Option<&'static str> {
    let rest = t.trim_start_matches(['r', 'R', 'u', 'U', 'b', 'B', 'f', 'F']);
    if t.len() - rest.len() > 2 {
        return None;
    }
    if rest.starts_with("\"\"\"") {
        Some("\"\"\"")
    } else if rest.starts_with("'''") {
        Some("'''")
    } else {
        None
    }
}

/// One `LineKind` per line of `text` (as `str::lines` splits it), judged by the file's extension. Comment and docstring
/// blocks are recognised only when they start a line, so a `/*` inside a string (`"src/**/*.rs"`) never opens one.
/// A prose file's (`.md`, `.txt`, `.rst`) non-blank lines are all `Prose`; an extension it does not know classifies only
/// blank lines. In Python a line that is only a one-quote string literal right after a `def`/`class` header is a docstring.
pub fn classify(path: &str, text: &str) -> Vec<LineKind> {
    let sx = syntax_of(path);
    // Python: the last code line was a def/class header, so a lone string literal now is its docstring
    let mut after_def = false;
    // the open multi-line construct: its closing delimiter and the kind of its lines
    let mut open: Option<(&'static str, LineKind)> = None;
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        let t = line.trim();
        if let Some((close, kind)) = open {
            if let Some(at) = t.find(close) {
                open = None;
                let after = t[at + close.len()..].trim();
                out.push(
                    if after.is_empty() || after.starts_with(',') || after.starts_with(')') {
                        kind
                    } else {
                        LineKind::Code
                    },
                );
            } else {
                out.push(if t.is_empty() { LineKind::Blank } else { kind });
            }
            continue;
        }
        if t.is_empty() {
            out.push(LineKind::Blank);
            continue;
        }
        if sx.prose {
            out.push(LineKind::Prose);
            continue;
        }
        if n == 0 && t.starts_with("#!") {
            out.push(LineKind::Comment);
            continue;
        }
        let line_comment = (sx.hash && t.starts_with('#') && !(sx.slash && t.starts_with("#[")))
            || (sx.slash && t.starts_with("//"))
            || (sx.dash && t.starts_with("--"))
            || (sx.semicolon && t.starts_with(';'));
        if line_comment {
            out.push(LineKind::Comment);
            continue;
        }
        let block = if sx.block && t.starts_with("/*") {
            Some(("/*", "*/", LineKind::Comment))
        } else if sx.markup && t.starts_with("<!--") {
            Some(("<!--", "-->", LineKind::Comment))
        } else {
            sx.triple
                .then(|| triple_opener(t))
                .flatten()
                .map(|q| (q, q, LineKind::Docstring))
        };
        let was_after_def = std::mem::replace(&mut after_def, false);
        if let Some((opener, close, kind)) = block {
            let body = &t[t.find(opener).unwrap() + opener.len()..];
            match body.find(close) {
                Some(at) => {
                    let after = body[at + close.len()..].trim();
                    out.push(
                        if after.is_empty() || after.starts_with(',') || after.starts_with(')') {
                            kind
                        } else {
                            LineKind::Code
                        },
                    );
                }
                None => {
                    open = Some((close, kind));
                    out.push(kind);
                }
            }
            continue;
        }
        if sx.triple && was_after_def && is_one_string(t) {
            out.push(LineKind::Docstring);
            continue;
        }
        after_def = sx.triple && is_def_header(t);
        out.push(if is_import(t, sx) {
            LineKind::Import
        } else {
            LineKind::Code
        });
    }
    out
}

// ---------------------------------------------------------------- the checks claim makes

/// What a checked span is: `imports_only` when its code lines are all import/use/package lines (passes, with a
/// limitation).
pub struct SpanCheck {
    pub imports_only: bool,
}

/// `path:N` for one line, `path:S-E` for a span.
pub fn span_label(norm: &str, s: usize, e: usize) -> String {
    if s == e {
        format!("{norm}:{s}")
    } else {
        format!("{norm}:{s}-{e}")
    }
}

fn describe(kinds: &[LineKind], s: usize) -> String {
    let mut parts: Vec<String> = vec![];
    let mut i = 0;
    while i < kinds.len() {
        let k = kinds[i];
        let mut j = i;
        while j + 1 < kinds.len() && kinds[j + 1] == k {
            j += 1;
        }
        parts.push(if i == j {
            format!("line {} {}", s + i, k.word())
        } else {
            format!("lines {}-{} {}", s + i, s + j, k.word())
        });
        i = j + 1;
    }
    parts.join(", ")
}

/// The lines of the file (1-based) where `pat` matches, at most `cap`, plus the total.
fn matching_lines(
    lines: &[&str],
    pat: &Pattern,
    cap: usize,
) -> Result<(Vec<usize>, usize), String> {
    let mut found = vec![];
    let mut total = 0;
    for (i, l) in lines.iter().enumerate() {
        if pat.is_match(l)? {
            total += 1;
            if found.len() < cap {
                found.push(i + 1);
            }
        }
    }
    Ok((found, total))
}

fn list_lines(ls: &[usize], total: usize) -> String {
    let mut s = ls
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if total > ls.len() {
        s.push_str(&format!(", +{} more", total - ls.len()));
    }
    s
}

/// Check lines `s..=e` (1-based, already in range) of `text`: refused when every line is blank, a comment or a
/// docstring (unless `allow_comment`), or when `expect` is given and matches none of the lines.
pub fn check_span(
    norm: &str,
    text: &str,
    s: usize,
    e: usize,
    expect: Option<&Pattern>,
    allow_comment: bool,
) -> Result<SpanCheck, String> {
    let kinds = classify(norm, text);
    let span = &kinds[s - 1..e];
    let lines: Vec<&str> = text.lines().collect();
    if !allow_comment && span.iter().all(|k| k.is_prose()) {
        return Err(format!(
            "{} cites no code ({}); cite the code that implements the feature, or pass --allow-comment when the comment or docstring is itself the evidence",
            span_label(norm, s, e),
            describe(span, s)
        ));
    }
    if let Some(p) = expect {
        let mut hit = false;
        for l in &lines[s - 1..e] {
            if p.is_match(l)? {
                hit = true;
                break;
            }
        }
        if !hit {
            let (found, total) = matching_lines(&lines, p, 5)?;
            let where_ = if found.is_empty() {
                "it matches nowhere in the file".to_string()
            } else {
                format!(
                    "it matches {} {} of the file",
                    if total == 1 { "line" } else { "lines" },
                    list_lines(&found, total)
                )
            };
            return Err(format!(
                "{norm}:{s}-{e} does not match --expect {:?}; {where_}",
                p.source
            ));
        }
    }
    let code: Vec<&LineKind> = span.iter().filter(|k| !k.is_prose()).collect();
    Ok(SpanCheck {
        imports_only: !code.is_empty() && code.iter().all(|k| **k == LineKind::Import),
    })
}

/// Split `path::regex` on the first `::`.
pub fn split_anchor(anchor: &str) -> Result<(&str, Pattern), String> {
    let Some((p, re)) = anchor.split_once("::") else {
        return Err(format!("--anchor {anchor}: expected 'path::regex'"));
    };
    if p.is_empty() || re.is_empty() {
        return Err(format!(
            "--anchor {anchor}: expected 'path::regex' with both parts non-empty"
        ));
    }
    let pat = Pattern::parse(re).map_err(|m| format!("--anchor {anchor}: the regex has {m}"))?;
    Ok((p, pat))
}

/// The one line of `text` that `pat` matches. Lines that are blank, comments or docstrings count only with
/// `allow_comment`, so a comment naming the symbol neither makes a match ambiguous nor becomes the citation.
pub fn resolve_anchor(
    norm: &str,
    text: &str,
    pat: &Pattern,
    allow_comment: bool,
) -> Result<usize, String> {
    let kinds = classify(norm, text);
    let lines: Vec<&str> = text.lines().collect();
    let (mut code, mut prose) = (vec![], vec![]);
    for (i, l) in lines.iter().enumerate() {
        if pat.is_match(l)? {
            if allow_comment || !kinds[i].is_prose() {
                code.push(i + 1);
            } else {
                prose.push(i + 1);
            }
        }
    }
    match code.len() {
        1 => Ok(code[0]),
        0 if !prose.is_empty() => Err(format!(
            "--anchor {norm}::{} matches no code line, only comment or docstring line(s) {}; pass --allow-comment to cite one",
            pat.source,
            list_lines(&prose[..prose.len().min(5)], prose.len())
        )),
        0 => Err(format!("--anchor {norm}::{} matches no line of the file", pat.source)),
        n => Err(format!(
            "--anchor {norm}::{} is ambiguous: it matches {n} lines ({}); make the regex match one line",
            pat.source,
            list_lines(&code[..n.min(8)], n)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, t: &str) -> bool {
        Pattern::parse(p).unwrap().is_match(t).unwrap()
    }

    #[test]
    fn patterns_match_like_a_regex() {
        assert!(m("run_turn", "def run_turn(msg):"));
        assert!(m(r"^def \w+\(", "def run_turn(msg):"));
        assert!(!m(r"^def \w+\(", "  def run_turn(msg):"));
        assert!(m(r"compact\(msg\)$", "    return compact(msg)"));
        assert!(!m(r"compact\(msg\)$", "def compact(msg):"));
        assert!(m("a|b", "xb"));
        assert!(m("(?:fn|def) +go", "pub fn  go()"));
        assert!(m("[A-Z][a-z]+Agent", "class LlmAgent:"));
        assert!(!m("[^a-z]x", "ax"));
        assert!(m(r"\bS\b", "pub struct S;"));
        assert!(!m(r"\bS\b", "pub struct Stuff;"));
        assert!(m("a{2,3}b", "caab"));
        assert!(!m("a{2,3}b", "cab"));
        assert!(m("x.*?y", "x123y"));
        assert!(m("Foo::bar", "Foo::bar()"));
        assert!(m("(a*)*b", "aaab"));
        assert!(!m("(a*)*b", "aaaa"), "an empty-loop group terminates");
        for bad in ["(", ")", "[a", "*a", r"\q", "(?=x)", "a{3,1}", ""] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pathological_pattern_is_answered_not_a_hang() {
        let p = Pattern::parse("(a|aa)*(a|aa)*(a|aa)*c").unwrap();
        assert_eq!(p.is_match(&"a".repeat(60)), Ok(false));
        assert_eq!(p.is_match(&("a".repeat(60) + "c")), Ok(true));
    }

    #[test]
    fn a_long_line_neither_overflows_the_stack_nor_hangs() {
        let long = "a".repeat(200_000);
        assert_eq!(Pattern::parse("a*b").unwrap().is_match(&long), Ok(false));
        assert_eq!(Pattern::parse(".*q").unwrap().is_match(&long), Ok(false));
        assert_eq!(Pattern::parse("a+$").unwrap().is_match(&long), Ok(true));
        assert_eq!(Pattern::parse("(a|aa)*c").unwrap().is_match(&long), Ok(false));
        assert_eq!(Pattern::parse("a{1000}b").unwrap().is_match(&long), Ok(false));
        assert!(Pattern::parse("(a{1000}){1000}").is_err(), "a program past SIZE_LIMIT");
    }

    #[test]
    fn lines_are_classified_by_language() {
        use LineKind::*;
        let py = "#!/usr/bin/env python\n# c\n\"\"\"doc\nmore\"\"\"\nimport os\nx = \"\"\"not a\nstring tail\"\"\"\n";
        assert_eq!(
            classify("a.py", py),
            vec![Comment, Comment, Docstring, Docstring, Import, Code, Code]
        );
        let rs = "//! d\n#[test]\n/* a\n b */\nlet g = \"src/**/*.rs\";\n/* x */ let y = 1;\nuse a::b;\n";
        assert_eq!(
            classify("a.rs", rs),
            vec![Comment, Code, Comment, Comment, Code, Code, Import]
        );
        assert_eq!(classify("a.sql", "-- c\nSELECT 1;\n"), vec![Comment, Code]);
        assert_eq!(classify("README.md", "# T\n\nRetries.\n"), vec![Prose, Blank, Prose]);
        assert_eq!(classify("notes.txt", "x = 1\n"), vec![Prose]);
        let doc = "def f():\n    'One line.'\n    x = 'not a doc'\n    'after code'\nclass C:\n    \"Doc.\"\n";
        assert_eq!(classify("a.py", doc), vec![Code, Docstring, Code, Code, Code, Docstring]);
        assert_eq!(
            classify("README", "# title\n\n"),
            vec![Code, Blank],
            "an unknown extension classifies only blanks"
        );
    }
}
