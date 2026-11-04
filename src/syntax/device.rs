//! The device table per dialect: a SPICE element letter, or a Spectre or
//! VACASK master word, ↔ the [`Primitive`] it names, with the card shape the
//! parser reads it by (how many leading words are nodes, where the model or
//! master sits, where the value sits). The parser reads a table word →
//! primitive ([`find`]); a renderer writes primitive → word ([`spelling`]);
//! a test checks that the two directions invert.
//!
//! The SPICE dialects share [`SPICE`], the twenty letters every one of them
//! reads; ngspice adds [`NGSPICE_OWN`]. A letter no table holds is an
//! element with no primitive. Spectre's table holds the masters an instance
//! may name directly; VACASK's holds the device modules a `model` statement
//! names (an instance's master is always a model or a subcircuit there).
//!
//! Cites (each row's general form): ngspice manual ch.3 (`RXXXXXXX n+ n-
//! <value> <mname>`, `CXXXXXXX`, `LYYYYYYY`, `KXXXXXXX LYYYYYYY LZZZZZZZ
//! value`, `SXXXXXXX N+ N- NC+ NC- MODEL`, `WYYYYYYY N+ N- VNAM MODEL`), ch.4
//! (`VXXXXXXX N+ N- <<DC> DC/TRAN VALUE> <AC ...>`), ch.5 (`EXXXXXXX N+ N- NC+
//! NC- VALUE`, `FXXXXXXX N+ N- VNAM VALUE`, `BXXXXXXX n+ n- <i=expr> <v=expr>`),
//! ch.6 (`TXXXXXXX N1 N2 N3 N4 Z0=VALUE`, `OXXXXXXX n1 n2 n3 n4 mname`,
//! `UXXXXXXX n1 n2 n3 mname`, `YXXXXXXX N1 0 N2 0 mname`, `PXXXXXXX NI1 ...
//! GND1 NO1 ... GND2 mname`), ch.7-ch.11 (`DXXXXXXX n+ n- mname`, `QXXXXXXX nc
//! nb ne <ns> <tj> mname`, `JXXXXXXX nd ng ns mname`, `ZXXXXXXX ND NG NS
//! MNAME`, `MXXXXXXX nd ng ns nb mname`, BSIMSOI `MXXXXXXX nd ng ns ne [np]
//! [nb] [nT] mname`), ch.12 (XSPICE `AXXXXXXX ... mname`), ch.13 (OSDI
//! `NXXXXXXX n1 ... mname`), §2.4.3 (`XYYYYYYY N1 <N2 N3 ...> SUBNAM`);
//! HSPICE `Mxxx nd ng ns <nb> mname` (its optional bulk); Spectre Circuit
//! Components Reference `[CMP]` p.628 (`resistor`), p.283 (`capacitor`),
//! p.372 (`inductor`), p.577 (`mutual_inductor`), p.683, p.380 (`vsource`,
//! `isource`), p.680, p.678, p.286, p.288 (`vcvs`, `vccs`, `cccs`, `ccvs`),
//! p.856 (`bsource`), p.302 (`diode`), the `bjt` and `mos` forms (`Name c b
//! e [s]`, `Name d g s b`); VACASK `share/doc/vacask/demo` (`model resistor
//! sp_resistor`, `k1 () mutual`; the built-ins the old VACASK writer declared).
//!
//! Contents:
//! - `pub enum Primitive`, `pub struct Device`, `pub enum Terminals`, `pub enum Model`, `pub enum Value`
//! - `pub const SPICE`, `NGSPICE_OWN`, `SPECTRE`, `VACASK: &[Device]`, `pub const FLAGS: &[&str]`
//! - `pub fn find(tables: &[&'static [Device]], word: &str, fold: bool) -> Option<&'static Device>`
//! - `pub fn spelling(tables: &[&'static [Device]], p: Primitive) -> Option<&'static Device>`
//! - `Primitive::ALL: [Primitive; 26]`, `Primitive::from_u32(code: u32) -> Option<Primitive>`

use Model as M;
use Primitive as P;
use Terminals as N;
use Value as V;

/// What an element is, whatever its dialect spells it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Primitive {
    Resistor,
    Capacitor,
    Inductor,
    /// Mutual inductance between two inductors.
    Coupling,
    VoltageSource,
    CurrentSource,
    Vcvs,
    Vccs,
    Cccs,
    Ccvs,
    /// The behavioural source (`B`, `bsource`).
    Behavioural,
    Diode,
    Bjt,
    Mosfet,
    Jfet,
    Mesfet,
    VoltageSwitch,
    CurrentSwitch,
    /// The lossless transmission line (`T`).
    TransmissionLine,
    /// A subcircuit instance (`X`): the master is the subcircuit's name.
    Subckt,
    /// ngspice `O`, the lossy transmission line (LTRA).
    LossyLine,
    /// ngspice `U`, the uniform distributed RC line.
    RcLine,
    /// ngspice `Y`, the single lossy line (TXL).
    Txl,
    /// ngspice `P`, coupled multiconductor lines (CPL).
    CoupledLines,
    /// ngspice `A`, an XSPICE code model.
    CodeModel,
    /// ngspice `N`, an OSDI (compiled Verilog-A) device.
    Osdi,
}

/// One table row: a spelling and the shape of the cards that use it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Device {
    /// A SPICE letter (lowercase), or a Spectre or VACASK master word.
    pub word: &'static str,
    pub primitive: Primitive,
    /// How many of the leading words are nodes.
    pub terminals: Terminals,
    /// Device names right after the nodes: `F`/`H`/`W`'s controlling
    /// source, `K`'s two inductors.
    pub refs: u8,
    /// Where the model or master sits.
    pub model: Model,
    /// Where the value sits.
    pub value: Value,
}

/// A card's terminal count.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terminals {
    Fixed(u8),
    /// At least `.0`, at most `.1`; which one, the model decides (a BJT's
    /// substrate node), so the parser reads the longest the card's shape
    /// allows and resolve checks it.
    Range(u8, u8),
    /// Any number (`X`).
    Any,
    /// The model alone knows (an OSDI module's ports, an XSPICE model's).
    ByModel,
}

/// Where a card's model or master sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// It has none.
    None,
    /// The word after the nodes and refs.
    AfterNodes,
    /// Maybe a word among the arguments (`R1 a b 1k rmod`, `R1 a b rmod`):
    /// only resolve, knowing the deck's models, can tell it from a value.
    InArgs,
    /// `X`'s: the last bare word before the parameters (`params:` or the
    /// first `k=v`), or one last word after them (sky130's late name).
    Subckt,
}

/// Where a card's value sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Value {
    /// It has none (a model, or parameters only).
    None,
    /// The first positional argument after the nodes and refs.
    First,
    /// A source's `dc`, `ac` and waveform specs.
    Specs,
    /// One of these keys (`B`'s `v=`/`i=`, Spectre's `r=`).
    Keys(&'static [&'static str]),
}

/// The words that end a SPICE node list before the model: the instance
/// flags of the semiconductor cards (ngspice ch.7-ch.11 `off`, `on`; the
/// HICUM/VBIC `thermal` node flag).
pub const FLAGS: &[&str] = &["thermal", "off", "on"];

/// The element letters every SPICE dialect reads (ngspice, HSPICE, Xyce and
/// LTspice share these twenty), in the module header's general forms.
pub const SPICE: &[Device] = &[
    row("r", P::Resistor, N::Fixed(2), 0, M::InArgs, V::First),
    row("c", P::Capacitor, N::Fixed(2), 0, M::InArgs, V::First),
    row("l", P::Inductor, N::Fixed(2), 0, M::InArgs, V::First),
    row("k", P::Coupling, N::Fixed(0), 2, M::None, V::First),
    row("v", P::VoltageSource, N::Fixed(2), 0, M::None, V::Specs),
    row("i", P::CurrentSource, N::Fixed(2), 0, M::None, V::Specs),
    row("e", P::Vcvs, N::Fixed(4), 0, M::None, V::First),
    row("g", P::Vccs, N::Fixed(4), 0, M::None, V::First),
    row("f", P::Cccs, N::Fixed(2), 1, M::None, V::First),
    row("h", P::Ccvs, N::Fixed(2), 1, M::None, V::First),
    row(
        "b",
        P::Behavioural,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["v", "i"]),
    ),
    row("d", P::Diode, N::Fixed(2), 0, M::AfterNodes, V::None),
    row("q", P::Bjt, N::Range(3, 5), 0, M::AfterNodes, V::None),
    // ngspice's bulk MOSFET has 4 nodes, HSPICE's bulk is optional, BSIMSOI
    // has up to 7.
    row("m", P::Mosfet, N::Range(3, 7), 0, M::AfterNodes, V::None),
    row("j", P::Jfet, N::Fixed(3), 0, M::AfterNodes, V::None),
    row("z", P::Mesfet, N::Fixed(3), 0, M::AfterNodes, V::None),
    row(
        "s",
        P::VoltageSwitch,
        N::Fixed(4),
        0,
        M::AfterNodes,
        V::None,
    ),
    row(
        "w",
        P::CurrentSwitch,
        N::Fixed(2),
        1,
        M::AfterNodes,
        V::None,
    ),
    row("t", P::TransmissionLine, N::Fixed(4), 0, M::None, V::None),
    row("x", P::Subckt, N::Any, 0, M::Subckt, V::None),
];

/// ngspice's own letters, past [`SPICE`].
pub const NGSPICE_OWN: &[Device] = &[
    row("o", P::LossyLine, N::Fixed(4), 0, M::AfterNodes, V::None),
    row("u", P::RcLine, N::Fixed(3), 0, M::AfterNodes, V::None),
    row("y", P::Txl, N::Fixed(4), 0, M::AfterNodes, V::None),
    row("p", P::CoupledLines, N::ByModel, 0, M::AfterNodes, V::None),
    row("a", P::CodeModel, N::ByModel, 0, M::AfterNodes, V::None),
    row("n", P::Osdi, N::ByModel, 0, M::AfterNodes, V::None),
];

/// The Spectre masters an instance names directly (`r1 (a b) resistor
/// r=1k`). The polynomial sources read as their linear primitive, so the
/// first spelling of each primitive is the one a writer uses.
pub const SPECTRE: &[Device] = &[
    row(
        "resistor",
        P::Resistor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["r"]),
    ),
    row(
        "capacitor",
        P::Capacitor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["c"]),
    ),
    row(
        "inductor",
        P::Inductor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["l"]),
    ),
    row(
        "mutual_inductor",
        P::Coupling,
        N::Fixed(0),
        0,
        M::None,
        V::Keys(&["coupling"]),
    ),
    row(
        "vsource",
        P::VoltageSource,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["dc"]),
    ),
    row(
        "isource",
        P::CurrentSource,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["dc"]),
    ),
    row("vcvs", P::Vcvs, N::Fixed(4), 0, M::None, V::Keys(&["gain"])),
    row("vccs", P::Vccs, N::Fixed(4), 0, M::None, V::Keys(&["gm"])),
    row("cccs", P::Cccs, N::Fixed(2), 0, M::None, V::Keys(&["gain"])),
    row("ccvs", P::Ccvs, N::Fixed(2), 0, M::None, V::Keys(&["rm"])),
    row("pvcvs", P::Vcvs, N::Any, 0, M::None, V::Keys(&["coeffs"])),
    row("pvccs", P::Vccs, N::Any, 0, M::None, V::Keys(&["coeffs"])),
    row(
        "pcccs",
        P::Cccs,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["coeffs"]),
    ),
    row(
        "pccvs",
        P::Ccvs,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["coeffs"]),
    ),
    row(
        "bsource",
        P::Behavioural,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["v", "i", "q"]),
    ),
    row("diode", P::Diode, N::Fixed(2), 0, M::None, V::None),
    row("bjt", P::Bjt, N::Range(3, 4), 0, M::None, V::None),
    row("mos1", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("mos2", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("mos3", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("bsim3v3", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("bsim4", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
];

/// The VACASK device modules a `model NAME <module>` names: the built-ins,
/// then the SPICE set VACASK builds as OSDI (`spice/*.osdi`).
pub const VACASK: &[Device] = &[
    row(
        "resistor",
        P::Resistor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["r"]),
    ),
    row(
        "capacitor",
        P::Capacitor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["c"]),
    ),
    row(
        "inductor",
        P::Inductor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["l"]),
    ),
    row(
        "mutual",
        P::Coupling,
        N::Fixed(0),
        0,
        M::None,
        V::Keys(&["k"]),
    ),
    row(
        "vsource",
        P::VoltageSource,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["dc"]),
    ),
    row(
        "isource",
        P::CurrentSource,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["dc"]),
    ),
    row("vcvs", P::Vcvs, N::Fixed(4), 0, M::None, V::Keys(&["gain"])),
    row("vccs", P::Vccs, N::Fixed(4), 0, M::None, V::Keys(&["gain"])),
    row("cccs", P::Cccs, N::Fixed(2), 0, M::None, V::Keys(&["gain"])),
    row("ccvs", P::Ccvs, N::Fixed(2), 0, M::None, V::Keys(&["gain"])),
    row(
        "sp_resistor",
        P::Resistor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["r"]),
    ),
    row(
        "sp_capacitor",
        P::Capacitor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["c"]),
    ),
    row(
        "sp_inductor",
        P::Inductor,
        N::Fixed(2),
        0,
        M::None,
        V::Keys(&["l"]),
    ),
    row("sp_diode", P::Diode, N::Fixed(2), 0, M::None, V::None),
    row("sp_bjt", P::Bjt, N::Range(3, 4), 0, M::None, V::None),
    row("sp_mos1", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_mos2", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_mos3", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_mos6", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_mos9", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_bsim3v3", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_bsim4v8", P::Mosfet, N::Fixed(4), 0, M::None, V::None),
    row("sp_vdmos", P::Mosfet, N::Fixed(3), 0, M::None, V::None),
    row("sp_jfet1", P::Jfet, N::Fixed(3), 0, M::None, V::None),
    row("sp_mes1", P::Mesfet, N::Fixed(3), 0, M::None, V::None),
];

/// The row spelled `word` in `tables` (the first table holding it wins),
/// case-blind where `fold`: the reading direction.
pub fn find(tables: &[&'static [Device]], word: &str, fold: bool) -> Option<&'static Device> {
    tables.iter().find_map(|t| {
        t.iter().find(|d| {
            if fold {
                d.word.eq_ignore_ascii_case(word)
            } else {
                d.word == word
            }
        })
    })
}

/// The first row of `tables` naming `p`: the spelling a writer uses.
pub fn spelling(tables: &[&'static [Device]], p: Primitive) -> Option<&'static Device> {
    tables
        .iter()
        .find_map(|t| t.iter().find(|d| d.primitive == p))
}

impl Primitive {
    /// Every primitive, in discriminant order.
    pub const ALL: [Primitive; 26] = [
        P::Resistor,
        P::Capacitor,
        P::Inductor,
        P::Coupling,
        P::VoltageSource,
        P::CurrentSource,
        P::Vcvs,
        P::Vccs,
        P::Cccs,
        P::Ccvs,
        P::Behavioural,
        P::Diode,
        P::Bjt,
        P::Mosfet,
        P::Jfet,
        P::Mesfet,
        P::VoltageSwitch,
        P::CurrentSwitch,
        P::TransmissionLine,
        P::Subckt,
        P::LossyLine,
        P::RcLine,
        P::Txl,
        P::CoupledLines,
        P::CodeModel,
        P::Osdi,
    ];

    /// The primitive stored as `code` (`p as u32`); `None` for any other
    /// word ([`crate::syntax::ast::NONE`]).
    pub fn from_u32(code: u32) -> Option<Primitive> {
        Primitive::ALL.get(code as usize).copied()
    }
}

// ---- private ----

const fn row(
    word: &'static str,
    primitive: Primitive,
    terminals: Terminals,
    refs: u8,
    model: Model,
    value: Value,
) -> Device {
    Device {
        word,
        primitive,
        terminals,
        refs,
        model,
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: [&[&[Device]]; 4] = [&[SPICE], &[SPICE, NGSPICE_OWN], &[SPECTRE], &[VACASK]];

    #[test]
    fn reading_and_writing_invert() {
        // Every spelling reads back as its primitive, and the spelling a
        // writer picks for a primitive reads back as that primitive.
        for tables in TABLES {
            for d in tables.iter().flat_map(|t| t.iter()) {
                let read = find(tables, d.word, true).expect("every word reads");
                assert_eq!(read, d, "{} is spelled by one row", d.word);
                let written = spelling(tables, d.primitive).expect("a read primitive is written");
                assert_eq!(
                    find(tables, written.word, true).map(|r| r.primitive),
                    Some(d.primitive)
                );
            }
        }
    }

    #[test]
    fn the_spice_letters_are_single_lowercase_bytes() {
        for d in SPICE.iter().chain(NGSPICE_OWN) {
            assert_eq!(d.word.len(), 1);
            assert!(d.word.as_bytes()[0].is_ascii_lowercase());
        }
        // SPICE spends the whole alphabet on element letters.
        let mut letters: Vec<u8> = SPICE
            .iter()
            .chain(NGSPICE_OWN)
            .map(|d| d.word.as_bytes()[0])
            .collect();
        letters.sort_unstable();
        assert_eq!(letters, (b'a'..=b'z').collect::<Vec<u8>>());
    }

    #[test]
    fn codes_round_trip() {
        for (k, p) in Primitive::ALL.into_iter().enumerate() {
            assert_eq!(p as usize, k);
            assert_eq!(Primitive::from_u32(p as u32), Some(p));
        }
        assert_eq!(Primitive::from_u32(u32::MAX), None);
    }
}
