"""
Python tests for the circuit builders: Subcircuit, Testbench, ModelLibrary.
Each call is checked by the ngspice card it writes (ngspice manual ch.3-4,
the element cards; ch.11, the analyses), never by internal state.

Run with: maturin develop && pytest tests/test_python_subcircuit.py
"""
import pytest


def import_spicerack():
    import spicerack  # unsupported fails, never skips: an unbuilt extension is an error
    return spicerack


def cards(sc):
    """The lines of an ngspice deck that defines `sc` (its `.subckt` block)."""
    c = import_spicerack().Circuit("t")
    c.subcircuit(sc)
    return str(c).splitlines()


# ── Subcircuit Tests ──


class TestSubcircuitCreation:
    def test_create_subcircuit(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("inverter", ["vdd", "vss", "vin", "vout"])
        assert "inverter" in str(sc)

    def test_create_subcircuit_no_ports(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("top")
        assert "top" in str(sc)

    def test_create_subcircuit_with_params(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("amp", ["in", "out"], W="1u", L="180n")
        assert ".subckt amp in out PARAMS: W=1u L=180n" in cards(sc)

    def test_repr(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("buf", ["a", "b"])
        r = repr(sc)
        assert "Subcircuit" in r
        assert "buf" in r

    def test_gnd(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("test")
        assert sc.gnd == "0"


class TestSubcircuitComponents:
    def test_resistor(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("r_test", ["in", "out"])
        sc.R(
            name="1",
            positive="in",
            negative="out",
            value=1000.0,
        )
        assert "R1 in out 1k" in cards(sc)

    def test_capacitor(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("c_test")
        sc.C(
            name="1",
            positive="out",
            negative=sc.gnd,
            value=10e-12,
        )
        assert "C1 out 0 10p" in cards(sc)

    def test_inductor(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("l_test")
        sc.L(
            name="1",
            positive="in",
            negative="out",
            value=1e-6,
        )
        assert "L1 in out 1u" in cards(sc)

    def test_mutual_inductor(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("k_test")
        sc.L(
            name="1",
            positive="a",
            negative="b",
            value=1e-3,
        )
        sc.L(
            name="2",
            positive="c",
            negative="d",
            value=1e-3,
        )
        sc.K(
            name="1",
            inductor1="1",
            inductor2="2",
            coupling=0.99,
        )
        assert "K1 L1 L2 0.99" in cards(sc)

    def test_voltage_source(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("v_test")
        sc.V(
            name="dd",
            positive="vdd",
            negative=sc.gnd,
            value=3.3,
        )
        assert "Vdd vdd 0 3.3" in cards(sc)

    def test_current_source(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("i_test")
        sc.I(
            name="bias",
            positive="vdd",
            negative="out",
            value=100e-6,
        )
        assert "Ibias vdd out 100u" in cards(sc)

    def test_behavioral_voltage(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("bv_test")
        sc.BV(
            name="1",
            positive="out",
            negative=sc.gnd,
            expression="V(in)*2",
        )
        assert "B1 out 0 V=V(in)*2" in cards(sc)

    def test_behavioral_current(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("bi_test")
        sc.BI(
            name="1",
            positive="out",
            negative=sc.gnd,
            expression="V(in)/1k",
        )
        assert "B1 out 0 I=V(in)/1k" in cards(sc)

    def test_vcvs(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("e_test")
        sc.E(
            name="1",
            positive="out",
            negative=sc.gnd,
            control_positive="inp",
            control_negative="inm",
            voltage_gain=10.0,
        )
        assert "E1 out 0 inp inm 10" in cards(sc)

    def test_vccs(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("g_test")
        sc.G(
            name="1",
            positive="out",
            negative=sc.gnd,
            control_positive="inp",
            control_negative="inm",
            transconductance=1e-3,
        )
        assert "G1 out 0 inp inm 0.001" in cards(sc)

    def test_cccs(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("f_test")
        sc.F(
            name="1",
            positive="out",
            negative=sc.gnd,
            vsense="Vsense",
            current_gain=100.0,
        )
        assert "F1 out 0 Vsense 100" in cards(sc)

    def test_ccvs(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("h_test")
        sc.H(
            name="1",
            positive="out",
            negative=sc.gnd,
            vsense="Vsense",
            transresistance=1e3,
        )
        assert "H1 out 0 Vsense 1000" in cards(sc)

    def test_diode(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("d_test")
        sc.D(
            name="1",
            anode="anode",
            cathode="cathode",
            model="D1N4148",
        )
        assert "D1 anode cathode D1N4148" in cards(sc)

    def test_bjt(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("q_test")
        sc.Q(
            name="1",
            collector="collector",
            base="base",
            emitter="emitter",
            model="2N2222",
        )
        assert "Q1 collector base emitter 2N2222" in cards(sc)

    def test_bjt_alias(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("bjt_test")
        sc.BJT(
            name="1",
            collector="c",
            base="b",
            emitter="e",
            model="npn_model",
        )
        assert "Q1 c b e npn_model" in cards(sc)

    def test_mosfet(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("m_test")
        sc.M(
            name="p1",
            drain="vout",
            gate="vin",
            source="vdd",
            bulk="vdd",
            model="pmos",
        )
        assert "Mp1 vout vin vdd vdd pmos" in cards(sc)

    def test_mosfet_alias(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("mos_test")
        sc.MOSFET(
            name="1",
            drain="d",
            gate="g",
            source="s",
            bulk="b",
            model="nmos",
        )
        assert "M1 d g s b nmos" in cards(sc)

    def test_jfet(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("j_test")
        sc.J(
            name="1",
            drain="drain",
            gate="gate",
            source="source",
            model="jfet_model",
        )
        assert "J1 drain gate source jfet_model" in cards(sc)

    def test_mesfet(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("z_test")
        sc.Z(
            name="1",
            drain="drain",
            gate="gate",
            source="source",
            model="mesfet_model",
        )
        assert "Z1 drain gate source mesfet_model" in cards(sc)

    def test_vswitch(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("s_test")
        sc.S(
            name="1",
            positive="out",
            negative=sc.gnd,
            control_positive="ctrl_p",
            control_negative="ctrl_m",
            model="sw_model",
        )
        assert "S1 out 0 ctrl_p ctrl_m sw_model" in cards(sc)

    def test_iswitch(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("w_test")
        sc.W(
            name="1",
            positive="out",
            negative=sc.gnd,
            vcontrol="Vctrl",
            model="isw_model",
        )
        assert "W1 out 0 Vctrl isw_model" in cards(sc)

    def test_tline(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("t_test")
        sc.T(
            name="1",
            input_positive="inp",
            input_negative="inm",
            output_positive="outp",
            output_negative="outm",
            Z0=50.0,
            TD=1e-9,
        )
        assert "T1 inp inm outp outm Z0=50 TD=0.000000001" in cards(sc)

    def test_xspice(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("a_test")
        sc.A(
            name="adc1",
            connections=["[vin]", "[dout]"],
            model="adc_buf",
        )
        assert "Aadc1 [vin] [dout] adc_buf" in cards(sc)

    def test_raw_resistor(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("raw_r_test")
        sc.R(
            name="1",
            positive="in",
            negative="out",
            value=0.0,
            raw_spice="1k tc=0.01",
        )
        # The text after the nodes is the card's own: value and parameters.
        assert "R1 in out 1k tc=0.01" in cards(sc)

class TestSubcircuitDirectives:
    def test_model(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("model_test")
        sc.model("nmos_3p3", "NMOS", VTO="0.7", KP="110e-6")
        assert ".model nmos_3p3 NMOS (VTO=0.7 KP=110e-6)" in cards(sc)

    def test_raw_spice(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("raw_test")
        sc.raw_spice(".options reltol=1e-6")
        assert ".options reltol=1e-6" in "\n".join(cards(sc))

    def test_include(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("inc_test")
        sc.include("/path/to/model.lib")
        assert ".include /path/to/model.lib" in cards(sc)

    def test_lib(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("lib_test")
        sc.lib("/path/to/pdk.lib", "tt")
        assert ".lib /path/to/pdk.lib tt" in cards(sc)

    def test_osdi(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("osdi_test")
        sc.osdi("/path/to/model.osdi")
        # ngspice loads OSDI from its `.control` block (manual §16.4, pre_osdi).
        assert "pre_osdi /path/to/model.osdi" in cards(sc)

    def test_parameter(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("param_test")
        sc.parameter("vdd", "3.3")
        assert ".subckt param_test PARAMS: vdd=3.3" in cards(sc)

class TestSubcircuitInstances:
    def test_instance(self):
        ps = import_spicerack()
        inv = ps.Subcircuit("inverter", ["vdd", "vss", "vin", "vout"])
        inv.M(
            name="p1",
            drain="vout",
            gate="vin",
            source="vdd",
            bulk="vdd",
            model="pmos",
        )
        inv.M(
            name="n1",
            drain="vout",
            gate="vin",
            source="vss",
            bulk="vss",
            model="nmos",
        )

        top = ps.Subcircuit("top")
        top.instance(inv, "inv1", "vdd", "vss", "a", "b")
        assert "Xinv1 vdd vss a b inverter" in cards(top)

    def test_x_instance(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("top")
        sc.X("1", "MyBuf", "in", "out", "vdd", "gnd")
        assert "X1 in out vdd gnd MyBuf" in cards(sc)

    def test_x_params_reach_the_card_on_every_backend(self):
        ps = import_spicerack()
        from spicerack.unit import u_kOhm
        top = ps.Subcircuit("top", ["a"])
        top.X("1", "rload", "a", "0", r=3 @ u_kOhm, nf=2, w="{wn*2}")
        tb = ps.Testbench(top)
        cards = {
            "ngspice": "X1 a 0 rload r=3e3 nf=2 w={wn*2}",
            "ltspice": "X1 a 0 rload r=3e3 nf=2 w={wn*2}",
            # Numbers in Spectre's suffixes, and the expression bare: Spectre
            # has no `{}` delimiter (Spectre Reference 19.1 p.506, `parameters
            # p1=1 p2=p1*2`: an expression is written as it is).
            "spectre": "x1 (a 0) rload r=3k nf=2 w=wn*2",
        }
        for backend, card in cards.items():
            assert card in tb.netlist(backend).splitlines(), backend

    def test_unit_value_params_are_si_not_display_text(self):
        # str(1 @ u_MOhm) is "1MOhm", which SPICE reads as 1 milliohm.
        ps = import_spicerack()
        from spicerack.unit import u_MOhm, u_pF
        sc = ps.Subcircuit("t", ["a"])
        sc.M(name="1", drain="a", gate="a", source="0", bulk="0",
             model="n", W=1 @ u_MOhm)
        sc.model("cm", "C", C=1.5 @ u_pF)
        deck = ps.Testbench(sc).netlist("ngspice")
        assert "W=1e6" in deck
        assert "C=1.5e-12" in deck

    def test_instance_params_match_x(self):
        ps = import_spicerack()
        child = ps.Subcircuit("rload", ["a", "b"], r="1k")
        top = ps.Subcircuit("top", ["a"])
        top.instance(child, "1", "a", "0", r=2000.0)
        assert "X1 a 0 rload r=2000" in ps.Testbench(top).netlist("ngspice")

    @pytest.mark.needs("ngspice")
    def test_circuit_x_params_override_the_subckt_default(self):
        ps = import_spicerack()
        from spicerack.unit import u_kOhm
        load = ps.Subcircuit("rload", ["a", "b"], r="1k")
        load.raw_spice("R1 a b {r}")
        c = ps.Circuit("x_params")
        c.subcircuit(load)
        c.V(name="s", positive="vin", negative=c.gnd, value=1.0)
        c.R(name="t", positive="vin", negative="vout", value=1 @ u_kOhm)
        c.X("L1", "rload", "vout", c.gnd, r=3 @ u_kOhm)
        assert "XL1 vout 0 rload r=3e3" in str(c)
        vout = c.simulator(simulator="ngspice").operating_point()["vout"]
        assert abs(vout - 0.75) < 1e-9


class TestSubcircuitStore:
    """The store is the deck's source text (D3), never a JSON form."""

    def _amp(self, ps):
        sc = ps.Subcircuit("amp", ["in", "out"], w="1u")
        sc.R(name="1", positive="in", negative="out", value=1000.0)
        sc.C(name="1", positive="out", negative=sc.gnd, value=10e-12)
        sc.model("d_model", "D", IS="1e-14")
        sc.include("/m.lib")
        return sc

    def test_source_round_trip(self):
        ps = import_spicerack()
        text = self._amp(ps).to_source()
        lines = text.splitlines()
        assert lines[:2] == ["* spicerack-source ngspice", "amp"]
        assert ".subckt amp in out PARAMS: w=1u" in lines
        assert "R1 in out 1k" in lines
        again = ps.Subcircuit.from_source(text)
        assert again.to_source() == text
        assert (again.name, again.ports) == ("amp", ["in", "out"])

    def test_save_and_load(self, tmp_path):
        ps = import_spicerack()
        sc = self._amp(ps)
        path = str(tmp_path / "amp.sp")
        sc.save(path)
        assert open(path).read() == sc.to_source()
        assert ps.Subcircuit.load(path).to_source() == sc.to_source()

    def test_from_source_reads_another_dialect(self):
        # Spectre UG p.94: `subckt name (ports)` … `ends`; written as ngspice.
        ps = import_spicerack()
        spectre = "// t\nsubckt cell (a b)\nr1 (a b) resistor r=1k\nends cell\n"
        for sc in (ps.Subcircuit.from_source(spectre, "spectre"),
                   ps.Subcircuit.from_source("* spicerack-source spectre\n" + spectre)):
            assert (sc.name, sc.ports) == ("cell", ["a", "b"])
            assert "R1 a b 1k" in sc.to_source().splitlines()

    def test_from_source_refuses_text_with_no_subcircuit(self):
        ps = import_spicerack()
        with pytest.raises(ValueError, match="no subcircuit"):
            ps.Subcircuit.from_source("t\nR1 a 0 1k\n.end\n")
        with pytest.raises(ValueError, match="unknown dialect"):
            ps.Subcircuit.from_source("t\n", "pspice")


class TestRemovedJson:
    """`to_json`, `from_json`, `save_json`, `load_json` went with the IR;
    each raises `NotImplementedError` naming the source-text method."""

    def _both(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("amp", ["in", "out"])
        return ps, sc, ps.Testbench(sc)

    def test_to_json(self):
        _, sc, tb = self._both()
        for obj in (sc, tb):
            with pytest.raises(NotImplementedError, match="to_source"):
                obj.to_json()

    def test_from_json(self):
        ps, _, _ = self._both()
        for cls in (ps.Subcircuit, ps.Testbench):
            with pytest.raises(NotImplementedError, match="from_source"):
                cls.from_json("{}")

    def test_save_json(self):
        _, sc, tb = self._both()
        for obj, instead in ((sc, r"save\(path\)"), (tb, r"save_source\(path\)")):
            with pytest.raises(NotImplementedError, match=instead):
                obj.save_json("x.json")

    def test_load_json(self):
        ps, _, _ = self._both()
        for cls in (ps.Subcircuit, ps.Testbench):
            with pytest.raises(NotImplementedError, match="load"):
                cls.load_json("x.json")


# Every target a netlist is written for (`Deck::netlist`).
TARGETS = ("ngspice", "espice", "ltspice", "hspice", "spectre", "vacask")


def _netlists(tb):
    """Each target's netlist, or the refusal's text."""
    out = {}
    for t in TARGETS:
        try:
            out[t] = tb.netlist(t)
        except RuntimeError as e:
            out[t] = f"refused: {e}"
    return out


class TestTestbenchStore:
    """A testbench's store is its deck's source text: read back, it writes
    the same netlist on every target (D3: the source is the lossless form)."""

    def _benches(self, ps):
        amp = ps.Subcircuit("amp", ["in", "out"], g="2")
        amp.R(name="1", positive="in", negative="out", value=1e3)
        amp.C(name="1", positive="out", negative=amp.gnd, value=1e-12)
        top = ps.Subcircuit("top", ["a", "b"])
        top.X("u1", "amp", "a", "b", g="3")
        plain = ps.Testbench(top)
        plain.add_subcircuit(amp)
        plain.V("in", "a", "0", 1.0, ac=1.0)
        plain.options(reltol=1e-4)
        plain.temperature = 50.0
        plain.save("v(b)")
        plain.measure("tran", "vmax", "max", "v(b)")
        plain.add_operating_point()
        plain.add_ac("dec", 10, 1.0, 1e9)
        plain.add_transient(step_time=1e-9, end_time=1e-6)
        yield plain
        # Every dialect's region, ngspice's mode, a load and a step.
        mixed = ps.Testbench(top)
        mixed.add_subcircuit(amp)
        mixed.V("in", "a", "0", 1.0)
        mixed.ngbehavior = "hsa"
        dev = ps.Subcircuit("dev", ["p"])
        dev.osdi("/m.osdi")
        mixed.add_subcircuit(dev)
        mixed.step_sweep("g", 1.0, 3.0, 1.0, "lin")
        mixed.add_spectre_monte_carlo(20, "tran1", "tran", seed=42)
        mixed.extra_line(".four 1k v(b)")
        mixed.extra_line("parameters k=1", dialect="spectre")
        mixed.add_transient(step_time=1e-9, end_time=1e-6)
        yield mixed

    def test_a_read_back_deck_writes_the_same_netlist_on_every_target(self):
        ps = import_spicerack()
        for tb in self._benches(ps):
            text = tb.to_source()
            assert text.startswith("* spicerack-source ngspice\n")
            again = ps.Testbench.from_source(text)
            assert again.to_source() == text
            assert _netlists(again) == _netlists(tb)

    def test_save_and_load(self, tmp_path):
        ps = import_spicerack()
        for tb in self._benches(ps):
            path = str(tmp_path / "tb.sp")
            tb.save_source(path)
            again = ps.Testbench.load(path)
            assert _netlists(again) == _netlists(tb)

    def test_calls_on_a_read_back_deck_add_after_its_statements(self):
        ps = import_spicerack()
        tb = next(self._benches(ps))
        again = ps.Testbench.from_source(tb.to_source())
        again.add_noise("b", "0", "Vin", "dec", 10, 1.0, 1e6)
        lines = again.netlist("ngspice").splitlines()
        assert any(line.lower().startswith((".noise", "noise ")) for line in lines), lines


class TestDialectText:
    """A Spectre-only statement and an `extra_line` are their dialect's
    text in the deck's source: written for that dialect's family, refused
    (`W0001`) for any other."""

    def _tb(self, ps):
        dut = ps.Subcircuit("rc", ["in", "out"])
        dut.R(name="1", positive="in", negative="out", value=1e3)
        dut.C(name="1", positive="out", negative=dut.gnd, value=1e-9)
        tb = ps.Testbench(dut)
        tb.V("in", "in", "0", 1.0)
        tb.add_transient(step_time=1e-6, end_time=1e-3)
        return tb

    def test_a_spectre_analysis_is_spectre_text_in_a_region(self):
        # Spectre Reference 19.1 p.174: `montecarlo numruns= seed= { … }`.
        ps = import_spicerack()
        tb = self._tb(ps)
        tb.add_spectre_monte_carlo(20, "tran1", "tran", seed=42)
        lines = tb.to_source().splitlines()
        i = lines.index("simulator lang=spectre")
        assert lines[i + 1] == "mc1 montecarlo numruns=20 seed=42 {"
        assert "simulator lang=spice" in lines[i:]
        assert "mc1 montecarlo numruns=20 seed=42 {" in tb.netlist("spectre").splitlines()
        for other in ("ngspice", "hspice", "vacask"):
            with pytest.raises(RuntimeError, match="W0001"):
                tb.netlist(other)

    def test_an_extra_line_is_copied_on_its_own_family_only(self):
        ps = import_spicerack()
        tb = self._tb(ps)
        tb.extra_line(".options  NOACCT")
        for own in ("ngspice", "espice"):
            assert ".options  NOACCT" in tb.netlist(own).splitlines(), own
        for other in ("hspice", "ltspice", "spectre", "vacask"):
            with pytest.raises(RuntimeError, match="W0001"):
                tb.netlist(other)

    def test_an_extra_line_names_its_dialect(self):
        ps = import_spicerack()
        tb = self._tb(ps)
        tb.extra_line("parameters k=2", dialect="spectre")
        assert "parameters k=2" in tb.netlist("spectre").splitlines()
        with pytest.raises(RuntimeError, match="W0001"):
            tb.netlist("ngspice")
        with pytest.raises(ValueError, match="unknown dialect"):
            tb.extra_line("x", dialect="pspice")
        with pytest.raises(ValueError, match="no cdl text"):
            tb.extra_line("x", dialect="cdl")


class TestSubcircuitWaveforms:
    def test_every_source_on_both_builders(self):
        """Subcircuit and Testbench get their sixteen sources from one table."""
        ps = import_spicerack()
        waves = ("Sinusoidal", "Pulse", "PieceWiseLinear", "Exponential", "SingleFrequencyFM", "AmplitudeModulated", "TransientNoise", "Random")
        names = [f"{w}{k}Source" for w in waves for k in ("Voltage", "Current")]
        for cls in (ps.Subcircuit, ps.Testbench):
            assert [n for n in names if hasattr(cls, n)] == names, cls.__name__

    def test_sinusoidal_voltage(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("sin_test")
        sc.SinusoidalVoltageSource(
            name="in",
            positive="inp",
            negative=sc.gnd,
            amplitude=1.0,
            frequency=1e6,
        )
        assert "Vin inp 0 0 SIN(0 1 1000000 0 0 0)" in cards(sc)

    def test_pulse_voltage(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("pulse_test")
        sc.PulseVoltageSource(
            name="clk",
            positive="clk",
            negative=sc.gnd,
            pulsed_value=3.3,
            period=10e-9,
        )
        assert any(l.startswith("Vclk clk 0 0 PULSE(0 3.3 ") for l in cards(sc))

    def test_pwl_voltage(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("pwl_test")
        sc.PieceWiseLinearVoltageSource(
            name="ramp",
            positive="ramp",
            negative=sc.gnd,
            values=[(0, 0), (1e-3, 1.0)],
        )
        assert "Vramp ramp 0 0 PWL(0 0 0.001 1)" in cards(sc)

    def test_sinusoidal_current(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("sin_i_test")
        sc.SinusoidalCurrentSource(
            name="in",
            positive="inp",
            negative=sc.gnd,
            amplitude=1e-3,
            frequency=1e3,
        )
        assert "Iin inp 0 0 SIN(0 0.001 1000 0 0 0)" in cards(sc)

    def test_pulse_current(self):
        ps = import_spicerack()
        sc = ps.Subcircuit("pulse_i_test")
        sc.PulseCurrentSource(
            name="pulse",
            positive="out",
            negative=sc.gnd,
            pulsed_value=1e-3,
        )
        assert any(l.startswith("Ipulse out 0 0 PULSE(0 0.001 ") for l in cards(sc))

class TestTestbenchCreation:
    def test_create_testbench(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("my_amp", ["inp", "out"])
        dut.R(
            name="1",
            positive="inp",
            negative="out",
            value=10e3,
        )
        tb = ps.Testbench(dut)
        assert "my_amp" in str(tb)

    def test_stimulus(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("rc", ["in", "out"])
        dut.R(
            name="1",
            positive="in",
            negative="out",
            value=1000.0,
        )
        dut.C(
            name="1",
            positive="out",
            negative=dut.gnd,
            value=1e-9,
        )

        tb = ps.Testbench(dut)
        tb.V(
            name="in",
            positive="in",
            negative=dut.gnd,
            value=1.0,
        )
        deck = tb.netlist("ngspice").splitlines()
        assert "Vin in 0 1" in deck

    def test_add_subcircuit(self):
        ps = import_spicerack()
        inv = ps.Subcircuit("inverter", ["vdd", "vss", "vin", "vout"])
        inv.M(
            name="p1",
            drain="vout",
            gate="vin",
            source="vdd",
            bulk="vdd",
            model="pmos",
        )

        dut = ps.Subcircuit("top")
        dut.instance(inv, "inv1", "vdd", "vss", "a", "b")

        tb = ps.Testbench(dut)
        tb.add_subcircuit(inv)
        deck = tb.netlist("ngspice").splitlines()
        assert ".subckt inverter vdd vss vin vout" in deck

    def test_options(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.options(reltol="1e-6", abstol="1e-12")
        assert ".options reltol=1e-6 abstol=1e-12" in tb.netlist("ngspice").splitlines()

    def test_temperature(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.temperature = 85.0
        assert ".temp 85" in tb.netlist("ngspice").splitlines()

    def test_save(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.save("v(out)", "i(Vin)")
        assert ".save v(out) i(Vin)" in tb.netlist("ngspice").splitlines()

    def test_measure(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.measure("TRAN", "vmax", "MAX", "V(out)")
        assert ".meas TRAN vmax MAX V(out)" in tb.netlist("ngspice").splitlines()

    def test_step(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.step("R1", 100.0, 10000.0, 100.0)
        # ngspice has no `.step` (it would run once); LTspice does (`.step param`).
        assert ".step param R1 100 10000 100" in tb.netlist("ltspice").splitlines()

    def test_waveform_sources(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("test")
        dut.R(
            name="1",
            positive="a",
            negative="b",
            value=100.0,
        )
        tb = ps.Testbench(dut)
        tb.SinusoidalVoltageSource(
            name="in",
            positive="a",
            negative="0",
            amplitude=1.0,
            frequency=1e6,
        )
        assert "Vin a 0 0 SIN(0 1 1000000 0 0 0)" in tb.netlist("ngspice").splitlines()

    def test_current_waveform_sources(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("current_tb", ["in", "out"])
        tb = ps.Testbench(dut)
        tb.PulseCurrentSource(
            name="q",
            positive="in",
            negative="0",
            pulsed_value=1e-6,
            pulse_width=1e-9,
        )
        assert any(l.startswith("Iq in 0 0 PULSE(0 0.000001 ") for l in tb.netlist("ngspice").splitlines())

    def test_child_subcircuit_directives_reach_the_deck(self):
        # A child's include/lib/osdi used to be dropped from Testbench decks.
        ps = import_spicerack()
        child = ps.Subcircuit("ch", ["a"])
        child.include("/c.inc")
        child.lib("/c.lib", "tt")
        child.osdi("/c.osdi")
        top = ps.Subcircuit("top", ["a"])
        top.X("1", "ch", "a")
        tb = ps.Testbench(top)
        tb.add_subcircuit(child)
        tb.add_subcircuit(child)
        deck = tb.netlist("ngspice").splitlines()
        assert deck.count(".include /c.inc") == 1
        assert deck.count(".lib /c.lib tt") == 1
        assert deck.count("pre_osdi /c.osdi") == 1
        assert 'include "/c.lib" section=tt' in tb.netlist("spectre")

    def test_unknown_backend_is_refused_at_selection(self):
        ps = import_spicerack()
        tb = ps.Testbench(ps.Subcircuit("d", ["a"]))
        with pytest.raises(ValueError, match="unknown backend"):
            tb.with_backend("xyce")
        with pytest.raises(ValueError, match="unknown backend"):
            ps.Circuit("c").simulator(simulator="egspice")
        for name in ["ngspice", "ngspice-subprocess", "ngspice-shared", "ltspice",
                     "vacask", "vacask-shared", "spectre", "espice"]:
            tb.with_backend(name)

    def test_add_multi_analysis_and_netlist(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("multi", ["in", "out"])
        dut.R(name="1", positive="in", negative="out", value=1000.0)
        tb = ps.Testbench(dut)
        tb.V(name="in", positive="in", negative="0", value=1.0, ac=1.0)
        tb.add_operating_point()
        tb.add_ac(variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e6)
        tb.measure("tran", "vmax", "MAX", "V(out)")

        netlist = tb.netlist("ngspice")
        # With a `.meas`, ngspice runs every analysis from one `.control`
        # block (it refuses `.measure` under `-b -r`, and an analysis left as
        # a card would run past the write); other dialects keep the cards.
        assert ".control\nop\nac dec 10 1 1meg\n.endc" in netlist
        assert ".ac dec 10 1 1meg" in tb.netlist("ltspice")
        assert ".meas tran vmax MAX V(out)" in netlist

    def test_sweep_name_is_checked_when_the_analysis_is_built(self):
        # Spec: dec/oct/lin/log in any case; anything else is a ValueError at
        # the call, not a deck a simulator later rejects.
        ps = import_spicerack()
        tb = ps.Testbench(ps.Subcircuit("d", ["a"]))
        tb.add_ac(variation="DEC", number_of_points=10, start_frequency=1.0, stop_frequency=1e6)
        assert ".ac dec 10 1 1meg" in tb.netlist("ngspice")
        with pytest.raises(ValueError, match="unknown sweep 'decade'"):
            tb.add_ac(variation="decade", number_of_points=10, start_frequency=1.0, stop_frequency=1e6)
        with pytest.raises(ValueError, match="unknown sweep"):
            tb.add_noise("a", "0", "V1", variation="per-decade")
        with pytest.raises(ValueError, match="unknown sweep"):
            tb.ac(variation="decade", number_of_points=10, start_frequency=1.0, stop_frequency=1e6)
        # The refused calls queued nothing: one `.ac`, no `.noise`.
        deck = tb.netlist("ngspice")
        assert deck.count(".ac ") == 1 and ".noise" not in deck

    def test_testbench_statistical_analysis_builders(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("amp_mc", ["vin", "vout"])

        tb = ps.Testbench(dut)
        tb.add_spectre_monte_carlo(20, "tran1", "tran", seed=42)
        spectre = tb.netlist("spectre")
        assert "mc1 montecarlo numruns=20" in spectre
        assert "seed=42" in spectre


class TestTestbenchCheckBackend:
    def test_check_backend_clean(self):
        ps = import_spicerack()
        dut = ps.Subcircuit("simple")
        dut.R(
            name="1",
            positive="in",
            negative="out",
            value=1000.0,
        )
        dut.V(
            name="1",
            positive="in",
            negative=dut.gnd,
            value=1.0,
        )
        tb = ps.Testbench(dut)
        issues = tb.check_backend("ngspice")
        assert len(issues) == 0

# ── ModelLibrary Tests ──


class TestModelLibrary:
    def test_create(self):
        ps = import_spicerack()
        lib = ps.ModelLibrary("/path/to/sky130.lib", corner="tt")
        assert lib.name == "sky130"
        assert lib.path == "/path/to/sky130.lib"
        assert lib.corner == "tt"

    def test_create_no_corner(self):
        ps = import_spicerack()
        lib = ps.ModelLibrary("/models/custom.lib")
        assert lib.name == "custom"
        assert lib.corner is None

    def test_repr(self):
        ps = import_spicerack()
        lib = ps.ModelLibrary("/path/to/sky130.lib", corner="tt")
        r = repr(lib)
        assert "ModelLibrary" in r
        assert "sky130" in r


# ── Backwards Compatibility ──


class TestBackwardsCompat:
    """Ensure existing Circuit API still works unchanged."""

    def test_circuit_basic(self):
        ps = import_spicerack()
        c = ps.Circuit("test")
        c.V(
            name="1",
            positive="in",
            negative=c.gnd,
            value=5.0,
        )
        c.R(
            name="1",
            positive="in",
            negative="out",
            value=1000.0,
        )
        c.R(
            name="2",
            positive="out",
            negative=c.gnd,
            value=1000.0,
        )
        netlist = str(c)
        assert "V1" in netlist
        assert "R1" in netlist
        assert "R2" in netlist

    def test_circuit_model(self):
        ps = import_spicerack()
        c = ps.Circuit("mos")
        c.M(
            name="1",
            drain="d",
            gate="g",
            source="s",
            bulk="b",
            model="nmos",
        )
        c.model("nmos", "NMOS", VTO="0.7")
        netlist = str(c)
        assert "M1" in netlist
        assert ".model" in netlist

    def test_circuit_simulator(self):
        ps = import_spicerack()
        c = ps.Circuit("sim_test")
        c.V(
            name="1",
            positive="in",
            negative=c.gnd,
            value=1.0,
        )
        c.R(
            name="1",
            positive="in",
            negative=c.gnd,
            value=1000.0,
        )
        sim = c.simulator()
        assert repr(sim) == "CircuitSimulator"

    def test_circuit_getitem(self):
        ps = import_spicerack()
        c = ps.Circuit("lookup")
        c.R(
            name="1",
            positive="a",
            negative="b",
            value=1000.0,
        )
        elem = c["1"]
        assert "R1" in elem

    def test_circuit_subcircuit_instance(self):
        ps = import_spicerack()
        c = ps.Circuit("x_test")
        c.X("1", "MyBuf", "in", "out", "vdd")
        netlist = str(c)
        assert "X1" in netlist
        assert "MyBuf" in netlist


class TestSimulatorCheckBackend:
    """Test check_backend method on existing CircuitSimulator."""

    def test_check_backend_on_simulator(self):
        ps = import_spicerack()
        c = ps.Circuit("check_test")
        c.R(
            name="1",
            positive="in",
            negative="out",
            value=1000.0,
        )
        c.V(
            name="1",
            positive="in",
            negative=c.gnd,
            value=1.0,
        )
        sim = c.simulator()
        issues = sim.check_backend("ngspice")
        assert isinstance(issues, list)


def test_name_and_ports_read_back_in_declaration_order():
    import spicerack as ps
    sc = ps.Subcircuit("opamp", ["inp", "inn", "out", "vdd", "vss"])
    assert sc.name == "opamp"
    assert sc.ports == ["inp", "inn", "out", "vdd", "vss"]
