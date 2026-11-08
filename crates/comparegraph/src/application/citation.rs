//! What a cited line must be before `claim` writes it (field-run asks 8, 11 and 14): not blank, a comment or a docstring
//! unless the caller allows it; matching an `--expect` pattern when one is given; and the one-line resolution of an
//! `--anchor 'path::regex'`. One classifier serves all three, so an anchor and an `--at` are judged the same way.
//!
//! Patterns are a regex subset matched per line by a small backtracking matcher (no `regex` dependency without a
//! ruling): literals, `.`, `[...]` (ranges, `^` negation), `\d \w \s \D \W \S \b \B`, escapes, `^ $`, groups `( )` and
//! `(?: )`, `|`, and the quantifiers `* + ? {n} {n,} {n,m}` (a trailing `?` for laziness is accepted; for a yes/no match
//! it changes nothing). Look-around, back-references and inline flags are refused as unsupported.
use std::cell::Cell;

// ---------------------------------------------------------------- pattern

#[derive(Debug, Clone)]
enum Atom {
    Char(char),
    Any,
    Class {
        neg: bool,
        ranges: Vec<(char, char)>,
        shorts: Vec<char>,
    },
    Short(char),
    Start,
    End,
    WordB(bool),
    Group(Vec<Vec<Item>>),
}
#[derive(Debug, Clone)]
struct Item {
    atom: Atom,
    min: usize,
    max: Option<usize>,
}

/// A compiled pattern; `is_match` tests one line.
#[derive(Debug, Clone)]
pub struct Pattern {
    alts: Vec<Vec<Item>>,
    pub source: String,
}

const STEP_BUDGET: usize = 200_000;

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn err<T>(&self, m: &str) -> Result<T, String> {
        Err(format!("{m} at character {}", self.i + 1))
    }
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn alts(&mut self, depth: usize) -> Result<Vec<Vec<Item>>, String> {
        let mut alts = vec![self.seq(depth)?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.seq(depth)?);
        }
        Ok(alts)
    }
    fn seq(&mut self, depth: usize) -> Result<Vec<Item>, String> {
        let mut out = vec![];
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom(depth)?;
            let (min, max) = self.quant()?;
            if (min, max) != (1, Some(1))
                && matches!(atom, Atom::Start | Atom::End | Atom::WordB(_))
            {
                return self.err("a quantifier after an anchor");
            }
            out.push(Item { atom, min, max });
        }
        Ok(out)
    }
    fn number(&mut self) -> Option<usize> {
        let st = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.s[st..self.i].iter().collect::<String>().parse().ok()
    }
    fn quant(&mut self) -> Result<(usize, Option<usize>), String> {
        let q = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => {
                let save = self.i;
                self.i += 1;
                let Some(n) = self.number() else {
                    self.i = save;
                    return self
                        .err("`{` must start a {n}, {n,} or {n,m} quantifier (escape it as \\{)");
                };
                let m = if self.peek() == Some(',') {
                    self.i += 1;
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(
                            self.number()
                                .ok_or_else(|| "a {n,m} quantifier needs a number".to_string())?,
                        )
                    }
                } else {
                    Some(n)
                };
                if self.peek() != Some('}') {
                    return self.err("an unclosed {n,m} quantifier");
                }
                if m.is_some_and(|m| m < n) {
                    return self.err("a {n,m} quantifier with m < n");
                }
                (n, m)
            }
            _ => return Ok((1, Some(1))),
        };
        self.i += 1;
        if self.peek() == Some('?') {
            self.i += 1; // lazy: same yes/no answer
        }
        if matches!(self.peek(), Some('*' | '+' | '?' | '{')) {
            return self.err("a repeated quantifier");
        }
        Ok(q)
    }
    fn escape(&mut self) -> Result<Atom, String> {
        self.i += 1;
        let Some(c) = self.peek() else {
            return self.err("a trailing backslash");
        };
        self.i += 1;
        Ok(match c {
            'd' | 'w' | 's' | 'D' | 'W' | 'S' => Atom::Short(c),
            'b' => Atom::WordB(true),
            'B' => Atom::WordB(false),
            'n' => Atom::Char('\n'),
            't' => Atom::Char('\t'),
            c if c.is_ascii_alphanumeric() => {
                self.i -= 1;
                return self.err(&format!("the escape \\{c} is not supported"));
            }
            c => Atom::Char(c),
        })
    }
    fn atom(&mut self, depth: usize) -> Result<Atom, String> {
        let c = self.peek().unwrap();
        match c {
            '\\' => self.escape(),
            '.' => {
                self.i += 1;
                Ok(Atom::Any)
            }
            '^' => {
                self.i += 1;
                Ok(Atom::Start)
            }
            '$' => {
                self.i += 1;
                Ok(Atom::End)
            }
            '*' | '+' | '?' => self.err("a quantifier with nothing to repeat"),
            '{' => self.err("`{` with nothing to repeat (escape it as \\{)"),
            '(' => {
                if depth > 32 {
                    return self.err("groups nested too deep");
                }
                self.i += 1;
                if self.peek() == Some('?') {
                    if self.s.get(self.i + 1) == Some(&':') {
                        self.i += 2;
                    } else {
                        return self.err("look-around and inline flags are not supported");
                    }
                }
                let alts = self.alts(depth + 1)?;
                if self.peek() != Some(')') {
                    return self.err("an unclosed group");
                }
                self.i += 1;
                Ok(Atom::Group(alts))
            }
            ')' => self.err("an unmatched `)`"),
            '[' => self.class(),
            c => {
                self.i += 1;
                Ok(Atom::Char(c))
            }
        }
    }
    fn class(&mut self) -> Result<Atom, String> {
        self.i += 1;
        let neg = self.peek() == Some('^');
        if neg {
            self.i += 1;
        }
        let (mut ranges, mut shorts) = (vec![], vec![]);
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                return self.err("an unclosed `[`");
            };
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            let lo = if c == '\\' {
                self.i += 1;
                let Some(e) = self.peek() else {
                    return self.err("a trailing backslash");
                };
                self.i += 1;
                match e {
                    'd' | 'w' | 's' | 'D' | 'W' | 'S' => {
                        shorts.push(e);
                        continue;
                    }
                    'n' => '\n',
                    't' => '\t',
                    e if e.is_ascii_alphanumeric() => {
                        return self.err(&format!("the escape \\{e} is not supported"))
                    }
                    e => e,
                }
            } else {
                self.i += 1;
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                self.i += 1;
                let mut hi = self.peek().unwrap();
                self.i += 1;
                if hi == '\\' {
                    hi = self
                        .peek()
                        .ok_or_else(|| "a trailing backslash".to_string())?;
                    self.i += 1;
                }
                if hi < lo {
                    return self.err("a reversed range in `[...]`");
                }
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
        Ok(Atom::Class {
            neg,
            ranges,
            shorts,
        })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn short_matches(s: char, c: char) -> bool {
    match s {
        'd' => c.is_ascii_digit(),
        'D' => !c.is_ascii_digit(),
        'w' => is_word(c),
        'W' => !is_word(c),
        's' => c.is_whitespace(),
        _ => !c.is_whitespace(),
    }
}

struct Matcher<'a> {
    t: &'a [char],
    steps: Cell<usize>,
}

impl Matcher<'_> {
    fn tick(&self) -> bool {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        n <= STEP_BUDGET
    }
    fn seq(&self, seq: &[Item], i: usize, pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        if !self.tick() {
            return false;
        }
        match seq.get(i) {
            None => k(pos),
            Some(it) => self.rep(it, 0, pos, &mut |p| self.seq(seq, i + 1, p, k)),
        }
    }
    fn rep(&self, it: &Item, count: usize, pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        if !self.tick() {
            return false;
        }
        if it.max.is_none_or(|m| count < m)
            && self.atom(&it.atom, pos, &mut |p| {
                // an empty iteration past the minimum would loop forever
                if p == pos && count >= it.min {
                    return false;
                }
                self.rep(it, count + 1, p, k)
            })
        {
            return true;
        }
        count >= it.min && k(pos)
    }
    fn atom(&self, a: &Atom, pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        let cur = self.t.get(pos).copied();
        match a {
            Atom::Char(c) => cur == Some(*c) && k(pos + 1),
            Atom::Any => cur.is_some() && k(pos + 1),
            Atom::Short(s) => cur.is_some_and(|c| short_matches(*s, c)) && k(pos + 1),
            Atom::Class {
                neg,
                ranges,
                shorts,
            } => {
                cur.is_some_and(|c| {
                    let inside = ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c))
                        || shorts.iter().any(|s| short_matches(*s, c));
                    inside != *neg
                }) && k(pos + 1)
            }
            Atom::Start => pos == 0 && k(pos),
            Atom::End => pos == self.t.len() && k(pos),
            Atom::WordB(want) => {
                let before = pos > 0 && is_word(self.t[pos - 1]);
                let after = cur.is_some_and(is_word);
                ((before != after) == *want) && k(pos)
            }
            Atom::Group(alts) => alts.iter().any(|seq| self.seq(seq, 0, pos, k)),
        }
    }
}

impl Pattern {
    pub fn parse(src: &str) -> Result<Pattern, String> {
        if src.is_empty() {
            return Err("an empty pattern".into());
        }
        let chars: Vec<char> = src.chars().collect();
        let mut p = Parser { s: &chars, i: 0 };
        let alts = p.alts(0)?;
        if p.i != chars.len() {
            return p.err("an unmatched `)`");
        }
        Ok(Pattern {
            alts,
            source: src.to_string(),
        })
    }
    /// Does the pattern match anywhere in `line`? Err when the match exceeds the step budget.
    pub fn is_match(&self, line: &str) -> Result<bool, String> {
        let t: Vec<char> = line.chars().collect();
        let m = Matcher {
            t: &t,
            steps: Cell::new(0),
        };
        for start in 0..=t.len() {
            if self
                .alts
                .iter()
                .any(|seq| m.seq(seq, 0, start, &mut |_| true))
            {
                return Ok(true);
            }
            if m.steps.get() > STEP_BUDGET {
                return Err(format!(
                    "the pattern {:?} is too costly to match (backtracking budget spent)",
                    self.source
                ));
            }
        }
        Ok(false)
    }
}

// ---------------------------------------------------------------- line classes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Comment,
    Docstring,
    /// an import, use, include or package line: real code, weak evidence
    Import,
    Code,
}
impl LineKind {
    pub fn is_prose(self) -> bool {
        matches!(
            self,
            LineKind::Blank | LineKind::Comment | LineKind::Docstring
        )
    }
    pub fn word(self) -> &'static str {
        match self {
            LineKind::Blank => "blank",
            LineKind::Comment => "comment",
            LineKind::Docstring => "docstring",
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
/// An extension it does not know classifies only blank lines.
pub fn classify(path: &str, text: &str) -> Vec<LineKind> {
    let sx = syntax_of(path);
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
    fn the_pattern_subset_matches_like_a_regex() {
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
        for bad in ["(", ")", "[a", "*a", "a**", r"\q", "(?=x)", "a{3,1}", ""] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pathological_pattern_is_an_error_not_a_hang() {
        let p = Pattern::parse("(a|aa)*(a|aa)*(a|aa)*c").unwrap();
        assert!(p.is_match(&"a".repeat(60)).is_err());
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
        assert_eq!(
            classify("README", "# title\n\n"),
            vec![Code, Blank],
            "an unknown extension classifies only blanks"
        );
    }
}
