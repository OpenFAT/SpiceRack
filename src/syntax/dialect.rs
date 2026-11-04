//! The netlist languages SpiceRack reads and writes, by name: what a
//! lexicon reads (`lexicon::of`), a renderer writes (`render`) and a
//! backend runs.
//!
//! Cites: none (names only; each dialect's lexicon and writer cite its
//! manual).
//!
//! Contents:
//! - `pub enum Dialect`, `Dialect::ALL: [Dialect; 9]`,
//!   `Dialect::ALIASES: [(&str, Dialect); 3]`, `Dialect::name(self) -> &'static str`,
//!   `Dialect::family(self) -> Dialect`, `Dialect::from_backend_name(&str) -> Option<Dialect>`,
//!   `impl FromStr for Dialect`

/// The netlist language a simulator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Ngspice,
    Ltspice,
    Spectre,
    Vacask,
    /// ngspice text with Verilog-A loaded as `.hdl` instead of `pre_osdi`.
    Espice,
    /// HSPICE: `'expr'` quoting, `E`/`G` `vol=`/`cur=` behavioural sources,
    /// `.hdl` Verilog-A, and the card forms in ESPice's
    /// `tests/frontend/grammar/hspice.peg` and `tests/conformance/tests/netlist/hspice`.
    Hspice,
    /// Xyce.
    Xyce,
    /// CDL for LVS: elements in the xschem layout, no analyses.
    Cdl,
    /// Structural Verilog: one module per subcircuit, its instances.
    Verilog,
}

impl Dialect {
    pub const ALL: [Dialect; 9] = [
        Dialect::Ngspice,
        Dialect::Ltspice,
        Dialect::Spectre,
        Dialect::Vacask,
        Dialect::Espice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Cdl,
        Dialect::Verilog,
    ];

    /// The dialect's name (`ngspice`, ...), as `FromStr` reads it.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Ngspice => "ngspice",
            Dialect::Ltspice => "ltspice",
            Dialect::Spectre => "spectre",
            Dialect::Vacask => "vacask",
            Dialect::Espice => "espice",
            Dialect::Hspice => "hspice",
            Dialect::Xyce => "xyce",
            Dialect::Cdl => "cdl",
            Dialect::Verilog => "verilog",
        }
    }

    /// The dialect whose text this one reads: ESPice reads ngspice decks.
    pub fn family(self) -> Dialect {
        if self == Dialect::Espice {
            Dialect::Ngspice
        } else {
            self
        }
    }

    /// Backend names kept as aliases of a dialect's own name, so scripts
    /// that name the old shared-library backends still run.
    pub const ALIASES: [(&'static str, Dialect); 3] = [
        ("ngspice-subprocess", Dialect::Ngspice),
        ("ngspice-shared", Dialect::Ngspice),
        ("vacask-shared", Dialect::Vacask),
    ];

    /// The dialect a backend name selects: its [`Self::name`] or one of
    /// [`Self::ALIASES`], spelled exactly (unlike `FromStr`, case matters).
    pub fn from_backend_name(s: &str) -> Option<Dialect> {
        let alias = Self::ALIASES.iter().find(|a| a.0 == s).map(|a| a.1);
        alias.or_else(|| Self::ALL.into_iter().find(|d| d.name() == s))
    }
}

impl std::str::FromStr for Dialect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Dialect::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown dialect '{s}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_back_and_espice_reads_ngspice() {
        for d in Dialect::ALL {
            assert_eq!(d.name().parse::<Dialect>(), Ok(d));
            assert_eq!(Dialect::from_backend_name(d.name()), Some(d));
        }
        assert_eq!("NGSPICE".parse::<Dialect>(), Ok(Dialect::Ngspice));
        assert_eq!(Dialect::from_backend_name("NGSPICE"), None, "exact");
        assert_eq!(Dialect::Espice.family(), Dialect::Ngspice);
        assert_eq!(Dialect::Hspice.family(), Dialect::Hspice);
        assert!("pspice".parse::<Dialect>().is_err());
    }
}
