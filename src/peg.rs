//! PEG grammar text -> rule table; rule table + input -> capture tree.
//!
//! A port of ESPice's `tests/frontend/peg.zig`, so the dialect
//! grammars in `grammar/*.peg` are read as ESPice reads them. Notation (Ford,
//! POPL 2004, peg/leg spelling):
//!
//! ```text
//! # comment               Rule <- a b / c      sequence, ordered choice
//! e* e+ e?                &e !e                greedy repetition; lookahead
//! 'lit' "lit" 'lit'i      [a-z_\]] [^\n] .     literal (i: ASCII case-blind); class; any byte
//! ```
//!
//! A rule named with an uppercase first letter is captured: its match is a
//! [`Row`] tagged with the AST tag of that name (the caller's `tag` table, so
//! a capture no tag names fails the compile). Lowercase rules are lexical and
//! leave nothing. Grammar texts layer: a rule in a later text replaces the
//! earlier text's rule of that name (a dialect over `spice.peg`).

/// Recursion past this depth fails the parse: input-controlled nesting is
/// capped, not trusted.
const DEPTH_MAX: u32 = 512;

#[derive(Clone, Copy)]
enum Node {
    Seq(u32, u32), // kids[a..a+b]
    Choice(u32, u32),
    Star(u32),
    Plus(u32),
    Opt(u32),
    And(u32),
    Not(u32),
    Lit(u32, u32, bool), // bytes[a..a+b], case-blind
    Class(u32),
    Any,
    Ref(u32),
}

pub struct Grammar {
    nodes: Vec<Node>,
    kids: Vec<u32>,
    bytes: Vec<u8>,
    classes: Vec<[bool; 256]>,
    names: Vec<String>,
    bodies: Vec<u32>,
    tags: Vec<Option<u8>>,
}

/// One captured match, in preorder; a row's children are the rows from
/// `i + 1` up to its `next`.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub rule: u32,
    pub tag: u8,
    pub start: u32,
    pub end: u32,
    pub next: u32,
}

pub struct Tree<'s> {
    pub rows: Vec<Row>,
    pub src: &'s str,
}

impl Grammar {
    /// Compiles grammar texts, each layered over the ones before it; `tag`
    /// names a captured rule's AST tag.
    pub fn compile(texts: &[&str], tag: impl Fn(&str) -> Option<u8>) -> Result<Grammar, String> {
        let mut g = Grammar { nodes: vec![], kids: vec![], bytes: vec![], classes: vec![], names: vec![], bodies: vec![], tags: vec![] };
        let mut refs = Vec::new();
        for text in texts {
            let mut c = Compiler { src: text.as_bytes(), pos: 0, g, refs, seen: vec![] };
            while c.peek() != 0 {
                let name = c.ident().ok_or_else(|| c.err("rule name"))?;
                if !c.arrow() {
                    return Err(c.err("`<-`"));
                }
                if c.seen.iter().any(|n| n == name) {
                    return Err(format!("duplicate rule {name}"));
                }
                c.seen.push(name.to_string());
                let body = c.choice()?;
                match c.g.names.iter().position(|n| n == name) {
                    Some(i) => c.g.bodies[i] = body,
                    None => {
                        let t = match name.starts_with(|ch: char| ch.is_ascii_uppercase()) {
                            true => Some(tag(name).ok_or_else(|| format!("rule {name} is captured but no AST tag"))?),
                            false => None,
                        };
                        c.g.names.push(name.to_string());
                        c.g.bodies.push(body);
                        c.g.tags.push(t);
                    }
                }
            }
            (g, refs) = (c.g, c.refs);
        }
        for n in &mut g.nodes {
            if let Node::Ref(i) = n {
                let name = &refs[*i as usize];
                *i = g.names.iter().position(|r| r == name).ok_or_else(|| format!("undefined rule {name}"))? as u32;
            }
        }
        Ok(g)
    }

    /// The rule named `name`.
    pub fn rule(&self, name: &str) -> Option<u32> {
        self.names.iter().position(|n| n == name).map(|i| i as u32)
    }

    pub fn name(&self, rule: u32) -> &str {
        &self.names[rule as usize]
    }

    /// Parses all of `src` from rule `start`; `Err` is the furthest byte any
    /// terminal reached.
    pub fn parse<'s>(&self, start: u32, src: &'s str) -> Result<Tree<'s>, usize> {
        let mut p = Parser { g: self, src: src.as_bytes(), rows: Vec::new(), depth: 0, furthest: 0 };
        match p.rule(start, 0) {
            Ok(Some(end)) if end as usize == src.len() => Ok(Tree { rows: p.rows, src }),
            _ => Err(p.furthest as usize),
        }
    }
}

impl<'s> Tree<'s> {
    pub fn tag(&self, i: usize) -> u8 {
        self.rows[i].tag
    }

    pub fn text(&self, i: usize) -> &'s str {
        let r = self.rows[i];
        &self.src[r.start as usize..r.end as usize]
    }

    /// Direct children of row `i`, or the roots for `None`.
    pub fn children(&self, i: Option<usize>) -> impl Iterator<Item = usize> + '_ {
        let (mut at, end) = match i {
            Some(p) => (p + 1, self.rows[p].next as usize),
            None => (0, self.rows.len()),
        };
        std::iter::from_fn(move || {
            (at < end).then(|| {
                let k = at;
                at = self.rows[k].next as usize;
                k
            })
        })
    }
}

struct Compiler<'a> {
    src: &'a [u8],
    pos: usize,
    g: Grammar,
    refs: Vec<String>,
    seen: Vec<String>, // this text's rules
}

impl<'a> Compiler<'a> {
    fn err(&self, want: &str) -> String {
        let line = self.src[..self.pos].iter().filter(|&&b| b == b'\n').count() + 1;
        format!("grammar line {line}: expected {want}")
    }

    fn skip(&mut self) {
        while let Some(&ch) = self.src.get(self.pos) {
            match ch {
                b'#' => {
                    while self.src.get(self.pos).is_some_and(|&b| b != b'\n') {
                        self.pos += 1;
                    }
                }
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> u8 {
        self.skip();
        self.src.get(self.pos).copied().unwrap_or(0)
    }

    fn ident(&mut self) -> Option<&'a str> {
        self.skip();
        let start = self.pos;
        while self.src.get(self.pos).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
            self.pos += 1;
        }
        (self.pos > start).then(|| std::str::from_utf8(&self.src[start..self.pos]).unwrap())
    }

    fn arrow(&mut self) -> bool {
        self.skip();
        let hit = self.src[self.pos..].starts_with(b"<-");
        if hit {
            self.pos += 2;
        }
        hit
    }

    fn add(&mut self, n: Node) -> u32 {
        self.g.nodes.push(n);
        (self.g.nodes.len() - 1) as u32
    }

    fn group(&mut self, choice: bool, items: Vec<u32>) -> u32 {
        if items.len() == 1 {
            return items[0];
        }
        let (a, b) = (self.g.kids.len() as u32, items.len() as u32);
        self.g.kids.extend(items);
        self.add(if choice { Node::Choice(a, b) } else { Node::Seq(a, b) })
    }

    fn choice(&mut self) -> Result<u32, String> {
        let mut alts = vec![self.sequence()?];
        while self.peek() == b'/' {
            self.pos += 1;
            alts.push(self.sequence()?);
        }
        Ok(self.group(true, alts))
    }

    fn at_rule_start(&mut self) -> bool {
        let save = self.pos;
        let hit = self.ident().is_some() && self.arrow();
        self.pos = save;
        hit
    }

    fn sequence(&mut self) -> Result<u32, String> {
        let mut items = Vec::new();
        loop {
            let ch = self.peek();
            if ch == 0 || ch == b'/' || ch == b')' || self.at_rule_start() {
                break;
            }
            items.push(self.prefix()?);
        }
        Ok(if items.is_empty() { self.add(Node::Seq(0, 0)) } else { self.group(false, items) })
    }

    fn prefix(&mut self) -> Result<u32, String> {
        let ch = self.peek();
        if ch == b'&' || ch == b'!' {
            self.pos += 1;
            let inner = self.suffix()?;
            return Ok(self.add(if ch == b'&' { Node::And(inner) } else { Node::Not(inner) }));
        }
        self.suffix()
    }

    fn suffix(&mut self) -> Result<u32, String> {
        let mut inner = self.primary()?;
        loop {
            let n = match self.peek() {
                b'*' => Node::Star(inner),
                b'+' => Node::Plus(inner),
                b'?' => Node::Opt(inner),
                _ => return Ok(inner),
            };
            self.pos += 1;
            inner = self.add(n);
        }
    }

    fn primary(&mut self) -> Result<u32, String> {
        match self.peek() {
            b'(' => {
                self.pos += 1;
                let inner = self.choice()?;
                if self.peek() != b')' {
                    return Err(self.err("`)`"));
                }
                self.pos += 1;
                Ok(inner)
            }
            b'.' => {
                self.pos += 1;
                Ok(self.add(Node::Any))
            }
            q @ (b'\'' | b'"') => self.literal(q),
            b'[' => self.class(),
            _ => {
                let name = self.ident().ok_or_else(|| self.err("expression"))?.to_string();
                self.refs.push(name);
                Ok(self.add(Node::Ref(self.refs.len() as u32 - 1)))
            }
        }
    }

    fn escaped(&mut self) -> Result<u8, String> {
        let ch = *self.src.get(self.pos).ok_or_else(|| self.err("byte"))?;
        self.pos += 1;
        if ch != b'\\' {
            return Ok(ch);
        }
        let e = *self.src.get(self.pos).ok_or_else(|| self.err("escape"))?;
        self.pos += 1;
        Ok(match e {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'\\' | b'\'' | b'"' | b']' | b'[' | b'-' | b'^' => e,
            b'x' => {
                let hex = self.src.get(self.pos..self.pos + 2).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()).ok_or_else(|| self.err("\\xHH"))?;
                self.pos += 2;
                hex
            }
            _ => return Err(self.err("known escape")),
        })
    }

    fn literal(&mut self, quote: u8) -> Result<u32, String> {
        self.pos += 1;
        let start = self.g.bytes.len() as u32;
        while self.src.get(self.pos) != Some(&quote) {
            if self.pos >= self.src.len() {
                return Err(self.err("closing quote"));
            }
            let b = self.escaped()?;
            self.g.bytes.push(b);
        }
        self.pos += 1;
        let len = self.g.bytes.len() as u32 - start;
        if len == 0 {
            return Err(self.err("non-empty literal"));
        }
        let blind = self.src.get(self.pos) == Some(&b'i') && !self.src.get(self.pos + 1).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if blind {
            self.pos += 1;
        }
        Ok(self.add(Node::Lit(start, len, blind)))
    }

    fn class(&mut self) -> Result<u32, String> {
        self.pos += 1;
        let mut set = [false; 256];
        let negate = self.src.get(self.pos) == Some(&b'^');
        if negate {
            self.pos += 1;
        }
        while self.src.get(self.pos) != Some(&b']') {
            if self.pos >= self.src.len() {
                return Err(self.err("`]`"));
            }
            let lo = self.escaped()?;
            let mut hi = lo;
            if self.src.get(self.pos) == Some(&b'-') && self.src.get(self.pos + 1).is_some_and(|&b| b != b']') {
                self.pos += 1;
                hi = self.escaped()?;
            }
            if hi < lo {
                return Err(self.err("ascending range"));
            }
            set[lo as usize..=hi as usize].fill(true);
        }
        self.pos += 1;
        if negate {
            set.iter_mut().for_each(|b| *b = !*b);
        }
        self.g.classes.push(set);
        Ok(self.add(Node::Class(self.g.classes.len() as u32 - 1)))
    }
}

struct Parser<'g, 's> {
    g: &'g Grammar,
    src: &'s [u8],
    rows: Vec<Row>,
    depth: u32,
    furthest: u32,
}

type Step = Result<Option<u32>, ()>; // Err: depth exceeded

impl Parser<'_, '_> {
    fn rule(&mut self, r: u32, pos: u32) -> Step {
        if self.depth == DEPTH_MAX {
            return Err(());
        }
        self.depth += 1;
        let body = self.g.bodies[r as usize];
        let out = if let Some(tag) = self.g.tags[r as usize] {
            let at = self.rows.len();
            self.rows.push(Row { rule: r, tag, start: pos, end: pos, next: 0 });
            let end = self.at(body, pos);
            match end {
                Ok(Some(e)) => {
                    self.rows[at].end = e;
                    self.rows[at].next = self.rows.len() as u32;
                }
                _ => self.rows.truncate(at),
            }
            end
        } else {
            self.at(body, pos)
        };
        self.depth -= 1;
        out
    }

    fn miss(&mut self, pos: u32) -> Step {
        self.furthest = self.furthest.max(pos);
        Ok(None)
    }

    fn at(&mut self, n: u32, pos: u32) -> Step {
        match self.g.nodes[n as usize] {
            Node::Seq(a, b) => {
                let save = self.rows.len();
                let mut at = pos;
                for k in a..a + b {
                    match self.at(self.g.kids[k as usize], at)? {
                        Some(e) => at = e,
                        None => {
                            self.rows.truncate(save);
                            return Ok(None);
                        }
                    }
                }
                Ok(Some(at))
            }
            Node::Choice(a, b) => {
                for k in a..a + b {
                    if let Some(e) = self.at(self.g.kids[k as usize], pos)? {
                        return Ok(Some(e));
                    }
                }
                Ok(None)
            }
            Node::Star(c) | Node::Plus(c) => {
                let (mut at, mut count) = (pos, 0);
                while let Some(e) = self.at(c, at)? {
                    count += 1;
                    if e == at {
                        break; // an empty match repeats forever
                    }
                    at = e;
                }
                Ok((count > 0 || matches!(self.g.nodes[n as usize], Node::Star(_))).then_some(at))
            }
            Node::Opt(c) => Ok(Some(self.at(c, pos)?.unwrap_or(pos))),
            Node::And(c) | Node::Not(c) => {
                let save = self.rows.len();
                let hit = self.at(c, pos)?.is_some();
                self.rows.truncate(save);
                Ok((hit == matches!(self.g.nodes[n as usize], Node::And(_))).then_some(pos))
            }
            Node::Lit(a, b, blind) => {
                let want = &self.g.bytes[a as usize..(a + b) as usize];
                match self.src.get(pos as usize..(pos + b) as usize) {
                    Some(got) if if blind { got.eq_ignore_ascii_case(want) } else { got == want } => Ok(Some(pos + b)),
                    _ => self.miss(pos),
                }
            }
            Node::Class(c) => match self.src.get(pos as usize) {
                Some(&b) if self.g.classes[c as usize][b as usize] => Ok(Some(pos + 1)),
                _ => self.miss(pos),
            },
            Node::Any => if (pos as usize) < self.src.len() { Ok(Some(pos + 1)) } else { self.miss(pos) },
            Node::Ref(r) => self.rule(r, pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_lookahead_and_layers() {
        let tag = |n: &str| ["List", "Item"].iter().position(|t| *t == n).map(|i| i as u8);
        let g = Grammar::compile(&["List <- Item (',' Item)* !.\nItem <- 'ab'i / [0-9]+ # comment\n"], tag).unwrap();
        let t = g.parse(g.rule("List").unwrap(), "AB,12,ab").unwrap();
        let items: Vec<&str> = t.children(Some(0)).map(|i| t.text(i)).collect();
        assert_eq!(items, ["AB", "12", "ab"]);
        assert!(t.children(Some(0)).all(|i| t.tag(i) == 1));
        assert!(g.parse(0, "ab,").is_err());
        assert!(Grammar::compile(&["List <- b"], tag).err().unwrap().contains("undefined rule b"));
        assert!(Grammar::compile(&["Bad <- 'x'"], tag).err().unwrap().contains("no AST tag"));
        assert!(Grammar::compile(&["a <- 'x'\na <- 'y'"], tag).err().unwrap().contains("duplicate"));
        // A later text replaces a rule.
        let g = Grammar::compile(&["List <- Item !.\nItem <- 'a'", "Item <- 'b'"], tag).unwrap();
        assert!(g.parse(0, "b").is_ok() && g.parse(0, "a").is_err());
    }
}
