//! SPICE dot cards → where the plan places them: includes and `.lib`
//! calls, `.global`, `.model`, `.param`, `.func`, a `.subckt`'s head, and the
//! testbench cards (analyses, options, outputs, `.control` commands), as
//! the old lowering placed them, and their views; each card first checked
//! against its rule in the old SPICE grammar (a card no rule reads is `Raw`
//! or `Unparsed` by its word, `Rd::refused`).
//!
//! Cites: the old lowering (as `syntax::adapter::cards` ports it);
//! the old SPICE grammar's `dotcard` and its overlays (`Include`, `Lib`, `Model`,
//! `Params`, `Func`, `Options`, `Temp`, `Ic`, `Save`, `Out`, `Print`, `Op`
//! … `Four`, ngspice `Sp`, HSPICE `Tran`/`Probe`, LTspice `Tran`/`Lib`/
//! `Step`/`Params_lt`, Xyce `Include`/`Options`/`Step`); Xyce RG p.96.
//!
//! Contents:
//! - `pub(in crate::syntax::render) struct Head`, `struct Model`
//! - `Rd::dot_card(&mut self, n: u32, here: &mut Block, scope: Scope, inside: bool, acc: &mut Acc)`
//! - `Rd::subckt_head(&self, n: u32) -> Option<Head>`, `Rd::model_head(&self, n: u32) -> Option<Node>`
//! - `Rd::model_view(&self, n: u32) -> Model`, `Rd::include_path(&self, n: u32) -> String`,
//!   `Rd::lib_view(&self, n: u32) -> (String, String)`, `Rd::func_view(&self, f: Func) -> (String, Vec<String>, String)`,
//!   `Rd::param_view(&self, a: u32) -> (String, String)`

use super::{Acc, Block, Func, Kept, Rd, Scope};
use crate::codegen::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

/// A `.subckt` head as the old grammar read it: its ports and parameter
/// defaults (each its text, trimmed).
pub(in crate::syntax::render) struct Head {
    pub(in crate::syntax::render) ports: Vec<String>,
    pub(in crate::syntax::render) params: Vec<(String, String)>,
}

/// A `.model` card as the old reader kept it.
pub(in crate::syntax::render) struct Model {
    pub(in crate::syntax::render) name: String,
    pub(in crate::syntax::render) kind: String,
    /// Each `k=v` (a flag's value empty), a matrix joined by blanks.
    pub(in crate::syntax::render) params: Vec<(String, String)>,
}

impl Rd<'_> {
    /// A dot card at `scope` (`inside` a `.subckt`).
    pub(super) fn dot_card(
        &mut self,
        n: u32,
        here: &mut Block,
        scope: Scope,
        inside: bool,
        acc: &mut Acc,
    ) {
        // quirk: the old overlays' `dialect` rule (the old LTspice grammar's
        // `Raw_lt`, the old Xyce grammar's `Raw_xy`) is tried before the
        // shared dot cards.
        let word = self.head_word(n);
        if self.dialect_raw(&word) {
            return self.keep(n, here, Kept::Raw);
        }
        let done = match self.tag(n) {
            Tag::Include => self.include(n, here),
            Tag::LibCall => self.lib_call(n, here),
            Tag::Global => self.global(n, acc),
            Tag::Model => self.model(n, here),
            Tag::Param => self.param_card(n, here, inside, acc),
            Tag::Func => self.func(n, inside, acc),
            Tag::Card => Err(self.refused(n)),
            _ if scope != Scope::Top => match self.testbench_peg(n) {
                true => Err(Kept::Raw),
                false => Err(self.refused(n)),
            },
            _ => match (self.testbench_peg(n), self.testbench_ok(n)) {
                (true, true) => {
                    acc.bench.push(n);
                    Ok(())
                }
                (true, false) => Err(Kept::Raw),
                (false, _) => Err(self.refused(n)),
            },
        };
        if let Err(k) = done {
            self.keep(n, here, k);
        }
    }

    /// The head of `.subckt name ports [params:] k=v`, when the old SPICE grammar's
    /// `Subckt` reads it.
    pub(in crate::syntax::render) fn subckt_head(&self, n: u32) -> Option<Head> {
        let items = self.fields(n);
        if self.named() {
            items.first()?;
            return Some(Head {
                ports: items[1..].iter().map(|&p| self.name_of(p)).collect(),
                params: vec![],
            });
        }
        let (&name, rest) = items.split_first()?;
        if !self.is_word_item(name) {
            return None;
        }
        let mut ports = Vec::new();
        let mut params = Vec::new();
        let mut k = 0;
        while k < rest.len() && self.is_word_item(rest[k]) && !self.is_params_colon(rest[k]) {
            ports.push(self.name_of(rest[k]));
            k += 1;
        }
        if k < rest.len() && self.is_params_colon(rest[k]) {
            k += 1;
        }
        for &a in &rest[k..] {
            let v = self.ast.rhs(Node(a));
            if self.tag(a) != Tag::Assign
                || !self.is_word_item(self.ast.lhs(Node(a)))
                || !self.val_ok(v)
            {
                return None;
            }
            params.push((self.key(a), self.text(v).trim().to_string()));
        }
        Some(Head { ports, params })
    }

    /// A `.model` card's name item, when the old SPICE grammar's `Model` reads the card.
    pub(in crate::syntax::render) fn model_head(&self, n: u32) -> Option<Node> {
        let items = self.fields(n);
        let [name, kind, rest @ ..] = items else {
            return None;
        };
        if !self.is_word_item(*name) || !self.is_word_item(*kind) {
            return None;
        }
        // quirk: the old SPICE grammar's `mbody`: parentheses and commas
        // separate, nothing else.
        let mut prev = self.last(*kind);
        for &k in rest {
            if !(prev + 1..self.first(k))
                .all(|x| matches!(self.ttag(x), T::Comma | T::LParen | T::RParen))
            {
                return None;
            }
            prev = self.last(k);
        }
        if !(prev + 1..=self.line_last(self.main(n)))
            .all(|x| matches!(self.ttag(x), T::Comma | T::LParen | T::RParen))
        {
            return None;
        }
        let xyce = self.d == Dialect::Xyce;
        let mval = |v: u32| match self.tag(v) {
            Tag::Word | Tag::Str => true,
            Tag::Delimited => self.expr_ok(v) || (xyce && self.ttag(self.main(v)) == T::LBrace),
            _ => false,
        };
        let ok = rest.iter().all(|&k| match self.tag(k) {
            Tag::Word => true,
            Tag::Assign => {
                let v = self.ast.rhs(Node(k));
                v != NONE
                    && match self.tag(v) {
                        Tag::Seq => {
                            let parts = self.ast.list(self.ast.lhs(Node(v)));
                            mval(parts[0]) && parts[1..].iter().all(|&p| self.is_word_item(p))
                        }
                        _ => mval(v),
                    }
            }
            _ => false,
        });
        ok.then_some(Node(*name))
    }

    /// A `.model` card's view (Xyce's BSIM3 level 8 or 9 read as 49).
    pub(in crate::syntax::render) fn model_view(&self, n: u32) -> Model {
        let items = self.fields(n);
        let mut m = Model {
            name: self.text(items[0]),
            kind: self.text(items[1]),
            params: self.params_of(&items[2..], true),
        };
        if self.d == Dialect::Xyce
            && matches!(m.kind.to_ascii_lowercase().as_str(), "nmos" | "pmos")
        {
            // Xyce's BSIM3 is level 9 (RG Table 2-36): the IR keeps 49.
            for (k, v) in &mut m.params {
                if k.eq_ignore_ascii_case("level") && matches!(v.parse::<f64>(), Ok(8.0 | 9.0)) {
                    *v = "49".into();
                }
            }
        }
        m
    }

    /// An `.include` card's path (Xyce `.incl` takes the rest of the card).
    pub(in crate::syntax::render) fn include_path(&self, n: u32) -> String {
        let items = self.fields(n);
        let first = items[0];
        if self.d == Dialect::Xyce && self.head_word(n) == "incl" {
            // quirk: the old Xyce grammar's `Path_xy`: a quoted path alone,
            // else all of the rest of the card.
            match items {
                [p] if self.tag(*p) == Tag::Str => self.path_of(*p),
                _ => self.span(self.main(first), self.line_last(self.main(n))),
            }
        } else {
            self.path_of(first)
        }
    }

    /// A `.lib path section` card's path and section (LTspice's whole-file
    /// `.lib path`: an empty section).
    pub(in crate::syntax::render) fn lib_view(&self, n: u32) -> (String, String) {
        match self.fields(n) {
            [p] => (self.path_of(*p), String::new()),
            [p, s, ..] => (self.path_of(*p), self.text(*s)),
            [] => unreachable!("lib_call read it"),
        }
    }

    /// A function's name, arguments and body text.
    pub(in crate::syntax::render) fn func_view(&self, f: Func) -> (String, Vec<String>, String) {
        let (g, body) = match f {
            Func::Param(a) => (self.ast.lhs(Node(a)), self.body_text(self.ast.rhs(Node(a)))),
            Func::Card(n) => {
                let g = self.func_group(n).expect("func read it");
                (g, self.func_body(n, g).expect("func read it"))
            }
        };
        (
            self.text(self.ast.lhs(Node(g))),
            self.group_items(g).iter().map(|&k| self.text(k)).collect(),
            body,
        )
    }

    /// A `.param` assignment's name and value text.
    pub(in crate::syntax::render) fn param_view(&self, a: u32) -> (String, String) {
        (
            self.text(self.ast.lhs(Node(a))),
            self.vtext(self.ast.rhs(Node(a))),
        )
    }
}

// ---- private ----

impl Rd<'_> {
    fn is_word_item(&self, n: u32) -> bool {
        n != NONE && self.tag(n) == Tag::Word
    }

    fn is_params_colon(&self, n: u32) -> bool {
        self.tag(n) == Tag::Word && self.text(n).eq_ignore_ascii_case("params:")
    }

    /// The card's head keyword, lowercased, without its dot.
    fn head_word(&self, n: u32) -> String {
        let w = self.t.text(self.main(n)).to_ascii_lowercase();
        w.strip_prefix('.').unwrap_or(&w).to_string()
    }

    /// A path item without its quotes (the old lowering's `path`).
    fn path_of(&self, p: u32) -> String {
        let t = self.text(p);
        let p = t.trim();
        match p.as_bytes() {
            [b'"', .., b'"'] | [b'\'', .., b'\''] => p[1..p.len() - 1].to_string(),
            _ => p.to_string(),
        }
    }

    /// `.include path` (Xyce `.incl` takes the rest of the card).
    fn include(&mut self, n: u32, here: &mut Block) -> Result<(), Kept> {
        if self.fields(n).is_empty() {
            return Err(self.refused(n));
        }
        here.includes.push(n);
        Ok(())
    }

    /// `.lib path section` (LTspice: the section optional).
    fn lib_call(&mut self, n: u32, here: &mut Block) -> Result<(), Kept> {
        match self.fields(n) {
            [_] if self.d == Dialect::Ltspice => {}
            [_, s, ..] if self.tag(*s) == Tag::Word || self.d == Dialect::Ltspice => {}
            _ => return Err(self.refused(n)),
        }
        here.libs.push(n);
        Ok(())
    }

    /// `.global a b`.
    fn global(&mut self, n: u32, acc: &mut Acc) -> Result<(), Kept> {
        let items = self.fields(n);
        if !items.iter().all(|&k| self.is_word_item(k)) {
            return Err(self.refused(n));
        }
        for &k in items {
            let g = self.name_of(k);
            if !acc.globals.contains(&g) {
                acc.globals.push(g);
            }
        }
        Ok(())
    }

    /// `.model name kind params`.
    fn model(&mut self, n: u32, here: &mut Block) -> Result<(), Kept> {
        let Some(_) = self.model_head(n) else {
            return Err(self.refused(n));
        };
        here.models.push(n);
        Ok(())
    }

    /// quirk: whether `a` is a `.param` function definition the old reader
    /// read (the old SPICE grammar's `FuncDef`: a body that is delimited or
    /// one blank- and comma-free run).
    fn funcdef_ok(&self, a: u32) -> bool {
        let g = self.ast.lhs(Node(a));
        let v = self.ast.rhs(Node(a));
        if self.tag(g) != Tag::Group || !self.func_head_ok(g) || v == NONE {
            return false;
        }
        if self.tag(v) == Tag::Delimited {
            return self.expr_ok(v);
        }
        let (a, b) = (self.first(v), self.last(v));
        (a..b).all(|k| self.t.end(k) == self.t.start(k + 1))
            && (a..=b).all(|k| self.ttag(k) != T::Comma)
    }

    /// the old SPICE grammar's `Key s? '(' s? (Arg (s? ',' s? Arg)*)? s? ')'`: a key and
    /// keys, closed.
    fn func_head_ok(&self, g: u32) -> bool {
        let name = self.ast.lhs(Node(g));
        let key = |k: u32| {
            self.is_word_item(k)
                && self
                    .text(k)
                    .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        };
        name != NONE
            && key(name)
            && self.group_closed(g)
            && self.group_items(g).iter().all(|&k| key(k))
    }

    /// `.param a=1 f(x)=…` (the old SPICE grammar's `Params`).
    fn param_card(
        &mut self,
        n: u32,
        here: &mut Block,
        inside: bool,
        acc: &mut Acc,
    ) -> Result<(), Kept> {
        let items = self.fields(n).to_vec();
        let ngspice = self.d == Dialect::Ngspice;
        let lt_params = self.d == Dialect::Ltspice && self.head_word(n) == "params";
        let mut assigns = Vec::new();
        for (k, &it) in items.iter().enumerate() {
            let plus = ngspice
                && self.tag(it) == Tag::Word
                && self.text(it) == "+"
                && k > 0
                && items
                    .get(k + 1)
                    .is_some_and(|&x| self.tag(x) == Tag::Assign);
            if plus {
                continue;
            }
            if self.tag(it) != Tag::Assign {
                return Err(Kept::Unparsed);
            }
            let key = self.ast.lhs(Node(it));
            let ok = match self.tag(key) {
                Tag::Group => self.funcdef_ok(it),
                _ => {
                    self.is_word_item(key) && self.val_ok(self.ast.rhs(Node(it)))
                        || self.val_p_ok(it)
                }
            };
            if !ok {
                return Err(Kept::Unparsed);
            }
            assigns.push(it);
        }
        if lt_params && assigns.is_empty() {
            return Err(Kept::Unparsed);
        }
        let funcs = assigns
            .iter()
            .any(|&a| self.tag(self.ast.lhs(Node(a))) == Tag::Group);
        if inside && funcs {
            return Err(Kept::Raw);
        }
        for a in assigns {
            if self.tag(self.ast.lhs(Node(a))) == Tag::Group {
                acc.funcs.push(Func::Param(a));
            } else if inside {
                here.locals.push(a);
            } else {
                here.params.push(a);
            }
        }
        Ok(())
    }

    /// quirk: the old ngspice grammar's `Val_p`: a braced group, an operator, an
    /// expression (`{1+x} + x*AGAUSS(0,1,1)`).
    fn val_p_ok(&self, a: u32) -> bool {
        let v = self.ast.rhs(Node(a));
        self.d == Dialect::Ngspice
            && v != NONE
            && self.tag(v) == Tag::Binary
            && self.ttag(self.first(v)) == T::LBrace
    }

    /// `.func name(args) [=] body` (the old SPICE grammar's `Func`).
    fn func(&mut self, n: u32, inside: bool, acc: &mut Acc) -> Result<(), Kept> {
        let Some(g) = self.func_group(n) else {
            return Err(Kept::Unparsed);
        };
        if self.func_body(n, g).is_none() {
            return Err(Kept::Unparsed);
        }
        if inside {
            return Err(Kept::Raw);
        }
        acc.funcs.push(Func::Card(n));
        Ok(())
    }

    /// A `.func` card's `name(args)` group, when its head reads.
    fn func_group(&self, n: u32) -> Option<u32> {
        let &first = self.fields(n).first()?;
        let g = match self.tag(first) {
            Tag::Group => first,
            Tag::Assign if self.tag(self.ast.lhs(Node(first))) == Tag::Group => {
                self.ast.lhs(Node(first))
            }
            _ => return None,
        };
        self.func_head_ok(g).then_some(g)
    }

    /// A `.func` card's body text (`g` its head group), when it reads.
    fn func_body(&self, n: u32, g: u32) -> Option<String> {
        let items = self.fields(n);
        // The body: after `)` and an optional `=`, to the card's end or a
        // ` key=` (the old SPICE grammar's `Body`).
        let close = self.last(g);
        let mut start = close + 1;
        if start <= self.line_last(self.main(n)) && self.ttag(start) == T::Equal {
            start += 1;
        }
        let last = self.line_last(self.main(n));
        if start > last {
            return None;
        }
        for k in start + 1..=last {
            if self.t.end(k - 1) != self.t.start(k)
                && self.ttag(k) == T::Ident
                && k < last
                && self.ttag(k + 1) == T::Equal
            {
                return None;
            }
        }
        // quirk: the old SPICE grammar's `Body`: an `expr` to the end, a
        // braced body to the end, else the text.
        let expr = items
            .iter()
            .copied()
            .chain(
                items
                    .iter()
                    .filter(|&&k| self.tag(k) == Tag::Assign)
                    .map(|&k| self.ast.rhs(Node(k))),
            )
            .find(|&k| k != NONE && self.first(k) == start && self.last(k) == last);
        Some(if expr.is_some_and(|e| self.expr_ok(e)) {
            self.inner(expr.expect("checked")).trim().to_string()
        } else if self.ttag(start) == T::LBrace && self.brace_close_at(start) == Some(last) {
            self.span(start + 1, last - 1).trim().to_string()
        } else {
            self.span(start, last).trim().to_string()
        })
    }

    fn brace_close_at(&self, open: u32) -> Option<u32> {
        let last = self.line_last(open);
        let mut depth = 0i32;
        (open..=last).find(|&k| {
            match self.ttag(k) {
                T::LBrace => depth += 1,
                T::RBrace => depth -= 1,
                _ => {}
            }
            depth == 0
        })
    }
}
