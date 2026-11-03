//! What a header statement's `code` names when it is no element: the
//! [`Analysis`] kinds of every dialect, Spectre's control statements
//! ([`Command`]), and the [`Statement`] a Spectre master word makes of
//! `name [(nodes)] word params`. The lexicon tables that spell them are the
//! dialects' (`Lexicon::masters`, `Lexicon::analyses`); SPICE spells its
//! analyses as dot-card keywords (`.tran`), so its kind comes from the
//! keyword token.
//!
//! Cites: ngspice manual ch.11 analyses (`.op`, `.dc`, `.ac`, `.tran`,
//! `.noise`, `.tf`, `.sens`, `.pz`, `.four`, `.sp`); Spectre Reference 19.1
//! `[REF]` analysis statements (p.43 `ac`, p.416 `tran`, p.181 `noise`,
//! p.438 `xf`, p.524 `sens`, p.272 `pss`, p.90 `hb`, p.387 `sp`, p.393 `stb`,
//! p.406 `sweep`, p.245 `pac`, p.253 `pnoise`, p.303 `pxf`, p.174
//! `montecarlo`) and control statements (`alter`, `altergroup`, `assert`,
//! `check`, `checklimit`, `info`, `options`, `paramset`, `paramtest`, `set`,
//! `shell`); VACASK's `analysis <name> op|dcinc|dcxf|ac|acxf|noise|tran|hb`
//! (its `share/doc/vacask/{demo,test}` decks).
//!
//! Contents:
//! - `pub enum Analysis`, `Analysis::ALL: [Analysis; 40]`, `Analysis::from_u32(code: u32) -> Option<Analysis>`
//! - `pub enum Command`, `Command::ALL: [Command; 10]`, `Command::from_u32(code: u32) -> Option<Command>`
//! - `pub enum Statement`

/// An analysis kind, whatever its dialect spells it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Analysis {
    Op,
    Dc,
    Ac,
    Tran,
    Noise,
    Tf,
    Sens,
    Pz,
    /// SPICE `.four`.
    Four,
    Sp,
    Montecarlo,
    Acmatch,
    Dcmatch,
    Hbnoise,
    Qpnoise,
    /// Spectre `fourier` (`[CMP]` p.323 calls it a component; today's
    /// reader took it for an analysis, and so does this table).
    Fourier,
    Pdisto,
    Pnoise,
    Hbstb,
    Envlp,
    Hbsp,
    Hbac,
    Hbxf,
    Qpac,
    Qpsp,
    Qpss,
    Qpxf,
    /// Spectre `sweep` (its body holds the swept analyses).
    Sweep,
    Pac,
    Psp,
    Pss,
    Pstb,
    Pxf,
    Tdr,
    Stb,
    Hb,
    Xf,
    /// VACASK `dcinc`, the small-signal DC analysis.
    Dcinc,
    /// VACASK `dcxf`, the DC transfer function.
    Dcxf,
    /// VACASK `acxf`, the AC transfer function.
    Acxf,
}

/// A Spectre control statement (`options` is [`Statement::Options`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Command {
    Altergroup,
    Checklimit,
    Paramtest,
    Paramset,
    Assert,
    Alter,
    Check,
    Shell,
    Info,
    Set,
}

/// What a master word makes of a Spectre `name [(nodes)] word params`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Statement {
    /// An analysis (`tran1 tran stop=1n`); nodes allowed (`noise1 (o 0)
    /// noise`).
    Analysis(Analysis),
    /// A control statement, only with no nodes.
    Command(Command),
    /// `opt1 options reltol=1e-3`, only with no nodes.
    Options,
}

impl Analysis {
    /// Every kind, in discriminant order.
    pub const ALL: [Analysis; 40] = {
        use Analysis::*;
        [
            Op, Dc, Ac, Tran, Noise, Tf, Sens, Pz, Four, Sp, Montecarlo, Acmatch, Dcmatch, Hbnoise,
            Qpnoise, Fourier, Pdisto, Pnoise, Hbstb, Envlp, Hbsp, Hbac, Hbxf, Qpac, Qpsp, Qpss,
            Qpxf, Sweep, Pac, Psp, Pss, Pstb, Pxf, Tdr, Stb, Hb, Xf, Dcinc, Dcxf, Acxf,
        ]
    };

    /// The kind stored as `code` (`a as u32`); `None` for any other word.
    pub fn from_u32(code: u32) -> Option<Analysis> {
        Analysis::ALL.get(code as usize).copied()
    }
}

impl Command {
    /// Every control statement, in discriminant order.
    pub const ALL: [Command; 10] = {
        use Command::*;
        [
            Altergroup, Checklimit, Paramtest, Paramset, Assert, Alter, Check, Shell, Info, Set,
        ]
    };

    /// The statement stored as `code`; `None` for any other word.
    pub fn from_u32(code: u32) -> Option<Command> {
        Command::ALL.get(code as usize).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for (k, a) in Analysis::ALL.into_iter().enumerate() {
            assert_eq!(a as usize, k);
            assert_eq!(Analysis::from_u32(a as u32), Some(a));
        }
        for (k, c) in Command::ALL.into_iter().enumerate() {
            assert_eq!(c as usize, k);
            assert_eq!(Command::from_u32(c as u32), Some(c));
        }
        assert_eq!(Analysis::from_u32(u32::MAX), None);
        assert_eq!(Command::from_u32(u32::MAX), None);
    }
}
