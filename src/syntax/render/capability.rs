//! What each target can write: one exhaustive `match` from a target and a
//! [`Construct`] (a thing whose spelling varies between dialects) to an
//! [`Action`]. A new construct does not compile until every target answers
//! (no `_` arm), as LLVM's legalize table and VerA's total fact tables.
//! The writers ask [`action`] and do what it says; nothing else decides
//! whether a construct is written.
//!
//! `Spell` and `Lift` keep the meaning: a named alternative form, or a
//! rewrite into extra statements recorded as fixups. Anything that cannot
//! keep its meaning is `Refuse` with a `W` code. Seven cells answer as the
//! old writers behaved, where that drops or garbles something: each is
//! marked `// quirk:` (the golden gate holds them until phase 7 decides).
//!
//! CDL's row is the SPICE writer's (its elements are ngspice cards in the
//! xschem layout). Spectre's and VACASK's are the named writer's
//! (`render::named`). Verilog's answer what its old writer did where the
//! construct reaches it (it stays on the old layout writer:
//! `tests/fixtures/transpiler/PHASE4-STATUS.md`).
//!
//! Cites: LLVM `TargetLowering.h` (`LegalizeAction`, one table read by one
//! generic pass); each row's manual clause in its `W` code (`syntax::diag`).
//!
//! Contents:
//! - `pub enum Construct`, `pub enum Action`, `pub enum Alt`, `pub enum Rule`
//! - `pub const fn action(target: Dialect, c: Construct) -> Action`

use crate::syntax::Dialect;
use crate::syntax::diag::Code;

/// A construct whose spelling, or whose existence, differs by target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Construct {
    /// A statement kept as text from another dialect family.
    ForeignText,
    /// A `^` the target reads with the other meaning (power, XOR).
    CaretReread,
    /// A call of `agauss`, `gauss`, `aunif`, `unif` or `limit`.
    StatisticalCall,
    /// A Verilog-A source load.
    VerilogALoad,
    /// A compiled OSDI binary load.
    OsdiLoad,
    /// `.lib file` with no section (LTspice's whole-file form).
    LibWholeFile,
    /// `.options ngbehavior=<mode>`: ngspice's compatibility mode, read
    /// before the deck (ngspice manual, the compatibility modes).
    NgBehavior,
    /// `.lib name` … `.endl`.
    LibSection,
    /// An R, C or L card naming a model.
    RclModel,
    /// `C1 a b q=expr`.
    ChargeCapacitor,
    /// `B1 a b V=expr` / `I=expr`.
    BehaviouralSource,
    /// A Laplace transfer function in a behavioural source.
    Laplace,
    /// `portnum n z0 Z` on a V source.
    RfPort,
    /// PWL `r=`/`td=` on a V source.
    PwlRepeatV,
    /// PWL `r=`/`td=` on an I source.
    PwlRepeatI,
    /// A TRNOISE or TRRANDOM waveform.
    NoiseWave,
    /// `.if` … `.endif`.
    Conditional,
    /// A `.param` or a load inside a conditional branch.
    BranchParam,
    /// `.func f(x) body`.
    Func,
    /// `.step`.
    Step,
    /// `.tf`.
    Tf,
    /// `.sens` (DC).
    Sens,
    /// `.sens … ac`.
    SensAc,
    /// `.pz`.
    PoleZero,
    /// `.sp`.
    SParameter,
    /// `.print`.
    Print,
    /// `.save` outputs.
    Save,
    /// An option with no portable spelling in the target's option form.
    UnmappedOption,
    /// A Spectre `statistics` block.
    Statistics,
    /// `.measure`.
    Measure,
}

/// What a writer does with a construct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Write it in the target's own form.
    Native,
    /// Write a named alternative form that means the same.
    Spell(Alt),
    /// Rewrite it into other statements (fixups), meaning the same.
    Lift(Rule),
    /// Refuse the deck: the target cannot say it.
    Refuse(Code),
}

/// A named alternative spelling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alt {
    /// `Q='expr'` (ngspice §3.3.9, read by HSPICE and ESPice).
    ChargeQuoted,
    /// `Q={expr}` (Xyce).
    ChargeBraced,
    /// `Q=expr` (LTspice).
    ChargeBare,
    /// `E… vol='expr'` / `G… cur='expr'` (HSPICE).
    VolCur,
    /// `B… V={expr}` (Xyce, RG p.238).
    BracedBehaviour,
    /// `.param f(x)='body'` (HSPICE).
    FuncParam,
    /// `.hdl "file"` (ESPice and HSPICE compile Verilog-A themselves).
    Hdl,
    /// `.control` / `pre_osdi file` / `.endc` (ngspice).
    PreOsdi,
    /// `.probe` per output, `.option post` for all (HSPICE).
    Probe,
    /// Not written, the code the note the writer records for it.
    Omit(Code),
    /// Written as is, the code the note the writer records for it.
    Copy(Code),
    /// `c1 (a b) bsource q=expr` (Spectre Reference 19.1 p.459).
    BsourceCharge,
    /// PWL `r=0` as `pwlperiod=` where it repeats exactly, `td=` as
    /// `delay=` (Spectre Reference 5.0 `vsource` #4, #17, #18, p.686).
    PwlPeriod,
    /// The `render::NGBEHAVIOR` line, which the ngspice backend lifts onto
    /// its command line (`-D ngbehavior=<mode>`).
    NgBehaviorLine,
    /// SPICE text fenced by `simulator lang=spice` / `lang=spectre`
    /// (`[KUN]` B.2; Spectre Reference 19.1 p.17: Spectre reads `.measure`).
    SpiceRegion,
}

/// A rewrite rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// Laplace → an XSPICE `s_xfer` instance and its `.model` (ngspice).
    LaplaceXfer,
    /// Laplace → `E/G … LAPLACE c+ c- num… / den…` (HSPICE, ESPice).
    LaplaceTable,
    /// Laplace → `E/G … Laplace=num/den` (LTspice).
    LaplaceAttr,
    /// A `statistics` block → two-layer `.param` draws (SPICE).
    StatisticsDraws,
    /// A statistical call → its `rack_*` function of a normalized variable
    /// declared `parameters v=0` and varied in a `statistics` block
    /// (Spectre; Spectre Reference 19.1 pp.175-179, p.481).
    RackVars,
    /// The statistical calls VACASK lacks spelled out: `limit` → an
    /// `aunif` ternary, Xyce's clamp → `min(max())`, a two-argument
    /// `agauss` its sigma 1 (VACASK `docs/cmd-analysis-mc.md`).
    VacaskCalls,
}

/// What `target` does with `c`.
pub const fn action(target: Dialect, c: Construct) -> Action {
    use Action::{Lift, Native, Refuse, Spell};
    use Dialect as D;
    match c {
        Construct::ForeignText => match target {
            // quirk: the xschem layout (CDL) copies another family's text
            // as written, as the old writer did; phase 7 refuses it.
            D::Cdl => Spell(Alt::Copy(Code::W0027)),
            D::Ngspice | D::Espice | D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0001),
            D::Spectre | D::Vacask | D::Verilog => Refuse(Code::W0001),
        },
        Construct::CaretReread => Refuse(Code::W0002),
        Construct::NgBehavior => match target {
            D::Ngspice => Spell(Alt::NgBehaviorLine),
            D::Espice | D::Hspice | D::Xyce | D::Ltspice => Spell(Alt::Omit(Code::W0030)),
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Spell(Alt::Omit(Code::W0030)),
        },
        Construct::StatisticalCall => match target {
            D::Ltspice => Refuse(Code::W0003),
            D::Ngspice | D::Espice | D::Hspice | D::Xyce => Native,
            D::Spectre => Lift(Rule::RackVars),
            D::Vacask => Lift(Rule::VacaskCalls),
            D::Cdl | D::Verilog => Native,
        },
        Construct::VerilogALoad => match target {
            D::Ngspice => Spell(Alt::PreOsdi),
            D::Espice | D::Hspice => Spell(Alt::Hdl),
            D::Xyce => Refuse(Code::W0004),
            // quirk: the old LTspice writer drops a load (LTspice has no
            // OSDI loader); phase 7 refuses it.
            D::Ltspice => Spell(Alt::Omit(Code::W0022)),
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::OsdiLoad => match target {
            D::Ngspice => Spell(Alt::PreOsdi),
            D::Espice | D::Hspice | D::Xyce => Refuse(Code::W0004),
            // quirk: as `VerilogALoad`.
            D::Ltspice => Spell(Alt::Omit(Code::W0022)),
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::LibWholeFile => match target {
            D::Ltspice => Native,
            D::Ngspice | D::Espice | D::Hspice | D::Xyce => Refuse(Code::W0005),
            // quirk: the old Spectre and VACASK writers wrote `include
            // "file" section=` (no section); phase 7 refuses it.
            D::Spectre | D::Vacask => Spell(Alt::Copy(Code::W0110)),
            D::Cdl | D::Verilog => Refuse(Code::W0005),
        },
        Construct::LibSection => match target {
            D::Ltspice => Refuse(Code::W0012),
            D::Ngspice | D::Espice | D::Hspice | D::Xyce => Native,
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::RclModel => match target {
            D::Ltspice | D::Spectre | D::Vacask => Refuse(Code::W0006),
            D::Ngspice | D::Espice | D::Hspice | D::Xyce => Native,
            D::Cdl | D::Verilog => Native,
        },
        Construct::ChargeCapacitor => match target {
            D::Ngspice | D::Espice | D::Hspice => Spell(Alt::ChargeQuoted),
            D::Xyce => Spell(Alt::ChargeBraced),
            D::Ltspice => Spell(Alt::ChargeBare),
            D::Spectre => Spell(Alt::BsourceCharge),
            // VACASK has no charge-based element.
            D::Vacask => Refuse(Code::W0105),
            D::Cdl | D::Verilog => Native,
        },
        Construct::BehaviouralSource => match target {
            D::Hspice => Spell(Alt::VolCur),
            D::Xyce => Spell(Alt::BracedBehaviour),
            D::Ngspice | D::Espice | D::Ltspice => Native,
            D::Spectre | D::Vacask => Refuse(Code::W0102),
            D::Cdl | D::Verilog => Native,
        },
        Construct::Laplace => match target {
            // CDL writes an element as its ngspice card.
            D::Ngspice | D::Cdl => Lift(Rule::LaplaceXfer),
            D::Espice | D::Hspice => Lift(Rule::LaplaceTable),
            D::Ltspice => Lift(Rule::LaplaceAttr),
            D::Xyce => Refuse(Code::W0011),
            D::Spectre | D::Vacask | D::Verilog => Refuse(Code::W0011),
        },
        Construct::RfPort => match target {
            D::Ngspice | D::Espice => Native,
            D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0007),
            // Spectre's `port` instance; the mapping from `portnum`/`z0`
            // is unverified.
            D::Spectre | D::Vacask => Refuse(Code::W0007),
            D::Cdl | D::Verilog => Native,
        },
        Construct::PwlRepeatV => match target {
            D::Ngspice | D::Espice | D::Cdl => Native,
            D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0008),
            D::Spectre => Spell(Alt::PwlPeriod),
            D::Vacask | D::Verilog => Refuse(Code::W0008),
        },
        Construct::PwlRepeatI => match target {
            D::Espice => Native,
            D::Ngspice | D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0008),
            D::Spectre => Spell(Alt::PwlPeriod),
            D::Vacask | D::Cdl | D::Verilog => Refuse(Code::W0008),
        },
        Construct::NoiseWave => match target {
            D::Ngspice | D::Cdl => Native,
            D::Espice | D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0009),
            D::Spectre | D::Vacask | D::Verilog => Refuse(Code::W0009),
        },
        Construct::Conditional => match target {
            D::Xyce | D::Ltspice => Refuse(Code::W0013),
            D::Ngspice | D::Espice | D::Hspice => Native,
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::BranchParam => match target {
            D::Ngspice | D::Espice | D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0014),
            // A VACASK `@if` holds instances and models only.
            D::Vacask => Refuse(Code::W0014),
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::Func => match target {
            D::Hspice => Spell(Alt::FuncParam),
            D::Ngspice | D::Espice | D::Xyce | D::Ltspice => Native,
            D::Vacask => Refuse(Code::W0106),
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::Step => match target {
            D::Xyce | D::Ltspice => Native,
            D::Ngspice | D::Espice | D::Hspice => Refuse(Code::W0016),
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::Tf => match target {
            D::Xyce => Refuse(Code::W0015),
            D::Ngspice | D::Espice | D::Hspice | D::Ltspice => Native,
            D::Spectre | D::Vacask | D::Cdl | D::Verilog => Native,
        },
        Construct::Sens => match target {
            D::Ngspice | D::Espice | D::Hspice => Native,
            D::Xyce | D::Ltspice | D::Vacask => Refuse(Code::W0015),
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::SensAc | Construct::SParameter => match target {
            D::Ngspice | D::Espice => Native,
            D::Hspice | D::Xyce | D::Ltspice | D::Vacask => Refuse(Code::W0015),
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::PoleZero => match target {
            D::Ngspice | D::Espice => Native,
            D::Hspice | D::Xyce | D::Ltspice => Refuse(Code::W0015),
            D::Spectre | D::Vacask => Refuse(Code::W0015),
            D::Cdl | D::Verilog => Native,
        },
        Construct::Print => match target {
            // An LTspice run saves every vector and has no stdout print.
            D::Ltspice => Spell(Alt::Omit(Code::W0023)),
            D::Ngspice | D::Espice | D::Hspice | D::Xyce => Native,
            // quirk: the old Spectre and VACASK writers drop a `.print`
            // (never mapped); phase 7 decides.
            D::Spectre | D::Vacask => Spell(Alt::Omit(Code::W0023)),
            D::Cdl | D::Verilog => Native,
        },
        Construct::Save => match target {
            D::Hspice => Spell(Alt::Probe),
            // VACASK saves every node and source flow unless `strictsave`.
            D::Xyce | D::Vacask => Spell(Alt::Omit(Code::W0023)),
            D::Ngspice | D::Espice | D::Ltspice => Native,
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::UnmappedOption => match target {
            D::Xyce | D::Vacask => Refuse(Code::W0017),
            D::Ngspice | D::Espice | D::Hspice | D::Ltspice => Native,
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::Statistics => match target {
            D::Ngspice | D::Espice | D::Hspice | D::Xyce | D::Ltspice => {
                Lift(Rule::StatisticsDraws)
            }
            // One layer: a VACASK `parameters` call is one value.
            D::Vacask => Lift(Rule::StatisticsDraws),
            D::Spectre | D::Cdl | D::Verilog => Native,
        },
        Construct::Measure => match target {
            D::Ngspice | D::Espice | D::Hspice | D::Xyce | D::Ltspice => Native,
            D::Spectre => Spell(Alt::SpiceRegion),
            D::Vacask => Refuse(Code::W0109),
            D::Cdl | D::Verilog => Native,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_w_refusal_codes_and_omissions_w_notes() {
        // Every refusal is a render-stage refusal code, every drop or copy a
        // render-stage note; the table is total over the targets the writers
        // serve.
        use crate::syntax::diag::{Kind, info};
        for d in Dialect::ALL {
            for c in CONSTRUCTS {
                let (code, kind) = match action(d, c) {
                    Action::Refuse(code) => (code, Kind::Refusal),
                    Action::Spell(Alt::Omit(code) | Alt::Copy(code)) => (code, Kind::Note),
                    Action::Native | Action::Spell(_) | Action::Lift(_) => continue,
                };
                assert!(format!("{code:?}").starts_with('W'), "{d:?} {c:?}");
                assert_eq!(info(code).kind, kind, "{d:?} {c:?}");
            }
        }
    }

    #[test]
    fn the_spice_family_differs_only_where_its_manuals_do() {
        use Dialect as D;
        assert_eq!(action(D::Ngspice, Construct::Conditional), Action::Native);
        assert_eq!(
            action(D::Xyce, Construct::Conditional),
            Action::Refuse(Code::W0013)
        );
        assert_eq!(
            action(D::Xyce, Construct::ChargeCapacitor),
            Action::Spell(Alt::ChargeBraced)
        );
        assert_eq!(action(D::Ltspice, Construct::LibWholeFile), Action::Native);
    }

    const CONSTRUCTS: [Construct; 30] = [
        Construct::ForeignText,
        Construct::NgBehavior,
        Construct::CaretReread,
        Construct::StatisticalCall,
        Construct::VerilogALoad,
        Construct::OsdiLoad,
        Construct::LibWholeFile,
        Construct::LibSection,
        Construct::RclModel,
        Construct::ChargeCapacitor,
        Construct::BehaviouralSource,
        Construct::Laplace,
        Construct::RfPort,
        Construct::PwlRepeatV,
        Construct::PwlRepeatI,
        Construct::NoiseWave,
        Construct::Conditional,
        Construct::BranchParam,
        Construct::Func,
        Construct::Step,
        Construct::Tf,
        Construct::Sens,
        Construct::SensAc,
        Construct::PoleZero,
        Construct::SParameter,
        Construct::Print,
        Construct::Save,
        Construct::UnmappedOption,
        Construct::Statistics,
        Construct::Measure,
    ];
}
